use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use batlehub_adapters::db::PgQuotaRepository;
use batlehub_adapters::registry::{
    CargoRegistryClient, ComposerRegistryClient, CondaRegistryClient, FanoutRegistryClient,
    ForgejoRegistryClient, GithubRegistryClient, GitlabRegistryClient, GoProxyRegistryClient,
    MavenRegistryClient, NpmRegistryClient, NugetRegistryClient, OpenVsxRegistryClient,
    PathProxyRegistryClient, PypiRegistryClient, RubyGemsRegistryClient, TerraformRegistryClient,
    UpstreamHttpOptions, VsCodeMarketplaceRegistryClient,
};
use batlehub_config::schema::{
    QuotaEnforcement as ConfigQuotaEnforcement, RegistryConfig, RuleConfig, UpstreamAuthConfig,
};
use batlehub_core::{
    entities::{RegistryKind, Role, Severity},
    ports::VulnerabilityRepository,
    rules::{
        BlockListRule, CveGateRule, DenyLatestRule, RbacRule, ReleaseAgeGateRule,
        RequireSignedReleaseRule, TrustedPublisherRule, VersionGateRule,
    },
    services::{QuotaEnforcement, QuotaService, RegistryPolicy, RegistryQuotaConfig},
};
use batlehub_web::CargoIndexProxy;

pub(super) fn parse_role(s: &str) -> Role {
    match s {
        "admin" => Role::Admin,
        "user" => Role::User,
        _ => Role::Anonymous,
    }
}

pub(super) fn upstream_options(
    reg: &RegistryConfig,
    global_proxy: Option<&batlehub_config::schema::UpstreamProxyConfig>,
) -> UpstreamHttpOptions {
    let (bearer_token, basic_auth, custom_header) = match &reg.upstream_auth {
        Some(UpstreamAuthConfig::Bearer(b)) => (Some(b.token.clone()), None, None),
        Some(UpstreamAuthConfig::Basic(b)) => {
            (None, Some((b.username.clone(), b.password.clone())), None)
        }
        Some(UpstreamAuthConfig::Header(h)) => {
            (None, None, Some((h.name.clone(), h.value.clone())))
        }
        None => (None, None, None),
    };
    let proxy = reg.proxy.as_ref().or(global_proxy);
    UpstreamHttpOptions {
        bearer_token,
        basic_auth,
        custom_header,
        ca_cert_path: reg.tls.as_ref().and_then(|t| t.ca_cert_path.clone()),
        search_url: reg.search_url.clone(),
        proxy_url: proxy.map(|p| p.url.clone()),
        proxy_username: proxy.and_then(|p| p.username.clone()),
        proxy_password: proxy.and_then(|p| p.password.clone()),
        no_proxy: proxy.and_then(|p| p.no_proxy.clone()),
    }
}

pub(super) fn build_cargo_index(
    reg: &RegistryConfig,
    global_proxy: Option<&batlehub_config::schema::UpstreamProxyConfig>,
) -> anyhow::Result<CargoIndexProxy> {
    let index_url = if let Some(ref url) = reg.index_url {
        url.clone()
    } else {
        let upstream = reg
            .upstreams
            .first()
            .map(|s| s.as_str())
            .unwrap_or("https://crates.io");
        if upstream.contains("crates.io") {
            "https://index.crates.io".to_owned()
        } else {
            upstream.to_owned()
        }
    };
    let opts = upstream_options(reg, global_proxy);
    let http = batlehub_adapters::registry::apply_upstream_options(
        reqwest::Client::builder().user_agent("batlehub/0.1"),
        &opts,
    )?;
    tracing::info!(index_url = %index_url, "cargo sparse index proxy configured");
    Ok(CargoIndexProxy { http, index_url })
}

