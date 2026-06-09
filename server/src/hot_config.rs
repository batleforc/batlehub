use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use anyhow::Context;

use batlehub_config::schema::{AppConfig, RegistryConfig, RegistryMode};
use batlehub_core::ports::{BetaChannelPort, PackageRepository};
use batlehub_core::services::{
    HotConfig, HotSbomConfig, SigningConfig as CoreSigningConfig, VersioningPolicy,
};
use batlehub_web::{
    AccessConfig, CargoIndexMap, CargoIndexProxy, RegistryMap, RegistryModeMap, UpstreamMap,
};

fn build_versioning_map(registries: &[RegistryConfig]) -> HashMap<String, VersioningPolicy> {
    registries
        .iter()
        .filter_map(|reg| {
            reg.versioning.as_ref().map(|v| {
                let pattern = v
                    .version_pattern
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
                (
                    reg.name.clone(),
                    VersioningPolicy {
                        enforce_semver: v.enforce_semver,
                        allow_prerelease: v.allow_prerelease,
                        version_pattern: pattern,
                    },
                )
            })
        })
        .collect()
}

fn build_signing_map(registries: &[RegistryConfig]) -> HashMap<String, CoreSigningConfig> {
    registries
        .iter()
        .filter_map(|reg| {
            reg.signing.as_ref().map(|s| {
                (
                    reg.name.clone(),
                    CoreSigningConfig {
                        required: s.required,
                        allowed_types: s.allowed_types.clone(),
                    },
                )
            })
        })
        .collect()
}

fn build_sbom_map(registries: &[RegistryConfig]) -> HashMap<String, HotSbomConfig> {
    registries
        .iter()
        .filter_map(|reg| {
            reg.sbom.as_ref().map(|s| {
                (
                    reg.name.clone(),
                    HotSbomConfig {
                        enabled: s.enabled,
                        formats: s.formats.clone(),
                        required: s.required,
                        fetch_upstream: s.fetch_upstream,
                        registry_type: reg.registry_type.clone(),
                    },
                )
            })
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
    let default_url = match reg.registry_type.as_str() {
        "npm" => "https://registry.npmjs.org",
        "terraform" => "https://registry.terraform.io",
        "pypi" => "https://pypi.org",
        "conda" => "https://conda.anaconda.org",
        "nuget" => "https://api.nuget.org",
        _ => return None,
    };
    Some(
        reg.upstreams
            .first()
            .cloned()
            .unwrap_or_else(|| default_url.to_owned()),
    )
}

pub(super) fn build_hot_bundle(
    cfg: &AppConfig,
    beta_channel_store: &Arc<dyn BetaChannelPort>,
    repo: &Arc<dyn PackageRepository>,
) -> anyhow::Result<(
    HotConfig,
    AccessConfig,
    RegistryMap,
    RegistryModeMap,
    UpstreamMap,
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
        reg_policies.insert(
            reg.name.clone(),
            Arc::new(crate::builders::build_policy(reg, Arc::clone(repo))),
        );
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
        beta_channel: build_beta_channel_map(Arc::clone(beta_channel_store), &cfg.registries),
        max_artifact_size_bytes: cfg.limits.max_artifact_size_bytes,
    };

    Ok((
        hot,
        build_access_config(cfg),
        RegistryMap::from(reg_type_map),
        RegistryModeMap::from(reg_mode_map),
        UpstreamMap::from(upstream_map),
    ))
}

pub(super) fn build_access_config(config: &AppConfig) -> AccessConfig {
    let mut group_access: HashMap<String, HashSet<String>> = HashMap::new();
    for r in &config.registries {
        for group_name in r.rbac.groups.keys() {
            group_access
                .entry(group_name.clone())
                .or_default()
                .insert(r.name.clone());
        }
    }
    AccessConfig {
        anonymous: config
            .registries
            .iter()
            .filter(|r| !r.rbac.anonymous.is_empty())
            .map(|r| r.name.clone())
            .collect(),
        user: config
            .registries
            .iter()
            .filter(|r| !r.rbac.anonymous.is_empty() || !r.rbac.user.is_empty())
            .map(|r| r.name.clone())
            .collect(),
        admin: config
            .registries
            .iter()
            .filter(|r| {
                !r.rbac.anonymous.is_empty() || !r.rbac.user.is_empty() || !r.rbac.admin.is_empty()
            })
            .map(|r| r.name.clone())
            .collect(),
        groups: group_access,
        explore_anonymous: config
            .registries
            .iter()
            .filter(|r| !r.rbac.anonymous.is_empty() && r.rbac.explore.anonymous)
            .map(|r| r.name.clone())
            .collect(),
        explore_user: config
            .registries
            .iter()
            .filter(|r| {
                (!r.rbac.anonymous.is_empty() || !r.rbac.user.is_empty()) && r.rbac.explore.user
            })
            .map(|r| r.name.clone())
            .collect(),
        explore_admin: config
            .registries
            .iter()
            .filter(|r| {
                (!r.rbac.anonymous.is_empty()
                    || !r.rbac.user.is_empty()
                    || !r.rbac.admin.is_empty())
                    && r.rbac.explore.admin
            })
            .map(|r| r.name.clone())
            .collect(),
    }
}

pub(super) fn make_hot_builder(
    beta_channel_store: Arc<dyn BetaChannelPort>,
    repo: Arc<dyn PackageRepository>,
) -> batlehub_web::services::HotConfigBuilder {
    Arc::new(move |cfg: &AppConfig| {
        let (hot, access, rm, rmm, um) = build_hot_bundle(cfg, &beta_channel_store, &repo)?;
        let mut cargo_map: HashMap<String, CargoIndexProxy> = HashMap::new();
        for reg in &cfg.registries {
            if reg.registry_type == "cargo" && !matches!(reg.mode, RegistryMode::Local) {
                let index = crate::builders::build_cargo_index(reg, cfg.proxy.as_ref())
                    .with_context(|| format!("building cargo index client for '{}'", reg.name))?;
                cargo_map.insert(reg.name.clone(), index);
            }
        }
        Ok((hot, access, rm, rmm, um, CargoIndexMap::new(cargo_map)))
    })
}
