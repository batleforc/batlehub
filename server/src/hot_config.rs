use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use anyhow::Context;

use batlehub_config::schema::{AppConfig, RegistryConfig, RegistryMode};
use batlehub_core::entities::RegistryKind;
use batlehub_core::ports::{BetaChannelPort, PackageRepository, VulnerabilityRepository};
use batlehub_core::services::{
    FeatureFlags, HotConfig, HotSbomConfig, IntegrityPolicy, SigningConfig as CoreSigningConfig,
    VersioningPolicy,
};

use crate::builders::parse_role;
use batlehub_web::{
    AccessConfig, CargoIndexMap, CargoIndexProxy, RegistryMap, RegistryModeMap, UpstreamMap,
    VulnDbMap,
};

/// Shared shape for the `build_*_map` functions below: an optional per-registry
/// config sub-block (`extract`) becomes one map entry keyed by registry name
/// (`build`), with registries where `extract` returns `None` simply absent
/// from the result. Field-by-field mapping stays inline in each `build`
/// closure rather than being hidden behind the helper.
fn map_registries<T, U>(
    registries: &[RegistryConfig],
    extract: impl Fn(&RegistryConfig) -> Option<&T>,
    build: impl Fn(&RegistryConfig, &T) -> U,
) -> HashMap<String, U> {
    registries
        .iter()
        .filter_map(|reg| extract(reg).map(|v| (reg.name.clone(), build(reg, v))))
        .collect()
}

fn build_versioning_map(registries: &[RegistryConfig]) -> HashMap<String, VersioningPolicy> {
    map_registries(
        registries,
        |reg| reg.versioning.as_ref(),
        |reg, v| {
            let pattern =
                v.version_pattern
                    .as_deref()
                    .and_then(|pat| match regex::Regex::new(pat) {
                        Ok(re) => Some(re),
                        Err(e) => {
                            tracing::warn!(
                                "invalid version_pattern for registry '{}': {e}",
                                reg.name
                            );
                            None
                        }
                    });
            VersioningPolicy {
                enforce_semver: v.enforce_semver,
                allow_prerelease: v.allow_prerelease,
                version_pattern: pattern,
            }
        },
    )
}

/// Build the per-registry integrity policy map. Only registries with an explicit
/// `[registries.integrity]` block get an entry; the proxy applies
/// [`IntegrityPolicy::default`] (verify + block-on-mismatch) to the rest.
fn build_integrity_map(registries: &[RegistryConfig]) -> HashMap<String, IntegrityPolicy> {
    map_registries(
        registries,
        |reg| reg.integrity.as_ref(),
        |reg, i| IntegrityPolicy {
            enabled: i.enabled,
            block_on_mismatch: i.block_on_mismatch,
            require_metadata: i.require_metadata,
            // `map_registries`'s per-field builder can't fail the whole reload over
            // one bad role, so — like `build_versioning_map`'s regex handling above
            // — an unrecognized entry is dropped with a warning rather than
            // silently coerced to `anonymous`.
            bypass_roles: i
                .bypass_roles
                .iter()
                .filter_map(|r| match parse_role(r) {
                    Ok(role) => Some(role),
                    Err(e) => {
                        tracing::warn!(
                            "invalid integrity bypass_roles entry for registry '{}': {e}",
                            reg.name
                        );
                        None
                    }
                })
                .collect(),
            verify_on_serve: i.verify_on_serve,
        },
    )
}

fn build_signing_map(registries: &[RegistryConfig]) -> HashMap<String, CoreSigningConfig> {
    map_registries(
        registries,
        |reg| reg.signing.as_ref(),
        |_reg, s| CoreSigningConfig {
            required: s.required,
            allowed_types: s.allowed_types.clone(),
            verify_on_download: s.verify_on_download,
            trusted_keys: s.trusted_keys.clone(),
        },
    )
}

fn build_sbom_map(registries: &[RegistryConfig]) -> HashMap<String, HotSbomConfig> {
    map_registries(
        registries,
        |reg| reg.sbom.as_ref(),
        |reg, s| HotSbomConfig {
            enabled: s.enabled,
            formats: s.formats.clone(),
            required: s.required,
            fetch_upstream: s.fetch_upstream,
            registry_type: reg.registry_type.clone(),
        },
    )
}

