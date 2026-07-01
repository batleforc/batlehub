use super::{
    default_per_page, get, web, AdminService, AppError, Arc, AuthIdentity, Deserialize,
    ExploreFilter, ExploreSortBy, IntoParams, PackageSource, Responder, Serialize, ToSchema,
};

// ── List packages (collapsed) ─────────────────────────────────────────────────

#[derive(Deserialize, IntoParams)]
pub struct ExploreQuery {
    pub registry: Option<String>,
    pub name: Option<String>,
    /// Sort order: `downloads` (default), `name`, or `recent`.
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
    pub last_accessed: Option<String>,
    /// `"proxied"` | `"local"` | `"both"`
    pub source: String,
    pub has_blocked: bool,
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
        _ => ExploreSortBy::Downloads,
    };

    // Clamp caller-supplied pagination: `per_page=0` would make `LIMIT 0` return
    // zero rows while the count query still reports a nonzero total, and would
    // also collapse `filter`'s cache key onto `count_filter`'s (both would be
    // limit=0,offset=0). `page` is capped to keep `page * per_page` from
    // overflowing `u64`.
    let per_page = query.per_page.clamp(1, 100);
    let page = query.page.min(10_000);

    let filter = ExploreFilter {
        registry: query.registry.clone(),
        registries: registries.clone(),
        name_contains: query.name.clone(),
        sort_by: sort_by.clone(),
        limit: per_page,
        offset: page * per_page,
    };
    let count_filter = ExploreFilter {
        registry: query.registry.clone(),
        registries,
        name_contains: query.name.clone(),
        sort_by,
        limit: 0,
        offset: 0,
    };

    let ((packages, pkg_unavailable), (total, count_unavailable)) = tokio::try_join!(
        admin_svc.explore_packages(filter),
        admin_svc.count_explore_packages(count_filter),
    )
    .map_err(AppError::from)?;

    let items: Vec<ExploreEntryDto> = packages
        .into_iter()
        .map(|e| ExploreEntryDto {
            registry: e.registry,
            name: e.name,
            version_count: e.version_count,
            total_downloads: e.total_downloads,
            last_accessed: e.last_accessed.map(|t| t.to_rfc3339()),
            source: match e.source {
                PackageSource::Proxied => "proxied".to_string(),
                PackageSource::Local => "local".to_string(),
                PackageSource::Both => "both".to_string(),
            },
            has_blocked: e.has_blocked,
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