/// Build the per-registry repository-metadata signing keys for `deb`/`rpm`/`pacman`
/// registries that configured `[registries.repo_signing]`. Registries without a
/// key host unsigned repositories.
pub(super) fn build_repo_signer_map(
    cfg: &batlehub_config::schema::AppConfig,
) -> anyhow::Result<batlehub_web::RepoSignerMap> {
    use batlehub_adapters::repo::OpenPgpSigner;
    let mut map = HashMap::new();
    for reg in &cfg.registries {
        if let Some(sign) = &reg.repo_signing {
            let signer = OpenPgpSigner::from_seed_hex(
                &sign.seed_hex,
                sign.created.unwrap_or(0),
                sign.user_id.as_deref().unwrap_or("BatleHub"),
            )
            .map_err(|e| anyhow::anyhow!("building repo signing key for '{}': {e}", reg.name))?;
            map.insert(reg.name.clone(), Arc::new(signer));
        }
    }
    Ok(batlehub_web::RepoSignerMap::from(map))
}

pub(super) fn build_registry_client(
    reg: &RegistryConfig,
    global_proxy: Option<&batlehub_config::schema::UpstreamProxyConfig>,
) -> anyhow::Result<Arc<dyn batlehub_core::ports::RegistryClient>> {
    fn resolve_urls(configured: &[String], default: &str) -> Vec<String> {
        if configured.is_empty() {
            vec![default.to_owned()]
        } else {
            configured.to_vec()
        }
    }
    fn make_one(
        kind: RegistryKind,
        url: &str,
        opts: &UpstreamHttpOptions,
    ) -> anyhow::Result<Arc<dyn batlehub_core::ports::RegistryClient>> {
        // Exhaustive match over `RegistryKind`: adding a new variant is a compile
        // error here until an adapter arm is added, instead of silently falling
        // through to a runtime "no adapter compiled in" bail.
        let client: Arc<dyn batlehub_core::ports::RegistryClient> = match kind {
            RegistryKind::Github => Arc::new(GithubRegistryClient::new(url, opts)?),
            RegistryKind::Forgejo => Arc::new(ForgejoRegistryClient::new(url, opts)?),
            RegistryKind::Gitlab => Arc::new(GitlabRegistryClient::new(url, opts)?),
            RegistryKind::Npm => Arc::new(NpmRegistryClient::new(url, opts)?),
            RegistryKind::Cargo => Arc::new(CargoRegistryClient::new(url, opts)?),
            RegistryKind::Nuget => Arc::new(NugetRegistryClient::new(url, opts)?),
            RegistryKind::Openvsx => Arc::new(OpenVsxRegistryClient::new(url, opts)?),
            RegistryKind::Goproxy => Arc::new(GoProxyRegistryClient::new(url, opts)?),
            RegistryKind::VscodeMarketplace => {
                Arc::new(VsCodeMarketplaceRegistryClient::new(url, opts)?)
            }
            RegistryKind::Maven => Arc::new(MavenRegistryClient::new(url, opts)?),
            RegistryKind::Terraform => Arc::new(TerraformRegistryClient::new(url, opts)?),
            RegistryKind::Rubygems => Arc::new(RubyGemsRegistryClient::new(url, opts)?),
            RegistryKind::Composer => Arc::new(ComposerRegistryClient::new(url, opts)?),
            RegistryKind::Pypi => Arc::new(PypiRegistryClient::new(url, opts)?),
            RegistryKind::Conda => Arc::new(CondaRegistryClient::new(url, opts)?),
            RegistryKind::Deb => Arc::new(PathProxyRegistryClient::new("deb", url, opts)?),
            RegistryKind::Rpm => Arc::new(PathProxyRegistryClient::new("rpm", url, opts)?),
            RegistryKind::Pacman => Arc::new(PathProxyRegistryClient::new("pacman", url, opts)?),
            RegistryKind::Jetbrains => {
                Arc::new(PathProxyRegistryClient::new("jetbrains", url, opts)?)
            }
        };
        Ok(client)
    }

    let opts = upstream_options(reg, global_proxy);
    let kind: RegistryKind = reg.registry_type.parse().map_err(anyhow::Error::msg)?;
    let urls = match kind {
        RegistryKind::Github => resolve_urls(&reg.upstreams, "https://api.github.com"),
        RegistryKind::Forgejo => resolve_urls(&reg.upstreams, "https://codeberg.org"),
        RegistryKind::Gitlab => resolve_urls(&reg.upstreams, "https://gitlab.com"),
        RegistryKind::Npm => resolve_urls(&reg.upstreams, "https://registry.npmjs.org"),
        RegistryKind::Cargo => resolve_urls(&reg.upstreams, "https://crates.io"),
        RegistryKind::Nuget => resolve_urls(&reg.upstreams, "https://api.nuget.org"),
        RegistryKind::Openvsx => resolve_urls(&reg.upstreams, "https://open-vsx.org"),
        RegistryKind::Goproxy => resolve_urls(&reg.upstreams, "https://proxy.golang.org"),
        RegistryKind::VscodeMarketplace => {
            resolve_urls(&reg.upstreams, "https://marketplace.visualstudio.com")
        }
        RegistryKind::Maven => resolve_urls(&reg.upstreams, "https://repo1.maven.org/maven2"),
        RegistryKind::Terraform => resolve_urls(&reg.upstreams, "https://registry.terraform.io"),
        RegistryKind::Rubygems => resolve_urls(&reg.upstreams, "https://rubygems.org"),
        RegistryKind::Composer => resolve_urls(&reg.upstreams, "https://repo.packagist.org"),
        RegistryKind::Pypi => resolve_urls(&reg.upstreams, "https://pypi.org"),
        RegistryKind::Conda => resolve_urls(&reg.upstreams, "https://conda.anaconda.org"),
        // Deb/RPM have no universal default upstream; proxy/hybrid mode requires an
        // explicit `upstreams` entry. The placeholder keeps a client constructible
        // for local-only mode, where the upstream is never contacted.
        RegistryKind::Deb => resolve_urls(&reg.upstreams, "https://deb.debian.org"),
        RegistryKind::Rpm => resolve_urls(&reg.upstreams, "https://example.invalid/rpm"),
        // Arch mirrors share a common layout (`$repo/os/$arch/…`); the geo CDN is a
        // sensible default, overridable via `upstreams`.
        RegistryKind::Pacman => resolve_urls(&reg.upstreams, "https://geo.mirror.pkgbuild.com"),
        // JetBrains IDE archives are served from a stable CDN, so it's a sensible
        // default; users can override `upstreams` (e.g. for plugins.jetbrains.com).
        RegistryKind::Jetbrains => resolve_urls(&reg.upstreams, "https://download.jetbrains.com"),
    };
    if urls.len() == 1 {
        make_one(kind, &urls[0], &opts)
    } else {
        let clients = urls
            .iter()
            .map(|u| make_one(kind, u, &opts))
            .collect::<anyhow::Result<Vec<_>>>()?;
        Ok(Arc::new(FanoutRegistryClient::new(
            &reg.registry_type,
            clients,
        )))
    }
}