fn build_feature_flags_map(registries: &[RegistryConfig]) -> HashMap<String, FeatureFlags> {
    // Populate every registry: flags default to "on", so a registry without a
    // `[registries.feature_flags]` block still gets the default (badge shown).
    registries
        .iter()
        .map(|reg| {
            let flags = reg
                .feature_flags
                .as_ref()
                .map_or_else(FeatureFlags::default, |f| FeatureFlags {
                    socket_badge: f.socket_badge,
                });
            (reg.name.clone(), flags)
        })
        .collect()
}

fn build_beta_channel_map(
    store: Arc<dyn BetaChannelPort>,
    registries: &[RegistryConfig],
) -> HashMap<String, Arc<dyn BetaChannelPort>> {
    registries
        .iter()
        .filter(|reg| reg.beta_channel.as_ref().is_some_and(|bc| bc.enabled))
        .map(|reg| (reg.name.clone(), Arc::clone(&store)))
        .collect()
}

pub(super) fn upstream_url_for(reg: &RegistryConfig) -> Option<String> {
    let kind: RegistryKind = reg.registry_type.parse().ok()?;
    let default_url = match kind {
        RegistryKind::Npm => "https://registry.npmjs.org",
        RegistryKind::Terraform => "https://registry.terraform.io",
        RegistryKind::Pypi => "https://pypi.org",
        RegistryKind::Conda => "https://conda.anaconda.org",
        RegistryKind::Nuget => "https://api.nuget.org",
        RegistryKind::Composer => "https://packagist.org",
        _ => return None,
    };
    Some(
        reg.upstreams
            .first()
            .cloned()
            .unwrap_or_else(|| default_url.to_owned()),
    )
}

fn build_vuln_db_map(registries: &[RegistryConfig]) -> VulnDbMap {
    const DEFAULT: &str = "https://vuln.go.dev";
    let urls = registries
        .iter()
        .filter(|r| r.registry_type == RegistryKind::Goproxy.as_str())
        .filter_map(|r| match r.vuln_db_url.as_deref() {
            Some("") => None,
            Some(url) => Some((r.name.clone(), url.trim_end_matches('/').to_owned())),
            None => Some((r.name.clone(), DEFAULT.to_owned())),
        })
        .collect();
    VulnDbMap::new(urls)
}

pub(super) fn build_hot_bundle(
    cfg: &AppConfig,
    beta_channel_store: &Arc<dyn BetaChannelPort>,
    repo: &Arc<dyn PackageRepository>,
    vuln_repo: &Arc<dyn VulnerabilityRepository>,
) -> anyhow::Result<(
    HotConfig,
    AccessConfig,
    RegistryMap,
    RegistryModeMap,
    UpstreamMap,
    VulnDbMap,
)> {
    let mut reg_clients: HashMap<String, Arc<dyn batlehub_core::ports::RegistryClient>> =
        HashMap::new();
    let mut reg_policies: HashMap<String, Arc<batlehub_core::services::RegistryPolicy>> =
        HashMap::new();
    let mut reg_type_map: HashMap<String, String> = HashMap::new();
    let mut reg_mode_map: HashMap<String, RegistryMode> = HashMap::new();
    let mut upstream_map: HashMap<String, String> = HashMap::new();

    for reg in &cfg.registries {
        let client = crate::builders::build_registry_client(reg, cfg.proxy.as_ref())
            .with_context(|| format!("building registry client for '{}'", reg.name))?;
        reg_clients.insert(reg.name.clone(), client);
        let policy = crate::builders::build_policy(reg, Arc::clone(repo), Arc::clone(vuln_repo))
            .with_context(|| format!("building policy for '{}'", reg.name))?;
        reg_policies.insert(reg.name.clone(), Arc::new(policy));
        reg_type_map.insert(reg.name.clone(), reg.registry_type.clone());
        reg_mode_map.insert(reg.name.clone(), reg.mode.clone());
        if let Some(url) = upstream_url_for(reg) {
            upstream_map.insert(reg.name.clone(), url);
        }
    }

    let hot = HotConfig {
        registries: reg_clients,
        policies: reg_policies,
        versioning: build_versioning_map(&cfg.registries),
        signing: build_signing_map(&cfg.registries),
        sbom: build_sbom_map(&cfg.registries),
        feature_flags: build_feature_flags_map(&cfg.registries),
        integrity: build_integrity_map(&cfg.registries),
        beta_channel: build_beta_channel_map(Arc::clone(beta_channel_store), &cfg.registries),
        max_artifact_size_bytes: cfg.limits.max_artifact_size_bytes,
    };

    Ok((
        hot,
        build_access_config(cfg),
        RegistryMap::from(reg_type_map),
        RegistryModeMap::from(reg_mode_map),
        UpstreamMap::from(upstream_map),
        build_vuln_db_map(&cfg.registries),
    ))
}

