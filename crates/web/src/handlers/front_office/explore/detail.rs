use super::{
    format_dt, get, web, AdminService, AppError, Arc, AuthIdentity, Deserialize, IntoParams,
    LocalRegistryService, PackageFilter, PackageStatus, Responder, Serialize, ToSchema,
};
use batlehub_config::schema::RegistryMode;
use batlehub_core::{
    entities::{absent_readme_state_for, ReadmeState, RegistryKind},
    services::{proxy::Freshness, ProxyService, SbomService},
};

use crate::RegistryModeMap;

use crate::badges::socket_badge_url;
use crate::handlers::back_office::packages::detail::VulnerabilityDto;
use crate::RegistryMap;

/// The recorded licence for one version, or `None` when it is not known.
///
/// A lookup failure reads as unknown rather than propagating: the licence is
/// one field on a page whose job is showing versions, and an SBOM outage should
/// not turn the package page into an error. `LicenseGateRule` is where a failed
/// lookup carries weight, and it logs its own.
async fn license_for(
    sbom_svc: &Option<web::Data<Arc<SbomService>>>,
    registry: &str,
    name: &str,
    version: &str,
) -> Option<String> {
    let svc = sbom_svc.as_ref()?;
    svc.repo
        .get_license_for_coordinate(registry, name, version)
        .await
        .unwrap_or(None)
}

// ── Package detail ─────────────────────────────────────────────────────────────

#[derive(Deserialize, IntoParams)]
pub struct PackageDetailPath {
    pub registry: String,
    pub name: String,
}

/// Whether the discovery read may run for this request.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum UpstreamMode {
    /// Ask upstream when this instance holds nothing of the package and the
    /// registry allows it. The default, and what the package page uses.
    #[default]
    Auto,
    /// Answer from local rows only. For callers that want the cheap answer —
    /// the admin panels that only care about held versions — and for anyone who
    /// wants exactly the shape this endpoint had before RFC 0007.
    Skip,
}

#[derive(Deserialize, IntoParams)]
pub struct PackageDetailQuery {
    #[serde(default)]
    #[param(inline)]
    pub upstream: UpstreamMode,
}

