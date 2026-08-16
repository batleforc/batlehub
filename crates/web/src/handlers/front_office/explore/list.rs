use std::collections::HashMap;

use batlehub_core::entities::{resolve_state, ResolutionPolicy};
use batlehub_core::services::LocalRegistryService;
use chrono::Utc;

use super::{
    default_per_page, format_dt, get, web, AdminService, AppError, Arc, AuthIdentity, Deserialize,
    ExploreFilter, ExploreSortBy, IntoParams, PackageSource, Responder, Serialize, ToSchema,
};

// ── List packages (collapsed) ─────────────────────────────────────────────────

#[derive(Deserialize, IntoParams)]
pub struct ExploreQuery {
    pub registry: Option<String>,
    pub name: Option<String>,
    /// Sort order: `downloads` (default), `name`, `recent`, or `fetched`.
    ///
    /// `recent` is when a client last downloaded a package *from* this
    /// instance; `fetched` is when this instance last pulled one *from
    /// upstream*. The catalog is ordered on the latter.
    #[serde(default)]
    pub sort: Option<String>,
    #[serde(default)]
    pub page: u64,
    #[serde(default = "default_per_page")]
    pub per_page: u64,
}

#[derive(Serialize, ToSchema)]
pub struct ExplorePackageListResponse {
    pub items: Vec<ExploreEntryDto>,
    pub total: usize,
    pub page: u64,
    pub per_page: u64,
    /// `true` when the upstream database was unreachable and no cached data was available.
    /// The result set will be empty; the UI should surface a warning to the user.
    pub upstream_unavailable: bool,
}

#[derive(Serialize, ToSchema)]
pub struct ExploreEntryDto {
    pub registry: String,
    pub name: String,
    pub version_count: u64,
    pub total_downloads: u64,
    /// When a client last downloaded this package from us (RFC 3339).
    pub last_accessed: Option<String>,
    /// `"proxied"` | `"local"` | `"both"`
    pub source: String,
    /// An administrator has blocked at least one version.
    pub has_blocked: bool,
    /// An owner has yanked at least one locally published version.
    ///
    /// Previously reported inside `has_blocked`, which conflated an operator's
    /// refusal with an owner's withdrawal.
    pub has_yanked: bool,
    /// This package's standing here, in DESIGN.md's resolution vocabulary:
    /// `"cached"` | `"stale"` | `"held"` | `"pending"` | `"yanked"` |
    /// `"blocked"`.
    ///
    /// Derived server-side rather than in the client because two of the six
    /// need configuration the browser has no business holding — the registry's
    /// artifact TTL, and the release-age gate's window and bypass roles — and
    /// because `"held"` depends on *who is asking*.
    pub state: String,
    /// How many versions this instance currently holds bytes for.
    pub cached_versions: u64,
    /// Bytes held across those versions. `null` means unknown, not zero: sizes
    /// were not recorded for artifacts cached before migration 004.
    pub cached_bytes: Option<u64>,
    /// When this instance last fetched any version from upstream (RFC 3339).
    pub last_fetched_at: Option<String>,
    /// The version most recently obtained, by `cached_at` for a proxied
    /// artifact or `published_at` for a local one. `null` when nothing is held.
    pub newest_version: Option<String>,
}