pub(super) fn build_policy(
    reg: &RegistryConfig,
    repo: Arc<dyn batlehub_core::ports::PackageRepository>,
    vuln_repo: Arc<dyn VulnerabilityRepository>,
) -> RegistryPolicy {
    let mut rules: Vec<Box<dyn batlehub_core::rules::Rule>> = Vec::new();
    let rbac_perms = HashMap::from([
        (Role::Anonymous, reg.rbac.anonymous.clone()),
        (Role::User, reg.rbac.user.clone()),
        (Role::Admin, reg.rbac.admin.clone()),
    ]);
    rules.push(Box::new(
        RbacRule::new(rbac_perms).with_groups(reg.rbac.groups.clone()),
    ));
    rules.push(Box::new(BlockListRule::new(repo)));
    for rule_cfg in &reg.rules {
        match rule_cfg {
            RuleConfig::ReleaseAgeGate(cfg) => {
                let bypass: Vec<Role> = cfg.bypass_roles.iter().map(|r| parse_role(r)).collect();
                rules.push(Box::new(
                    ReleaseAgeGateRule::new(Duration::from_secs(cfg.min_age_secs), bypass)
                        .with_deny_missing_timestamp(cfg.deny_missing_timestamp),
                ));
            }
            RuleConfig::RequireSignedRelease(cfg) => {
                if cfg.enabled {
                    let bypass: Vec<Role> =
                        cfg.bypass_roles.iter().map(|r| parse_role(r)).collect();
                    rules.push(Box::new(
                        RequireSignedReleaseRule::new(bypass)
                            .with_deny_missing_signature(cfg.deny_missing_signature),
                    ));
                }
            }
            RuleConfig::DenyLatest(cfg) => {
                let bypass: Vec<Role> = cfg.bypass_roles.iter().map(|r| parse_role(r)).collect();
                rules.push(Box::new(DenyLatestRule::new(bypass)));
            }
            RuleConfig::CveGate(cfg) => {
                let bypass: Vec<Role> = cfg.bypass_roles.iter().map(|r| parse_role(r)).collect();
                let min_severity = Severity::parse(&cfg.min_severity).unwrap_or(Severity::High);
                rules.push(Box::new(CveGateRule::new(
                    Arc::clone(&vuln_repo),
                    min_severity,
                    bypass,
                    cfg.block,
                )));
            }
            RuleConfig::VersionGate(cfg) => {
                let bypass: Vec<Role> = cfg.bypass_roles.iter().map(|r| parse_role(r)).collect();
                rules.push(Box::new(VersionGateRule::new(
                    &cfg.allow, &cfg.block, bypass,
                )));
            }
            RuleConfig::TrustedPublisher(cfg) => {
                let bypass: Vec<Role> = cfg.bypass_roles.iter().map(|r| parse_role(r)).collect();
                rules.push(Box::new(TrustedPublisherRule::new(&cfg.allow, bypass)));
            }
        }
    }
    RegistryPolicy {
        metadata_ttl: Some(Duration::from_secs(reg.cache.metadata_ttl_secs)),
        firewall_only: reg.firewall_only,
        serve_stale_metadata: reg.cache.serve_stale,
        artifact_ttl: reg.cache.artifact_ttl_secs.map(Duration::from_secs),
        rules,
    }
}