/// What the discovery read did, so the page can say which rung answered rather
/// than presenting a degraded answer as a complete one.
#[derive(Serialize, ToSchema)]
pub struct UpstreamReadDto {
    /// `false` for a `local`-mode registry, a kind with no upstream to ask, a
    /// `?upstream=skip` caller, a registry with the read disabled, or a package
    /// published here — a private name is never sent to a public index.
    pub attempted: bool,
    /// `cached` | `fresh` | `stale`, when the read answered.
    pub freshness: Option<String>,
    /// How many upstream-only versions came back.
    pub version_count: usize,
    /// `max_versions` shortened the list. A silently shortened list is a lie
    /// about the registry.
    pub truncated: bool,
    /// Set when the read was attempted and every rung failed. The page says the
    /// upstream could not be reached rather than showing an empty table as an
    /// answer.
    pub error: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct ExplorePackageDetailResponse {
    pub registry: String,
    pub name: String,
    pub gate: GateDto,
    pub versions: Vec<ExploreVersionDto>,
    /// `true` when the upstream database was unreachable and this package has no cached data.
    pub upstream_unavailable: bool,
    /// What the discovery read did (RFC 0007 §4.2).
    pub upstream: UpstreamReadDto,
}

#[derive(Serialize, ToSchema)]
pub struct GateDto {
    /// Whether the caller's role can access this registry through the proxy.
    pub registry_accessible: bool,
    /// Whether the caller is a beta-channel member for this registry.
    pub beta_member: bool,
}

#[derive(Serialize, ToSchema)]
pub struct ExploreVersionDto {
    pub version: String,
    /// `"proxied"` | `"local"` | `"upstream"`
    ///
    /// `upstream` means this instance holds no bytes for the version and knows
    /// about it only because it asked. Every cell that would be a fact about
    /// what we hold is `null` on such a row rather than `0`.
    pub source: String,
    pub firewall: FirewallDto,
    /// `null` on an upstream-only row — **not** `0`.
    ///
    /// `0` would be a definite answer with nothing behind it: nobody has
    /// downloaded it *through here*, which is not the same as nobody having
    /// downloaded it, and the row exists precisely because this instance has
    /// never held the version (RFC 0007 §4.2).
    pub download_count: Option<u64>,
    pub last_accessed: Option<String>,
    pub published_at: Option<String>,
    pub is_prerelease: bool,
    /// Known vulnerabilities for this version (from the periodic SBOM re-scan).
    pub vulnerabilities: Vec<VulnerabilityDto>,
    /// The licence this version's own manifest declared, verbatim.
    ///
    /// Null means *unknown*, never "unlicensed": it is read out of the archive
    /// when the version is cached or published, so it is absent for anything
    /// fetched before extraction existed and for the registry types with no
    /// manifest parser (RFC 0004-bis §13.1).
    pub license: Option<String>,
    /// socket.dev badge URL when enabled for this registry; else null.
    pub socket_badge_url: Option<String>,
    /// Flagged as deprecated (still downloadable). Local versions only.
    pub deprecated: bool,
    /// Optional deprecation message.
    pub deprecation_message: Option<String>,
    /// Hidden from registry-protocol listings but still downloadable by exact
    /// coordinate. Local versions only.
    pub unlisted: bool,
    /// `available` | `none` | `unknown` — whether this version has a README.
    ///
    /// A tri-state rather than a boolean, because the answer genuinely has three
    /// values and the third is the common one for an archive-borne registry
    /// kind: a version this instance holds no bytes for has a README that cannot
    /// be read yet. `false` rendered for *"we have not looked"* would be a
    /// definite-looking answer with nothing behind it (RFC 0007 §4.2).
    pub readme: ReadmeState,
    /// Whether this version has ever been scanned for vulnerabilities.
    ///
    /// `vulnerabilities: []` means *scanned and clear* only when this is `true`.
    /// On a version nothing has ever opened it means *never scanned*, and the
    /// two must not render identically — a green row on a package this instance
    /// has never held is a claim we cannot support.
    pub vulnerabilities_scanned: bool,
}

#[derive(Serialize, ToSchema)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum FirewallDto {
    Clear,
    Blocked {
        reason: String,
        blocked_by: String,
        blocked_at: String,
    },
    Yanked,
}