/// Explore available packages (one entry per unique package name).
///
/// Returns packages from both the proxy cache and locally published packages,
/// collapsed to one entry per registry+name combination.
/// Only registries the caller is allowed to explore are included.
#[utoipa::path(
    get,
    path = "/api/v1/explore/packages",
    tag = "explore",
    params(ExploreQuery),
    responses(
        (status = 200, description = "Package explorer listing", body = ExplorePackageListResponse),
    ),
    security(("bearer_token" = [])),
)]
#[get("/api/v1/explore/packages")]
pub async fn explore_packages(
    query: web::Query<ExploreQuery>,
    identity: AuthIdentity,
    admin_svc: web::Data<Arc<AdminService>>,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    access: web::Data<crate::AccessConfigLock>,
) -> Result<impl Responder, AppError> {
    let accessible = access
        .read()
        .await
        .explore_accessible_registries_for(&identity);

    if let Some(ref reg) = query.registry {
        if !accessible.contains(reg) {
            return Ok(web::Json(ExplorePackageListResponse {
                items: vec![],
                total: 0,
                page: query.page.min(10_000),
                per_page: query.per_page.clamp(1, 100),
                upstream_unavailable: false,
            }));
        }
    }

    let registries: Vec<String> = if query.registry.is_none() {
        accessible.into_iter().collect()
    } else {
        vec![]
    };

    let sort_by = match query.sort.as_deref() {
        Some("name") => ExploreSortBy::Name,
        Some("recent") => ExploreSortBy::Recent,
        Some("fetched") => ExploreSortBy::Fetched,
        _ => ExploreSortBy::Downloads,
    };

    // Snapshot the per-registry resolution inputs before any `await`, per the
    // hot-reload convention: clone out of the lock and let a config swap
    // mid-request finish against the values this request started with. The map
    // is a handful of small structs, one per configured registry.
    let resolution: HashMap<String, ResolutionPolicy> =
        local_svc.hot.read().await.resolution.clone();

    // See `clamp_pagination`'s doc comment for why page/per_page are clamped;
    // an unclamped per_page=0 here would also collapse `filter`'s cache key
    // onto `count_filter`'s (both would be limit=0,offset=0).
    let (page, per_page) = crate::handlers::clamp_pagination(query.page, query.per_page);

    // Registry-level access (above) is the coarse gate; this is the per-package
    // one. Without it an `internal`/`team` package's name and version count are
    // listed to anyone who can explore the registry, even though the same caller
    // gets a 403 trying to download it.
    let viewer = crate::handlers::explore_viewer_for(&identity);

    let filter = ExploreFilter {
        registry: query.registry.clone(),
        registries: registries.clone(),
        name_contains: query.name.clone(),
        sort_by: sort_by.clone(),
        limit: per_page,
        offset: page * per_page,
        viewer: viewer.clone(),
    };
    let count_filter = ExploreFilter {
        registry: query.registry.clone(),
        registries,
        name_contains: query.name.clone(),
        sort_by,
        limit: 0,
        offset: 0,
        viewer,
    };

    let ((packages, pkg_unavailable), (total, count_unavailable)) = tokio::try_join!(
        admin_svc.explore_packages(filter),
        admin_svc.count_explore_packages(count_filter),
    )
    .map_err(AppError::from)?;

    // One instant for the whole page, not one per row: `Utc::now()` inside the
    // loop can straddle a TTL or a quarantine boundary, so two packages fetched
    // in the same second could be graded against different "now"s.
    let now = Utc::now();
    let items: Vec<ExploreEntryDto> = packages
        .into_iter()
        .map(|e| {
            // A registry with no entry gets the default policy: no TTL, no age
            // gate. That is the same answer as a registry configured with
            // neither, which is what an unknown registry effectively is here.
            let policy = resolution.get(&e.registry).cloned().unwrap_or_default();
            let state = resolve_state(&e, &policy, &identity.0.role, now);
            ExploreEntryDto {
                registry: e.registry,
                name: e.name,
                version_count: e.version_count,
                total_downloads: e.total_downloads,
                last_accessed: e.last_accessed.map(format_dt),
                source: match e.source {
                    PackageSource::Proxied => "proxied".to_string(),
                    PackageSource::Local => "local".to_string(),
                    PackageSource::Both => "both".to_string(),
                },
                has_blocked: e.has_blocked,
                has_yanked: e.has_yanked,
                state: state.as_str().to_string(),
                cached_versions: e.cached_versions,
                cached_bytes: e.cached_bytes,
                last_fetched_at: e.last_fetched_at.map(format_dt),
                newest_version: e.newest_version,
            }
        })
        .collect();

    Ok(web::Json(ExplorePackageListResponse {
        total: total as usize,
        items,
        page,
        per_page,
        upstream_unavailable: pkg_unavailable || count_unavailable,
    }))
}