pub(super) fn build_quota_service(
    pool: sqlx::PgPool,
    registries: &[RegistryConfig],
) -> QuotaService {
    let repo = Arc::new(PgQuotaRepository::new(pool));
    let configs = registries
        .iter()
        .filter_map(|reg| {
            reg.quota.as_ref().map(|q| {
                let enforcement = match q.enforcement {
                    ConfigQuotaEnforcement::Block => QuotaEnforcement::Block,
                    ConfigQuotaEnforcement::Warn => QuotaEnforcement::Warn,
                };
                (
                    reg.name.clone(),
                    RegistryQuotaConfig {
                        max_storage_bytes_per_user: q.max_storage_bytes_per_user,
                        max_packages_per_user: q.max_packages_per_user,
                        warn_threshold: q.warn_threshold_pct.clamp(1, 100) as f64 / 100.0,
                        enforcement,
                    },
                )
            })
        })
        .collect();
    QuotaService::new(repo, configs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use batlehub_adapters::in_memory::{
        InMemoryPackageRepository, InMemoryVulnerabilityRepository,
    };
    use batlehub_config::schema::UpstreamProxyConfig;

    fn make_registry(reg_type: &str, name: &str, extra: &str) -> RegistryConfig {
        #[derive(serde::Deserialize)]
        struct Wrapper {
            registries: Vec<RegistryConfig>,
        }
        let toml_str = format!(
            r#"
            [[registries]]
            type = "{reg_type}"
            name = "{name}"
            {extra}
            "#
        );
        let w: Wrapper = toml::from_str(&toml_str).expect("valid registry toml");
        w.registries.into_iter().next().unwrap()
    }

    #[test]
    fn parse_role_variants() {
        assert_eq!(parse_role("admin"), Role::Admin);
        assert_eq!(parse_role("user"), Role::User);
        assert_eq!(parse_role("anonymous"), Role::Anonymous);
        assert_eq!(parse_role("anything-else"), Role::Anonymous);
    }

    #[test]
    fn upstream_options_bearer_auth() {
        let r = make_registry(
            "npm",
            "reg",
            r#"
            [registries.upstream_auth]
            type = "bearer"
            token = "tok123"
            "#,
        );
        let opts = upstream_options(&r, None);
        assert_eq!(opts.bearer_token.as_deref(), Some("tok123"));
        assert!(opts.basic_auth.is_none());
        assert!(opts.custom_header.is_none());
    }

    #[test]
    fn upstream_options_basic_auth() {
        let r = make_registry(
            "npm",
            "reg",
            r#"
            [registries.upstream_auth]
            type = "basic"
            username = "u"
            password = "p"
            "#,
        );
        let opts = upstream_options(&r, None);
        assert_eq!(opts.basic_auth, Some(("u".to_owned(), "p".to_owned())));
        assert!(opts.bearer_token.is_none());
    }

    #[test]
    fn upstream_options_header_auth() {
        let r = make_registry(
            "npm",
            "reg",
            r#"
            [registries.upstream_auth]
            type = "header"
            name = "X-Api-Key"
            value = "secret"
            "#,
        );
        let opts = upstream_options(&r, None);
        assert_eq!(
            opts.custom_header,
            Some(("X-Api-Key".to_owned(), "secret".to_owned()))
        );
    }

    #[test]
    fn upstream_options_proxy_from_registry_overrides_global() {
        let r = make_registry(
            "npm",
            "reg",
            r#"
            [registries.proxy]
            url = "http://reg-proxy:3128"
            "#,
        );
        let global = UpstreamProxyConfig {
            url: "http://global-proxy:3128".into(),
            username: None,
            password: None,
            no_proxy: None,
        };
        let opts = upstream_options(&r, Some(&global));
        assert_eq!(opts.proxy_url.as_deref(), Some("http://reg-proxy:3128"));
    }

    #[test]
    fn upstream_options_proxy_falls_back_to_global() {
        let r = make_registry("npm", "reg", "");
        let global = UpstreamProxyConfig {
            url: "http://global-proxy:3128".into(),
            username: Some("u".into()),
            password: Some("p".into()),
            no_proxy: Some("localhost".into()),
        };
        let opts = upstream_options(&r, Some(&global));
        assert_eq!(opts.proxy_url.as_deref(), Some("http://global-proxy:3128"));
        assert_eq!(opts.proxy_username.as_deref(), Some("u"));
        assert_eq!(opts.proxy_password.as_deref(), Some("p"));
        assert_eq!(opts.no_proxy.as_deref(), Some("localhost"));
    }

    #[test]
    fn upstream_options_search_url_and_ca_cert() {
        let r = make_registry(
            "maven",
            "reg",
            r#"
            search_url = "https://search.example.com"
            [registries.tls]
            ca_cert_path = "/etc/ssl/ca.pem"
            "#,
        );
        let opts = upstream_options(&r, None);
        assert_eq!(
            opts.search_url.as_deref(),
            Some("https://search.example.com")
        );
        assert_eq!(opts.ca_cert_path.as_deref(), Some("/etc/ssl/ca.pem"));
    }

    #[test]
    fn build_cargo_index_uses_explicit_index_url() {
        let r = make_registry(
            "cargo",
            "reg",
            r#"index_url = "https://my-index.example.com""#,
        );
        let proxy = build_cargo_index(&r, None).unwrap();
        assert_eq!(proxy.index_url, "https://my-index.example.com");
    }

    #[test]
    fn build_cargo_index_defaults_crates_io_to_index_crates_io() {
        let r = make_registry("cargo", "reg", "");
        let proxy = build_cargo_index(&r, None).unwrap();
        assert_eq!(proxy.index_url, "https://index.crates.io");
    }

    #[test]
    fn build_cargo_index_non_crates_upstream_used_directly() {
        let r = make_registry(
            "cargo",
            "reg",
            r#"upstreams = ["https://my-mirror.example.com"]"#,
        );
        let proxy = build_cargo_index(&r, None).unwrap();
        assert_eq!(proxy.index_url, "https://my-mirror.example.com");
    }

    #[test]
    fn build_registry_client_unknown_type_errors() {
        let r = make_registry("not-a-real-type", "reg", "");
        assert!(build_registry_client(&r, None).is_err());
    }

    #[test]
    fn build_registry_client_single_upstream() {
        let r = make_registry("npm", "reg", "");
        let client = build_registry_client(&r, None).unwrap();
        assert_eq!(client.registry_type(), "npm");
    }

    #[test]
    fn build_registry_client_multi_upstream_uses_fanout() {
        let r = make_registry(
            "npm",
            "reg",
            r#"upstreams = ["https://a.example.com", "https://b.example.com"]"#,
        );
        let client = build_registry_client(&r, None).unwrap();
        assert_eq!(client.registry_type(), "npm");
    }

    #[test]
    fn build_policy_default_has_rbac_and_block_list_rules() {
        let r = make_registry("npm", "reg", "");
        let repo: Arc<dyn batlehub_core::ports::PackageRepository> =
            InMemoryPackageRepository::new();
        let policy = build_policy(&r, repo, InMemoryVulnerabilityRepository::arc());
        let names: Vec<&str> = policy.rules.iter().map(|rule| rule.name()).collect();
        assert_eq!(names, vec!["rbac", "block_list"]);
        assert!(!policy.firewall_only);
        assert!(policy.serve_stale_metadata);
        assert_eq!(policy.metadata_ttl, Some(Duration::from_secs(300)));
        assert!(policy.artifact_ttl.is_none());
    }

    #[test]
    fn build_policy_with_release_age_gate_and_deny_latest_rules() {
        let r = make_registry(
            "npm",
            "reg",
            r#"
            firewall_only = true

            [registries.cache]
            metadata_ttl_secs = 60
            serve_stale = false
            artifact_ttl_secs = 3600

            [[registries.rules]]
            kind = "release_age_gate"
            min_age_secs = 7200
            bypass_roles = ["admin"]

            [[registries.rules]]
            kind = "deny_latest"
            bypass_roles = ["user"]

            [[registries.rules]]
            kind = "require_signed_release"
            enabled = true
            "#,
        );
        let repo: Arc<dyn batlehub_core::ports::PackageRepository> =
            InMemoryPackageRepository::new();
        let policy = build_policy(&r, repo, InMemoryVulnerabilityRepository::arc());
        let names: Vec<&str> = policy.rules.iter().map(|rule| rule.name()).collect();
        assert_eq!(
            names,
            vec![
                "rbac",
                "block_list",
                "release_age_gate",
                "deny_latest",
                "require_signed_release"
            ]
        );
        assert!(policy.firewall_only);
        assert!(!policy.serve_stale_metadata);
        assert_eq!(policy.metadata_ttl, Some(Duration::from_secs(60)));
        assert_eq!(policy.artifact_ttl, Some(Duration::from_secs(3600)));
    }

    #[test]
    fn build_policy_appends_cve_gate_rule() {
        let r = make_registry(
            "cargo",
            "reg",
            r#"
            [[registries.rules]]
            kind = "cve_gate"
            min_severity = "critical"
            block = true
            bypass_roles = ["admin"]
            "#,
        );
        let repo: Arc<dyn batlehub_core::ports::PackageRepository> =
            InMemoryPackageRepository::new();
        let policy = build_policy(&r, repo, InMemoryVulnerabilityRepository::arc());
        let names: Vec<&str> = policy.rules.iter().map(|rule| rule.name()).collect();
        assert_eq!(names, vec!["rbac", "block_list", "cve_gate"]);
    }

    #[test]
    fn build_policy_appends_trusted_publisher_rule() {
        let r = make_registry(
            "github",
            "reg",
            r#"
            [[registries.rules]]
            kind = "trusted_publisher"
            allow = ["my-org"]
            bypass_roles = ["admin"]
            "#,
        );
        let repo: Arc<dyn batlehub_core::ports::PackageRepository> =
            InMemoryPackageRepository::new();
        let policy = build_policy(&r, repo, InMemoryVulnerabilityRepository::arc());
        let names: Vec<&str> = policy.rules.iter().map(|rule| rule.name()).collect();
        assert_eq!(names, vec!["rbac", "block_list", "trusted_publisher"]);
    }
}