/// Package detail view: all known versions with gate and firewall status.
#[utoipa::path(
    get,
    path = "/api/v1/explore/packages/{registry}/{name}",
    tag = "explore",
    params(PackageDetailPath, PackageDetailQuery),
    responses(
        (status = 200, description = "Package detail", body = ExplorePackageDetailResponse),
    ),
    security(("bearer_token" = [])),
)]
// Eight extractors, one per thing the page needs to answer for: the coordinate,
// the caller, the catalogue, the local backend, the registry's type, the access
// policy, the recorded licences and the README store. Actix injects each by
// type, so collapsing them into a bag would hide what the handler reads.
#[allow(clippy::too_many_arguments)]
#[get("/api/v1/explore/packages/{registry}/{name}")]
pub async fn explore_package_detail(
    path: web::Path<PackageDetailPath>,
    query: web::Query<PackageDetailQuery>,
    identity: AuthIdentity,
    admin_svc: web::Data<Arc<AdminService>>,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    registry_map: web::Data<RegistryMap>,
    access: web::Data<crate::AccessConfigLock>,
    sbom_svc: Option<web::Data<Arc<SbomService>>>,
    proxy_svc: web::Data<Arc<ProxyService>>,
    mode_map: web::Data<RegistryModeMap>,
) -> Result<impl Responder, AppError> {
    let registry = &path.registry;
    let name = &path.name;

    // socket.dev badge: enabled per registry via feature flag, mapped by type.
    let socket_badge_enabled = local_svc
        .hot
        .read()
        .await
        .feature_flags
        .get(registry)
        .is_none_or(|f| f.socket_badge);
    let registry_type = registry_map.type_of(registry);
    let badge_for = |version: &str| -> Option<String> {
        if !socket_badge_enabled {
            return None;
        }
        registry_type
            .as_deref()
            .and_then(|t| socket_badge_url(t, name, version))
    };

    // Gate: registry-level proxy access
    let registry_accessible = access
        .read()
        .await
        .accessible_registries_for(&identity)
        .contains(registry);

    // Gate: per-package visibility. The listing filters `internal`/`team`
    // packages out entirely, so the detail view has to agree — otherwise the
    // name is hidden from the index while remaining readable to anyone who
    // guesses or is told the URL, and the filter buys nothing.
    //
    // 404 rather than 403 on purpose: a 403 confirms the package exists, which
    // is the fact a non-public package is trying not to disclose. Denied and
    // absent look identical from outside.
    if let Err(e) = local_svc.check_visibility(registry, name, &identity).await {
        tracing::debug!(
            registry = %registry, package = %name, error = %e,
            "explore detail: hidden by package visibility"
        );
        return Err(AppError::not_found(format!(
            "package '{name}' not found in registry '{registry}'"
        )));
    }

    // Gate: beta channel membership
    let beta_member = {
        let beta_port = local_svc
            .hot
            .read()
            .await
            .beta_channel
            .get(registry)
            .cloned();
        if let Some(bp) = beta_port {
            bp.is_member(registry, &identity).await.unwrap_or(false)
        } else {
            false
        }
    };

    // Proxied versions from package_statuses
    let proxied_filter = PackageFilter {
        registry: Some(registry.clone()),
        registries: vec![],
        name_exact: Some(name.clone()),
        name_contains: None,
        blocked_only: false,
        limit: 500,
        offset: 0,
    };
    let (proxied_summaries, upstream_unavailable) =
        match admin_svc.list_packages(proxied_filter).await {
            Ok(summaries) => (summaries, false),
            Err(_) => (vec![], true),
        };

    // Local versions from local_packages
    let local_versions = local_svc
        .backend
        .get_versions(registry, name)
        .await
        .unwrap_or_default();

    // Which versions have a stored README, in **one** query rather than a probe
    // per row: the table can be hundreds of versions long and a lookup each
    // would be that many round trips for one page load.
    let readme_versions: std::collections::HashSet<String> = match proxy_svc.readme.as_ref() {
        Some(svc) => svc
            .repo
            .list_versions_with_readme(registry, name)
            .await
            .unwrap_or_default()
            .into_iter()
            .collect(),
        None => Default::default(),
    };
    let kind = registry_type
        .as_deref()
        .and_then(|t| t.parse::<RegistryKind>().ok());
    // A version we hold and that has no stored README is *unknown* rather than
    // *none* when the kind reads its README out of the archive: the text exists,
    // it just has not been read yet — for a `from_archive = false` registry, or
    // for a version cached before this feature existed.
    let absent_state = absent_readme_state_for(kind);
    let readme_state = |version: &str| -> ReadmeState {
        if readme_versions.contains(version) {
            ReadmeState::Available
        } else {
            absent_state
        }
    };

    // Whether this package is *published here*, which is what suppresses the
    // discovery read — not whether we happen to hold some of its versions.
    // Holding three versions out of forty is exactly the case where the missing
    // rows are worth showing; the suppression is about provenance (§4.2).
    let published_locally = !local_versions.is_empty();

    // Blocked versions, so an upstream-only row shows `Blocked` with its reason
    // rather than as installable. Read once for the whole merge.
    let blocked_versions: std::collections::HashMap<String, (String, String, String)> = admin_svc
        .list_packages(PackageFilter {
            registry: Some(registry.clone()),
            registries: vec![],
            name_exact: Some(name.clone()),
            name_contains: None,
            blocked_only: true,
            limit: 500,
            offset: 0,
        })
        .await
        .unwrap_or_default()
        .into_iter()
        .filter_map(|summary| match summary.status {
            PackageStatus::Blocked {
                reason,
                blocked_by,
                blocked_at,
            } => Some((
                summary.package_id.version,
                (reason, blocked_by, blocked_at.to_rfc3339()),
            )),
            PackageStatus::Available => None,
        })
        .collect();

    // Build version entries
    let mut versions: Vec<ExploreVersionDto> = Vec::new();

    // Track which versions came from local to avoid duplicating proxied entries
    let local_version_set: std::collections::HashSet<&str> =
        local_versions.iter().map(|v| v.version.as_str()).collect();

    for summary in proxied_summaries {
        // Skip versions also present in local (they'll appear as "local")
        if local_version_set.contains(summary.package_id.version.as_str()) {
            continue;
        }
        let firewall = match summary.status {
            PackageStatus::Available => FirewallDto::Clear,
            PackageStatus::Blocked {
                reason,
                blocked_by,
                blocked_at,
            } => FirewallDto::Blocked {
                reason,
                blocked_by,
                blocked_at: blocked_at.to_rfc3339(),
            },
        };
        let is_prerelease = summary.package_id.version.contains('-');
        let vulnerabilities = admin_svc
            .list_vulnerabilities(registry, name, &summary.package_id.version)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(VulnerabilityDto::from)
            .collect();
        let socket_badge_url = badge_for(&summary.package_id.version);
        let license = license_for(&sbom_svc, registry, name, &summary.package_id.version).await;
        let readme = readme_state(&summary.package_id.version);
        versions.push(ExploreVersionDto {
            readme,
            // Every version in this list is one this instance has pulled
            // through, so the scanner has had the bytes and the empty list means
            // "clear" rather than "never looked".
            vulnerabilities_scanned: true,
            version: summary.package_id.version,
            source: "proxied".to_string(),
            firewall,
            download_count: Some(summary.access_count),
            last_accessed: summary.last_accessed.map(format_dt),
            published_at: None,
            is_prerelease,
            vulnerabilities,
            license,
            socket_badge_url,
            deprecated: false,
            deprecation_message: None,
            unlisted: false,
        });
    }

    for pkg in local_versions {
        let firewall = if pkg.yanked {
            FirewallDto::Yanked
        } else {
            FirewallDto::Clear
        };
        let is_prerelease = pkg.version.contains('-');
        let vulnerabilities = admin_svc
            .list_vulnerabilities(registry, name, &pkg.version)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(VulnerabilityDto::from)
            .collect();
        let socket_badge_url = badge_for(&pkg.version);
        let license = license_for(&sbom_svc, registry, name, &pkg.version).await;
        let readme = readme_state(&pkg.version);
        versions.push(ExploreVersionDto {
            readme,
            vulnerabilities_scanned: true,
            version: pkg.version,
            source: "local".to_string(),
            firewall,
            download_count: Some(0),
            last_accessed: None,
            published_at: Some(pkg.published_at.to_rfc3339()),
            is_prerelease,
            vulnerabilities,
            license,
            socket_badge_url,
            deprecated: pkg.deprecated,
            deprecation_message: pkg.deprecation_message,
            unlisted: pkg.unlisted,
        });
    }

    // ── The discovery read ────────────────────────────────────────────────
    //
    // Everything above is exactly the code this endpoint had before RFC 0007,
    // and the merge below only *adds* rows — so a bug in the new path cannot
    // change what the page says about a version this instance holds (§6.4).
    let upstream = discovery_read(
        &proxy_svc,
        &mode_map,
        registry,
        name,
        &identity,
        query.upstream,
        // A package the local backend hosts is never asked about upstream, on
        // any mode: on a hybrid registry a private package shares a namespace
        // with a public index, and sending its name there on every page view
        // would leak the existence of internal software to a third party — the
        // same disclosure `sumdb_url = ""` exists for. It would also invite a
        // dependency-confusion answer, where the page shows upstream's versions
        // of a name that means something else here (§4.4, §7.7).
        published_locally,
    )
    .await;

    if let Some(detail) = upstream.detail.as_ref() {
        // Local rows win every collision: a version we hold is described by what
        // we know about it, and the upstream document cannot overwrite any of
        // it. The merge only adds rows the local sources did not have — the same
        // precedence `SearchMode::Hybrid` already uses, and for the same reason.
        let held: std::collections::HashSet<&str> =
            versions.iter().map(|v| v.version.as_str()).collect();
        let mut extra: Vec<ExploreVersionDto> = Vec::new();
        for candidate in &detail.versions {
            if held.contains(candidate.version.as_str()) {
                continue;
            }
            // Rules are evaluated for *display*, never for permission: the
            // blocked set is consulted so an administrator's block shows as
            // `Blocked` with its reason rather than as installable. The gate
            // that matters still runs on the download.
            let firewall = match blocked_versions.get(candidate.version.as_str()) {
                Some((reason, by, at)) => FirewallDto::Blocked {
                    reason: reason.clone(),
                    blocked_by: by.clone(),
                    blocked_at: at.clone(),
                },
                None if candidate.yanked => FirewallDto::Yanked,
                None => FirewallDto::Clear,
            };
            extra.push(ExploreVersionDto {
                readme: if detail.readmes.contains_key(&candidate.version) {
                    ReadmeState::Available
                } else {
                    absent_state
                },
                // Nothing has ever opened these bytes, so an empty
                // vulnerability list here means *never scanned* — and a green
                // row on a package this instance has never held is a claim we
                // cannot support.
                vulnerabilities_scanned: false,
                version: candidate.version.clone(),
                source: "upstream".to_owned(),
                firewall,
                download_count: None,
                last_accessed: None,
                published_at: candidate.published_at.map(|t| t.to_rfc3339()),
                is_prerelease: candidate.is_prerelease,
                vulnerabilities: Vec::new(),
                license: None,
                socket_badge_url: badge_for(&candidate.version),
                deprecated: candidate.deprecated.is_some(),
                deprecation_message: candidate.deprecated.clone(),
                unlisted: false,
            });
        }
        versions.extend(extra);
    }

    // Sort: stable versions first, then pre-release; within each group newest first
    versions.sort_by(|a, b| {
        b.is_prerelease
            .cmp(&a.is_prerelease)
            .then(b.version.cmp(&a.version))
    });

    Ok(web::Json(ExplorePackageDetailResponse {
        registry: registry.clone(),
        name: name.clone(),
        gate: GateDto {
            registry_accessible,
            beta_member,
        },
        versions,
        upstream_unavailable,
        upstream: upstream.dto,
    }))
}