pub(super) fn build_access_config(config: &AppConfig) -> AccessConfig {
    let mut group_access: HashMap<String, HashSet<String>> = HashMap::new();
    let mut anonymous = HashSet::new();
    let mut user = HashSet::new();
    let mut admin = HashSet::new();
    let mut explore_anonymous = HashSet::new();
    let mut explore_user = HashSet::new();
    let mut explore_admin = HashSet::new();

    // Each registry's proxy-access tier is cumulative (admin implies user implies
    // anonymous — an admin-only registry is still "admin accessible" even with an
    // empty `rbac.anonymous`/`rbac.user`), and explore access is gated by both the
    // matching proxy tier and its own `rbac.explore.*` flag. Computed once per
    // registry in a single pass instead of six independent filter/map/collect
    // passes each restating the same cumulative conditions.
    for r in &config.registries {
        for group_name in r.rbac.groups.keys() {
            group_access
                .entry(group_name.clone())
                .or_default()
                .insert(r.name.clone());
        }

        let has_anonymous = !r.rbac.anonymous.is_empty();
        let has_user = has_anonymous || !r.rbac.user.is_empty();
        let has_admin = has_user || !r.rbac.admin.is_empty();

        if has_anonymous {
            anonymous.insert(r.name.clone());
        }
        if has_user {
            user.insert(r.name.clone());
        }
        if has_admin {
            admin.insert(r.name.clone());
        }
        if has_anonymous && r.rbac.explore.anonymous {
            explore_anonymous.insert(r.name.clone());
        }
        if has_user && r.rbac.explore.user {
            explore_user.insert(r.name.clone());
        }
        if has_admin && r.rbac.explore.admin {
            explore_admin.insert(r.name.clone());
        }
    }

    AccessConfig {
        anonymous,
        user,
        admin,
        groups: group_access,
        explore_anonymous,
        explore_user,
        explore_admin,
    }
}

