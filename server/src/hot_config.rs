use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;

use batlehub_config::schema::{AppConfig, RegistryConfig, RegistryMode};
use batlehub_core::entities::RegistryKind;
use batlehub_core::ports::{
    BetaChannelPort, PackageRepository, SbomRepository, VulnerabilityRepository,
};
use batlehub_core::services::{
    FeatureFlags, HotConfig, HotReadmeConfig, HotSbomConfig, HotUpstreamDetailConfig,
    IntegrityPolicy, RemoteImagePolicy, RetentionPolicy, SignedUrlService,
    SigningConfig as CoreSigningConfig, VersioningPolicy,
};

use crate::builders::parse_role;
use batlehub_web::{
    AccessConfig, CargoIndexMap, CargoIndexProxy, RegistryHostMap, RegistryMap, RegistryModeMap,
    SumDbMap, UpstreamMap, VulnDbMap,
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

/// Only the registries that wrote a `[registries.retention]` block, the same way
/// `build_sbom_map` works and the opposite of `build_readme_map`: an absent entry
/// means keep everything forever, which is the default and needs no row.
fn build_retention_map(registries: &[RegistryConfig]) -> HashMap<String, RetentionPolicy> {
    map_registries(
        registries,
        |reg| reg.retention.as_ref(),
        |_reg, r| RetentionPolicy {
            // `validate()` has already refused 0 and anything under the 30-day
            // floor, so this multiplication cannot produce a window that strips
            // detail the moment it is written.
            tombstone_detail_for: r
                .tombstone_detail_for_days
                .map(|d| Duration::from_secs(u64::from(d) * 24 * 60 * 60)),
            dry_run: r.dry_run,
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

/// Populate every registry, because the absence of a `[registries.readme]`
/// block means **on** (RFC 0007 §4.1) — the opposite of `[registries.sbom]`.
/// A map keyed only by the registries that wrote the block down would make the
/// default "off" for everyone else, which is the wrong default and would be
/// invisible.
fn build_readme_map(registries: &[RegistryConfig]) -> HashMap<String, HotReadmeConfig> {
    registries
        .iter()
        .map(|reg| {
            let cfg = reg.readme.as_ref().map_or_else(
                || HotReadmeConfig {
                    registry_type: reg.registry_type.clone(),
                    ..HotReadmeConfig::default()
                },
                |r| HotReadmeConfig {
                    enabled: r.enabled,
                    from_archive: r.from_archive,
                    max_bytes: r.max_bytes,
                    // `validate()` has already refused an unrecognised value, so
                    // this cannot silently become the default on a running
                    // server — the process would not have started.
                    remote_images: RemoteImagePolicy::parse(&r.remote_images).unwrap_or_default(),
                    remote_image_hosts: r.remote_image_hosts.clone(),
                    image_max_bytes: r.image_max_bytes,
                    registry_type: reg.registry_type.clone(),
                },
            );
            (reg.name.clone(), cfg)
        })
        .collect()
}

/// Populate every registry, for the same reason `build_readme_map` does: the
/// absence of a `[registries.upstream_detail]` block means **on**.
fn build_upstream_detail_map(
    registries: &[RegistryConfig],
) -> HashMap<String, HotUpstreamDetailConfig> {
    registries
        .iter()
        .map(|reg| {
            let cfg =
                reg.upstream_detail
                    .as_ref()
                    .map_or_else(HotUpstreamDetailConfig::default, |d| {
                        HotUpstreamDetailConfig {
                            enabled: d.enabled,
                            max_versions: d.max_versions,
                            negative_ttl: std::time::Duration::from_secs(d.negative_ttl_secs),
                        }
                    });
            (reg.name.clone(), cfg)
        })
        .collect()
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
        RegistryKind::JetbrainsMarketplace => "https://plugins.jetbrains.com",
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

/// Per-registry Go checksum database URLs (RFC 0009 §7.4).
///
/// Same absence-means-disabled contract as `build_vuln_db_map`, and the same
/// three cases: unset defaults to the public log, `""` disables the route, and
/// an explicit value points at a private mirror.
fn build_sumdb_map(registries: &[RegistryConfig]) -> SumDbMap {
    const DEFAULT: &str = "https://sum.golang.org";
    let urls = registries
        .iter()
        .filter(|r| r.registry_type == RegistryKind::Goproxy.as_str())
        .filter_map(|r| match r.sumdb_url.as_deref() {
            Some("") => None,
            Some(url) => Some((r.name.clone(), url.trim_end_matches('/').to_owned())),
            None => Some((r.name.clone(), DEFAULT.to_owned())),
        })
        .collect();
    SumDbMap::new(urls)
}

pub(super) fn build_hot_bundle(
    cfg: &AppConfig,
    beta_channel_store: &Arc<dyn BetaChannelPort>,
    repo: &Arc<dyn PackageRepository>,
    vuln_repo: &Arc<dyn VulnerabilityRepository>,
    sbom_repo: &Arc<dyn SbomRepository>,
) -> anyhow::Result<(
    HotConfig,
    AccessConfig,
    RegistryMap,
    RegistryModeMap,
    UpstreamMap,
    VulnDbMap,
    SumDbMap,
)> {
    let mut reg_clients: HashMap<String, Arc<dyn batlehub_core::ports::RegistryClient>> =
        HashMap::new();
    let mut reg_policies: HashMap<String, Arc<batlehub_core::services::RegistryPolicy>> =
        HashMap::new();
    let mut reg_type_map: HashMap<String, String> = HashMap::new();
    let mut reg_mode_map: HashMap<String, RegistryMode> = HashMap::new();
    let mut upstream_map: HashMap<String, String> = HashMap::new();
    let mut reg_resolution: HashMap<String, batlehub_core::entities::ResolutionPolicy> =
        HashMap::new();

    for reg in &cfg.registries {
        let client = crate::builders::build_registry_client(reg, cfg.proxy.as_ref())
            .with_context(|| format!("building registry client for '{}'", reg.name))?;
        reg_clients.insert(reg.name.clone(), client);
        let policy = crate::builders::build_policy(
            reg,
            Arc::clone(repo),
            Arc::clone(vuln_repo),
            Arc::clone(sbom_repo),
        )
        .with_context(|| format!("building policy for '{}'", reg.name))?;
        reg_policies.insert(reg.name.clone(), Arc::new(policy));
        // Built from the same `reg` in the same pass as the policy above, so a
        // registry can never end up with a rule the catalog cannot see.
        reg_resolution.insert(
            reg.name.clone(),
            crate::builders::build_resolution_policy(reg)
                .with_context(|| format!("building resolution policy for '{}'", reg.name))?,
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
        readme: build_readme_map(&cfg.registries),
        upstream_detail: build_upstream_detail_map(&cfg.registries),
        console_fetch: cfg
            .registries
            .iter()
            .map(|r| (r.name.clone(), r.console_fetch))
            .collect(),
        feature_flags: build_feature_flags_map(&cfg.registries),
        integrity: build_integrity_map(&cfg.registries),
        beta_channel: build_beta_channel_map(Arc::clone(beta_channel_store), &cfg.registries),
        retention: build_retention_map(&cfg.registries),
        resolution: reg_resolution,
        signed_downloads: cfg
            .registries
            .iter()
            .map(|r| (r.name.clone(), r.signed_downloads))
            .collect(),
        signed_url: build_signed_url_service(cfg),
        max_artifact_size_bytes: cfg.limits.max_artifact_size_bytes,
        versions_per_page: cfg.limits.versions_per_page,
        packages_per_page: cfg.limits.packages_per_page,
    };

    Ok((
        hot,
        build_access_config(cfg),
        RegistryMap::from(reg_type_map),
        RegistryModeMap::from(reg_mode_map),
        UpstreamMap::from(upstream_map),
        build_vuln_db_map(&cfg.registries),
        build_sumdb_map(&cfg.registries),
    ))
}

/// The instance signer for RFC 0012 download URLs, when one is configured.
///
/// Built here rather than once at startup so a config reload rotates the secret
/// without a restart — which is the whole point of `previous_secrets`, and it
/// would be a strange rotation story if adding the new key needed a bounce.
///
/// `AppConfig::validate()` has already rejected a secret that is absent, empty,
/// short, or paired with a registry that signs; anything reaching here is
/// well-formed, so there is no error to return.
fn build_signed_url_service(cfg: &AppConfig) -> Option<Arc<SignedUrlService>> {
    let block = cfg.server.signed_urls.as_ref()?;
    Some(Arc::new(SignedUrlService::new(
        block.secret.trim().as_bytes().to_vec(),
        block
            .active_previous_secrets()
            .into_iter()
            .map(|s| s.into_bytes())
            .collect(),
        block.ttl_seconds,
    )))
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
        // A registry reachable *only* through `[registries.rbac.groups]` — a
        // team-only registry — has all three role tiers empty. Its proxy access
        // is granted per-caller by `accessible_registries_for`, which unions the
        // group grants in, so gating the explore sets on the role tiers alone
        // left it out of every one of them: `explore_accessible_registries_for`
        // intersects proxy access with the explore set, so a team member's
        // explore set came back empty for the one registry they can pull from.
        //
        // Harmless while the set was only a listing filter (an empty vector
        // reads as "no restriction" in `ExploreFilter`); a hard `404` on the
        // detail, README and image endpoints once those refuse on the set
        // itself. `rbac.explore.*` documents itself as defaulting to "any role
        // that has proxy access", and a group member has proxy access.
        //
        // Safe to widen here because the intersection still applies: naming a
        // group-only registry in `explore_user` grants nothing to a caller whose
        // `accessible_registries_for` does not already contain it, and
        // `r.rbac.explore.*` is still honoured.
        let has_group = !r.rbac.groups.is_empty();

        if has_anonymous {
            anonymous.insert(r.name.clone());
        }
        if has_user {
            user.insert(r.name.clone());
        }
        if has_admin {
            admin.insert(r.name.clone());
        }
        if (has_anonymous || has_group) && r.rbac.explore.anonymous {
            explore_anonymous.insert(r.name.clone());
        }
        if (has_user || has_group) && r.rbac.explore.user {
            explore_user.insert(r.name.clone());
        }
        if (has_admin || has_group) && r.rbac.explore.admin {
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

/// What startup settled about `[search] text_config`, so a *reload* can be told
/// no.
///
/// `[search] readmes` is hot-reloadable and `text_config` is not — the index is
/// a generated column, and changing the configuration means dropping and
/// re-adding it, which rewrites every stored README under a lock. Two things
/// followed from that being unchecked on the reload path:
///
/// - a **typo** reloaded green. `validate()` only checks the value is non-empty,
///   nothing else on that path talks to Postgres, and the name is only resolved
///   at boot — so the server kept running and refused to start the next time it
///   was restarted, hours or weeks later;
/// - a reload that turned prose search **on** ran queries with the configured
///   name against a column built with a different one, which matches almost
///   nothing and reports it as a `200` with an empty list.
///
/// [`settle_text_config`] closes the second at its root — what the repository
/// queries with is what the column actually has — and this closes the first.
#[derive(Clone)]
pub(super) struct SettledTextConfig {
    /// Every name `pg_ts_config` listed at boot.
    known: Arc<HashSet<String>>,
    /// The configuration the FTS column is built with, i.e. the one every query
    /// uses for as long as this process runs.
    in_force: String,
}

impl SettledTextConfig {
    /// The configuration to query with — the column's own, never merely the
    /// configured one.
    pub(super) fn in_force(&self) -> &str {
        &self.in_force
    }

    /// Refuse a candidate config whose `[search] text_config` this Postgres has
    /// never heard of; warn when it names a real configuration that is not the
    /// one in force.
    ///
    /// A warning rather than a refusal for the second case: the change is
    /// legitimate, it simply cannot take effect without the rebuild a restart
    /// does, and refusing would leave an operator who set it before enabling
    /// search unable to reload *anything* until they restarted.
    fn check(&self, search: &batlehub_config::schema::SearchConfig) -> anyhow::Result<()> {
        if !self.known.contains(&search.text_config) {
            let mut known: Vec<&str> = self.known.iter().map(String::as_str).collect();
            known.sort_unstable();
            anyhow::bail!(
                "[search] text_config = '{}' is not a text search configuration on this Postgres \
                 server (known: {}); refusing the reload rather than accepting a value that would \
                 only fail on the next restart",
                search.text_config,
                known.join(", ")
            );
        }
        if search.text_config != self.in_force {
            tracing::warn!(
                configured = %search.text_config,
                in_force = %self.in_force,
                "search: [search] text_config changed, and changing it rebuilds the README \
                 full-text column — queries keep using the configuration the column was built \
                 with until this server is restarted"
            );
        }
        Ok(())
    }
}

/// Resolve `[search] text_config` against the database at startup.
///
/// Always validates the name, whether or not prose search is on: the whole point
/// is that a value which cannot work is refused when it is written rather than
/// at the next restart.
///
/// The column is only *rebuilt* when search is on — that rewrites every stored
/// README and holds a lock, which is not a thing to do for a feature that is
/// switched off. With search off, what comes back is what the column actually
/// has, so a reload that turns search on cannot end up searching `english`
/// against a `french` column.
pub(super) async fn settle_text_config(
    pool: &sqlx::PgPool,
    search: &batlehub_config::schema::SearchConfig,
) -> anyhow::Result<SettledTextConfig> {
    let known: HashSet<String> = batlehub_adapters::db::text_config_names(pool)
        .await?
        .into_iter()
        .collect();
    if !known.contains(&search.text_config) {
        let mut names: Vec<&str> = known.iter().map(String::as_str).collect();
        names.sort_unstable();
        anyhow::bail!(
            "[search] text_config = '{}' is not a text search configuration on this Postgres \
             server; known: {}",
            search.text_config,
            names.join(", ")
        );
    }

    let in_force = if search.readmes {
        batlehub_adapters::db::ensure_readme_text_config(pool, &search.text_config).await?
    } else {
        let current = batlehub_adapters::db::column_text_config(pool)
            .await?
            .unwrap_or_else(|| batlehub_adapters::db::DEFAULT_TEXT_CONFIG.to_owned());
        if current != search.text_config {
            tracing::warn!(
                configured = %search.text_config,
                in_force = %current,
                "search: [search] readmes is off, so the README full-text column was not rebuilt \
                 for the configured text_config — enabling prose search by reload will query with \
                 the configuration the column already has; restart with readmes = true to rebuild"
            );
        }
        current
    };

    Ok(SettledTextConfig {
        known: Arc::new(known),
        in_force,
    })
}

pub(super) fn make_hot_builder(
    beta_channel_store: Arc<dyn BetaChannelPort>,
    repo: Arc<dyn PackageRepository>,
    vuln_repo: Arc<dyn VulnerabilityRepository>,
    sbom_repo: Arc<dyn SbomRepository>,
    text_config: SettledTextConfig,
) -> batlehub_web::services::HotConfigBuilder {
    Arc::new(move |cfg: &AppConfig| {
        // Before anything is built: a reload carrying a text search
        // configuration this server does not have is refused here, on the same
        // path the config editor's "validate" button takes.
        text_config.check(&cfg.search)?;
        let (hot, access, rm, rmm, um, vuln_db, sumdb) =
            build_hot_bundle(cfg, &beta_channel_store, &repo, &vuln_repo, &sbom_repo)?;
        let mut cargo_map: HashMap<String, CargoIndexProxy> = HashMap::new();
        for reg in &cfg.registries {
            if reg.registry_type == RegistryKind::Cargo.as_str()
                && !matches!(reg.mode, RegistryMode::Local)
            {
                let index = crate::builders::build_cargo_index(reg, cfg.proxy.as_ref())
                    .with_context(|| format!("building cargo index client for '{}'", reg.name))?;
                cargo_map.insert(reg.name.clone(), index);
            }
        }
        let repo_signer_map = crate::builders::build_repo_signer_map(cfg)?;
        Ok(batlehub_web::services::BuiltHotState {
            hot,
            access,
            search_readmes: cfg.search.readmes,
            registry_map: rm,
            registry_mode_map: rmm,
            upstream_map: um,
            cargo_index_map: CargoIndexMap::new(cargo_map),
            repo_signer_map,
            vuln_db_map: vuln_db,
            sumdb_map: sumdb,
            registry_host_map: RegistryHostMap::from_app_config(cfg),
        })
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

    // ── [search] text_config on the reload path ──────────────────────────────

    fn settled(in_force: &str, known: &[&str]) -> SettledTextConfig {
        SettledTextConfig {
            known: Arc::new(known.iter().map(|s| (*s).to_owned()).collect()),
            in_force: in_force.to_owned(),
        }
    }

    fn search(text_config: &str) -> batlehub_config::schema::SearchConfig {
        batlehub_config::schema::SearchConfig {
            readmes: true,
            text_config: text_config.to_owned(),
        }
    }

    /// The typo case. `validate()` only checks the value is non-empty, so before
    /// this the reload was accepted, ran on with the old configuration, and the
    /// server refused to start the next time it was restarted — which is where
    /// an operator finds out, hours later, from a server that will not come back.
    #[test]
    fn a_reload_naming_an_unknown_text_configuration_is_refused() {
        let s = settled("english", &["english", "french", "simple"]);
        let err = s
            .check(&search("englsh"))
            .expect_err("an unknown configuration must be refused");
        let msg = err.to_string();
        assert!(msg.contains("englsh"), "{msg}");
        // And it says which ones exist, so the fix does not need a psql session.
        assert!(msg.contains("english, french, simple"), "{msg}");
    }

    /// A real configuration that is not the one the column was built with is a
    /// legitimate change that simply needs a restart. Refusing it would leave an
    /// operator who set it *before* enabling search unable to reload anything at
    /// all until they restarted.
    #[test]
    fn a_known_text_configuration_that_is_not_in_force_is_allowed_through() {
        let s = settled("english", &["english", "french"]);
        assert!(s.check(&search("french")).is_ok());
        assert!(s.check(&search("english")).is_ok());
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
    fn upstream_url_for_jetbrains_marketplace_default() {
        let r = make_registry("jetbrains-marketplace", "jbm-reg", "");
        assert_eq!(
            upstream_url_for(&r),
            Some("https://plugins.jetbrains.com".to_owned())
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

        // A team-only registry is browsable by its team. `group-reg` names no
        // role tier at all, so gating the explore sets on the tiers alone left
        // it out of every one of them — and once the detail, README and image
        // endpoints began refusing on `explore_accessible_registries_for`, that
        // was a `404` for the only people who can pull from it.
        assert!(access.explore_anonymous.contains("group-reg"));
        assert!(access.explore_user.contains("group-reg"));
        assert!(access.explore_admin.contains("group-reg"));
    }

    /// The end of that: a `ci-bots` member browses `group-reg`, and nobody else
    /// does. The set is still an *intersection* with proxy access, so naming a
    /// group-only registry in every explore tier grants nothing on its own.
    #[test]
    fn a_group_only_registry_is_browsable_by_its_group_and_nobody_else() {
        use batlehub_core::entities::{Identity, Role};

        let cfg = make_app_config(
            r#"
            [[registries]]
            type = "npm"
            name = "group-reg"
            [registries.rbac.groups]
            ci-bots = ["read"]
            "#,
        );
        let access = build_access_config(&cfg);

        let member = Identity {
            user_id: None,
            role: Role::User,
            auth_provider: None,
            groups: vec!["ci-bots".to_owned()],
        };
        let outsider = Identity {
            user_id: None,
            role: Role::Admin,
            auth_provider: None,
            groups: vec![],
        };

        assert!(access
            .explore_accessible_registries_for(&member)
            .contains("group-reg"));
        assert!(access
            .explore_accessible_registries_for(&outsider)
            .is_empty());
    }
}