/// The discovery read, and everything that decides not to do it.
struct DiscoveryResult {
    detail: Option<batlehub_core::services::UpstreamDetail>,
    dto: UpstreamReadDto,
}

async fn discovery_read(
    proxy_svc: &ProxyService,
    mode_map: &RegistryModeMap,
    registry: &str,
    name: &str,
    identity: &AuthIdentity,
    mode: UpstreamMode,
    published_locally: bool,
) -> DiscoveryResult {
    let not_attempted = |error: Option<String>| DiscoveryResult {
        detail: None,
        dto: UpstreamReadDto {
            attempted: false,
            freshness: None,
            version_count: 0,
            truncated: false,
            error,
        },
    };

    if mode == UpstreamMode::Skip || published_locally {
        return not_attempted(None);
    }
    // A `local`-mode registry has no upstream: there is nothing to ask, and
    // asking would be a request to a URL that is not configured.
    if mode_map.get(registry) == RegistryMode::Local {
        return not_attempted(None);
    }

    match proxy_svc.upstream_detail(registry, name, &identity.0).await {
        Ok(Some(outcome)) => DiscoveryResult {
            dto: UpstreamReadDto {
                attempted: true,
                freshness: Some(freshness_word(outcome.freshness).to_owned()),
                version_count: outcome.detail.versions.len(),
                truncated: outcome.truncated,
                error: None,
            },
            detail: Some(outcome.detail),
        },
        // Not attempted: the registry has it off, the kind cannot be asked, or
        // upstream is already known not to have this package. None of those is
        // an error, and reporting one would put a banner on a page that is
        // simply complete.
        Ok(None) => not_attempted(None),
        // Attempted and every rung failed. The page answers from local rows and
        // says so — rung 3 never degrades to an empty page presented as an
        // answer, which is what the old "no versions yet" did.
        Err(e) => {
            tracing::debug!(
                registry = %registry, package = %name, error = %e,
                "explore detail: the discovery read failed; answering from local rows"
            );
            DiscoveryResult {
                detail: None,
                dto: UpstreamReadDto {
                    attempted: true,
                    freshness: None,
                    version_count: 0,
                    truncated: false,
                    error: Some(e.to_string()),
                },
            }
        }
    }
}

/// The vocabulary `Freshness::header_value` already uses, so the page says the
/// same word the protocol paths do.
fn freshness_word(freshness: Freshness) -> &'static str {
    match freshness {
        Freshness::Cached => "cached",
        Freshness::Fresh => "fresh",
        Freshness::Stale => "stale",
    }
}