pub(super) fn make_hot_builder(
    beta_channel_store: Arc<dyn BetaChannelPort>,
    repo: Arc<dyn PackageRepository>,
    vuln_repo: Arc<dyn VulnerabilityRepository>,
) -> batlehub_web::services::HotConfigBuilder {
    Arc::new(move |cfg: &AppConfig| {
        let (hot, access, rm, rmm, um, vuln_db) =
            build_hot_bundle(cfg, &beta_channel_store, &repo, &vuln_repo)?;
        let mut cargo_map: HashMap<String, CargoIndexProxy> = HashMap::new();
        for reg in &cfg.registries {
            if reg.registry_type == "cargo" && !matches!(reg.mode, RegistryMode::Local) {
                let index = crate::builders::build_cargo_index(reg, cfg.proxy.as_ref())
                    .with_context(|| format!("building cargo index client for '{}'", reg.name))?;
                cargo_map.insert(reg.name.clone(), index);
            }
        }
        let repo_signer_map = crate::builders::build_repo_signer_map(cfg)?;
        Ok((
            hot,
            access,
            rm,
            rmm,
            um,
            CargoIndexMap::new(cargo_map),
            repo_signer_map,
            vuln_db,
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use batlehub_adapters::in_memory::InMemoryBetaChannelStore;

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

    fn make_app_config(registries_toml: &str) -> AppConfig {
        let toml_str = format!(
            r#"
            [server]
            host = "127.0.0.1"
            port = 8080

            [database]
            type = "postgresql"
            url = "postgresql://user:pass@localhost/db"

            [storage]
            type = "filesystem"
            path = "./tmp"

            {registries_toml}
            "#
        );
        toml::from_str(&toml_str).expect("valid app config toml")
    }

    #[test]
    fn build_versioning_map_valid_pattern() {
        let r = make_registry(
            "npm",
            "test-reg",
            r#"
            [registries.versioning]
            enforce_semver = true
            allow_prerelease = false
            version_pattern = "^[0-9]+\\.[0-9]+\\.[0-9]+$"
            "#,
        );
        let map = build_versioning_map(&[r]);
        let policy = map.get("test-reg").expect("entry present");
        assert!(policy.enforce_semver);
        assert!(!policy.allow_prerelease);
        assert!(policy.version_pattern.is_some());
    }

    #[test]
    fn build_versioning_map_invalid_pattern_becomes_none() {
        let r = make_registry(
            "npm",
            "test-reg",
            r#"
            [registries.versioning]
            version_pattern = "[invalid("
            "#,
        );
        let map = build_versioning_map(&[r]);
        let policy = map.get("test-reg").expect("entry present");
        assert!(policy.version_pattern.is_none());
    }

    #[test]
    fn build_versioning_map_absent_for_unconfigured_registry() {
        let r = make_registry("npm", "test-reg", "");
        assert!(build_versioning_map(&[r]).is_empty());
    }

    #[test]
    fn build_integrity_map_present_parses_bypass_roles() {
        let r = make_registry(
            "cargo",
            "test-reg",
            r#"
            [registries.integrity]
            enabled = true
            block_on_mismatch = false
            require_metadata = true
            bypass_roles = ["admin"]
            "#,
        );
        let map = build_integrity_map(&[r]);
        let cfg = map.get("test-reg").expect("entry present");
        assert!(cfg.enabled);
        assert!(!cfg.block_on_mismatch);
        assert!(cfg.require_metadata);
        assert_eq!(cfg.bypass_roles, vec![batlehub_core::entities::Role::Admin]);
    }

    #[test]
    fn build_integrity_map_absent_for_unconfigured_registry() {
        // No [registries.integrity] block → no entry; the proxy applies the
        // default policy (verify + block-on-mismatch) via unwrap_or_default().
        let r = make_registry("npm", "test-reg", "");
        assert!(build_integrity_map(&[r]).is_empty());
    }

    #[test]
    fn build_signing_map_present() {
        let r = make_registry(
            "npm",
            "test-reg",
            r#"
            [registries.signing]
            required = true
            allowed_types = ["pgp", "ed25519"]
            "#,
        );
        let map = build_signing_map(&[r]);
        let cfg = map.get("test-reg").expect("entry present");
        assert!(cfg.required);
        assert_eq!(
            cfg.allowed_types,
            vec!["pgp".to_owned(), "ed25519".to_owned()]
        );
    }

    #[test]
    fn build_signing_map_absent_for_unconfigured_registry() {
        let r = make_registry("npm", "test-reg", "");
        assert!(build_signing_map(&[r]).is_empty());
    }

    #[test]
    fn build_sbom_map_present_carries_registry_type() {
        let r = make_registry(
            "maven",
            "test-reg",
            r#"
            [registries.sbom]
            enabled = true
            formats = ["spdx"]
            required = true
            fetch_upstream = false
            "#,
        );
        let map = build_sbom_map(&[r]);
        let cfg = map.get("test-reg").expect("entry present");
        assert!(cfg.enabled);
        assert_eq!(cfg.formats, vec!["spdx".to_owned()]);
        assert!(cfg.required);
        assert!(!cfg.fetch_upstream);
        assert_eq!(cfg.registry_type, "maven");
    }

    #[test]
    fn build_sbom_map_absent_for_unconfigured_registry() {
        let r = make_registry("npm", "test-reg", "");
        assert!(build_sbom_map(&[r]).is_empty());
    }

    #[test]
    fn build_feature_flags_map_defaults_on_and_respects_override() {
        let default_reg = make_registry("npm", "default-reg", "");
        let disabled_reg = make_registry(
            "cargo",
            "disabled-reg",
            "[registries.feature_flags]\nsocket_badge = false",
        );
        let map = build_feature_flags_map(&[default_reg, disabled_reg]);
        // Every registry gets an entry (default-on when the block is absent).
        assert_eq!(map.len(), 2);
        assert!(map["default-reg"].socket_badge);
        assert!(!map["disabled-reg"].socket_badge);
    }

    #[test]
    fn build_beta_channel_map_only_includes_enabled_registries() {
        let store: Arc<dyn BetaChannelPort> = InMemoryBetaChannelStore::new();
        let enabled = make_registry(
            "npm",
            "enabled-reg",
            "[registries.beta_channel]\nenabled = true",
        );
        let disabled = make_registry(
            "npm",
            "disabled-reg",
            "[registries.beta_channel]\nenabled = false",
        );
        let absent = make_registry("npm", "absent-reg", "");

        let map = build_beta_channel_map(store, &[enabled, disabled, absent]);
        assert_eq!(map.len(), 1);
        assert!(map.contains_key("enabled-reg"));
    }

    #[test]
    fn upstream_url_for_known_type_default() {
        let r = make_registry("npm", "npm-reg", "");
        assert_eq!(
            upstream_url_for(&r),
            Some("https://registry.npmjs.org".to_owned())
        );
    }

    #[test]
    fn upstream_url_for_configured_override() {
        let r = make_registry(
            "npm",
            "npm-reg",
            r#"upstreams = ["https://npm.example.com"]"#,
        );
        assert_eq!(
            upstream_url_for(&r),
            Some("https://npm.example.com".to_owned())
        );
    }

    #[test]
    fn upstream_url_for_composer_default() {
        let r = make_registry("composer", "composer-reg", "");
        assert_eq!(
            upstream_url_for(&r),
            Some("https://packagist.org".to_owned())
        );
    }

    #[test]
    fn upstream_url_for_unknown_type_returns_none() {
        let r = make_registry("github", "gh-reg", "");
        assert_eq!(upstream_url_for(&r), None);
    }

    #[test]
    fn build_access_config_table_driven() {
        let cfg = make_app_config(
            r#"
            [[registries]]
            type = "npm"
            name = "anon-reg"
            [registries.rbac]
            anonymous = ["read"]

            [[registries]]
            type = "npm"
            name = "user-reg"
            [registries.rbac]
            user = ["read"]

            [[registries]]
            type = "npm"
            name = "admin-reg"
            [registries.rbac]
            admin = ["read"]

            [[registries]]
            type = "npm"
            name = "no-access-reg"

            [[registries]]
            type = "npm"
            name = "group-reg"
            [registries.rbac.groups]
            ci-bots = ["read"]

            [[registries]]
            type = "npm"
            name = "explore-reg"
            [registries.rbac]
            anonymous = ["read"]
            user = ["read"]
            admin = ["read"]
            [registries.rbac.explore]
            anonymous = false
            user = true
            admin = false
            "#,
        );

        let access = build_access_config(&cfg);

        assert!(access.anonymous.contains("anon-reg"));
        assert!(!access.anonymous.contains("user-reg"));

        assert!(access.user.contains("anon-reg"));
        assert!(access.user.contains("user-reg"));
        assert!(!access.user.contains("admin-reg"));

        assert!(access.admin.contains("anon-reg"));
        assert!(access.admin.contains("user-reg"));
        assert!(access.admin.contains("admin-reg"));
        assert!(!access.admin.contains("no-access-reg"));

        assert_eq!(
            access.groups.get("ci-bots"),
            Some(&HashSet::from(["group-reg".to_owned()]))
        );

        assert!(!access.explore_anonymous.contains("explore-reg"));
        assert!(access.explore_user.contains("explore-reg"));
        assert!(!access.explore_admin.contains("explore-reg"));
    }
}
