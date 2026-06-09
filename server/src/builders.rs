use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use batlehub_adapters::db::PgQuotaRepository;
use batlehub_adapters::registry::{
    CargoRegistryClient, ComposerRegistryClient, CondaRegistryClient, FanoutRegistryClient,
    GithubRegistryClient, GoProxyRegistryClient, MavenRegistryClient, NpmRegistryClient,
    NugetRegistryClient, OpenVsxRegistryClient, PypiRegistryClient, RubyGemsRegistryClient,
    TerraformRegistryClient, UpstreamHttpOptions, VsCodeMarketplaceRegistryClient,
};
use batlehub_config::schema::{
    QuotaEnforcement as ConfigQuotaEnforcement, RegistryConfig, RuleConfig, UpstreamAuthConfig,
};
use batlehub_core::{
    entities::Role,
    rules::{BlockListRule, DenyLatestRule, RbacRule, ReleaseAgeGateRule},
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
        registry_type: &str,
        url: &str,
        opts: &UpstreamHttpOptions,
    ) -> anyhow::Result<Arc<dyn batlehub_core::ports::RegistryClient>> {
        let client: Arc<dyn batlehub_core::ports::RegistryClient> = match registry_type {
            "github" => Arc::new(GithubRegistryClient::new(url, opts)?),
            "npm" => Arc::new(NpmRegistryClient::new(url, opts)?),
            "cargo" => Arc::new(CargoRegistryClient::new(url, opts)?),
            "nuget" => Arc::new(NugetRegistryClient::new(url, opts)?),
            "openvsx" => Arc::new(OpenVsxRegistryClient::new(url, opts)?),
            "goproxy" => Arc::new(GoProxyRegistryClient::new(url, opts)?),
            "vscode-marketplace" => Arc::new(VsCodeMarketplaceRegistryClient::new(url, opts)?),
            "maven" => Arc::new(MavenRegistryClient::new(url, opts)?),
            "terraform" => Arc::new(TerraformRegistryClient::new(url, opts)?),
            "rubygems" => Arc::new(RubyGemsRegistryClient::new(url, opts)?),
            "composer" => Arc::new(ComposerRegistryClient::new(url, opts)?),
            "pypi" => Arc::new(PypiRegistryClient::new(url, opts)?),
            "conda" => Arc::new(CondaRegistryClient::new(url, opts)?),
            other => {
                anyhow::bail!("registry type '{other}' is configured but no adapter is compiled in")
            }
        };
        Ok(client)
    }

    let opts = upstream_options(reg, global_proxy);
    let urls = match reg.registry_type.as_str() {
        "github" => resolve_urls(&reg.upstreams, "https://api.github.com"),
        "npm" => resolve_urls(&reg.upstreams, "https://registry.npmjs.org"),
        "cargo" => resolve_urls(&reg.upstreams, "https://crates.io"),
        "nuget" => resolve_urls(&reg.upstreams, "https://api.nuget.org"),
        "openvsx" => resolve_urls(&reg.upstreams, "https://open-vsx.org"),
        "goproxy" => resolve_urls(&reg.upstreams, "https://proxy.golang.org"),
        "vscode-marketplace" => {
            resolve_urls(&reg.upstreams, "https://marketplace.visualstudio.com")
        }
        "maven" => resolve_urls(&reg.upstreams, "https://repo1.maven.org/maven2"),
        "terraform" => resolve_urls(&reg.upstreams, "https://registry.terraform.io"),
        "rubygems" => resolve_urls(&reg.upstreams, "https://rubygems.org"),
        "composer" => resolve_urls(&reg.upstreams, "https://repo.packagist.org"),
        "pypi" => resolve_urls(&reg.upstreams, "https://pypi.org"),
        "conda" => resolve_urls(&reg.upstreams, "https://conda.anaconda.org"),
        other => {
            anyhow::bail!("registry type '{other}' is configured but no adapter is compiled in")
        }
    };
    if urls.len() == 1 {
        make_one(&reg.registry_type, &urls[0], &opts)
    } else {
        let clients = urls
            .iter()
            .map(|u| make_one(&reg.registry_type, u, &opts))
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
                    tracing::warn!(
                        "require_signed_release rule is configured but not yet implemented"
                    );
                }
            }
            RuleConfig::DenyLatest(cfg) => {
                let bypass: Vec<Role> = cfg.bypass_roles.iter().map(|r| parse_role(r)).collect();
                rules.push(Box::new(DenyLatestRule::new(bypass)));
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
