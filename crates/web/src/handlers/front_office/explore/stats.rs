use super::{
    get, web, AdminService, AppError, Arc, AuthIdentity, Deserialize, ExploreFilter, ExploreSortBy,
    IntoParams, ProxyService, Responder, Serialize, ToSchema,
};

// ── Registry stats ─────────────────────────────────────────────────────────────

#[derive(Serialize, ToSchema)]
pub struct RegistryStatDto {
    pub registry: String,
    pub package_count: u64,
    pub total_downloads: u64,
}

#[derive(Serialize, ToSchema)]
pub struct ExploreRegistryStatsResponse {
    pub registries: Vec<RegistryStatDto>,
    /// `true` when the upstream database was unreachable and no cached data was available.
    pub upstream_unavailable: bool,
}

/// Per-registry package counts and download totals for the explorer sidebar.
#[utoipa::path(
    get,
    path = "/api/v1/explore/registries",
    tag = "explore",
    responses(
        (status = 200, description = "Registry statistics", body = ExploreRegistryStatsResponse),
    ),
    security(("bearer_token" = [])),
)]
#[get("/api/v1/explore/registries")]
pub async fn explore_registry_stats(
    identity: AuthIdentity,
    admin_svc: web::Data<Arc<AdminService>>,
    access: web::Data<crate::AccessConfigLock>,
) -> Result<impl Responder, AppError> {
    let accessible: Vec<String> = access
        .read()
        .await
        .explore_accessible_registries_for(&identity)
        .into_iter()
        .collect();

    let (stats, upstream_unavailable) = admin_svc
        .registry_explore_stats(&accessible)
        .await
        .map_err(AppError::from)?;

    let registries: Vec<RegistryStatDto> = stats
        .into_iter()
        .map(|s| RegistryStatDto {
            registry: s.registry,
            package_count: s.package_count,
            total_downloads: s.total_downloads,
        })
        .collect();

    Ok(web::Json(ExploreRegistryStatsResponse {
        registries,
        upstream_unavailable,
    }))
}

// ── Upstream package search ────────────────────────────────────────────────────

#[derive(Deserialize, IntoParams)]
pub struct UpstreamSearchQuery {
    pub name: String,
    /// Specific registry to search. When absent, searches all accessible registries.
    pub registry: Option<String>,
    #[serde(default = "default_upstream_limit")]
    pub limit: usize,
}

fn default_upstream_limit() -> usize {
    10
}

#[derive(Serialize, ToSchema)]
pub struct UpstreamSearchResponse {
    pub items: Vec<UpstreamPackageDto>,
}

#[derive(Serialize, ToSchema)]
pub struct UpstreamPackageDto {
    pub registry: String,
    pub name: String,
    pub latest_version: String,
    pub description: Option<String>,
    /// `true` when this package already exists in the proxy cache or local registry.
    pub already_cached: bool,
}

/// Search upstream registries for packages not yet in the proxy.
///
/// Queries each accessible registry's upstream search API. Results include a
/// `already_cached` flag so the UI can distinguish new discoveries from known packages.
#[utoipa::path(
    get,
    path = "/api/v1/explore/upstream",
    tag = "explore",
    params(UpstreamSearchQuery),
    responses(
        (status = 200, description = "Upstream search results", body = UpstreamSearchResponse),
    ),
    security(("bearer_token" = [])),
)]
#[get("/api/v1/explore/upstream")]
pub async fn explore_upstream_search(
    query: web::Query<UpstreamSearchQuery>,
    identity: AuthIdentity,
    proxy_svc: web::Data<Arc<ProxyService>>,
    admin_svc: web::Data<Arc<AdminService>>,
    access: web::Data<crate::AccessConfigLock>,
) -> Result<impl Responder, AppError> {
    let accessible = access
        .read()
        .await
        .explore_accessible_registries_for(&identity);

    tracing::info!(
        name = %query.name,
        accessible_registries = ?accessible,
        "upstream search: resolving clients"
    );

    // Collect registry clients to search (snapshot from hot config)
    let clients_to_search: Vec<(String, Arc<dyn batlehub_core::ports::RegistryClient>)> = {
        let hot = proxy_svc.hot.read().await;
        hot.registries
            .iter()
            .filter(|(name, _)| {
                if let Some(ref reg) = query.registry {
                    name.as_str() == reg && accessible.contains(name.as_str())
                } else {
                    accessible.contains(name.as_str())
                }
            })
            .map(|(name, client)| (name.clone(), Arc::clone(client)))
            .collect()
    };

    tracing::info!(
        clients = ?clients_to_search.iter().map(|(n, _)| n).collect::<Vec<_>>(),
        "upstream search: clients selected"
    );

    // Collect already-known names so we can set the already_cached flag
    let known_filter = ExploreFilter {
        registry: query.registry.clone(),
        registries: if query.registry.is_none() {
            accessible.into_iter().collect()
        } else {
            vec![]
        },
        name_contains: Some(query.name.clone()),
        sort_by: ExploreSortBy::Name,
        limit: 500,
        offset: 0,
    };
    let (known, _) = admin_svc
        .explore_packages(known_filter)
        .await
        .map_err(AppError::from)?;
    let known_set: std::collections::HashSet<(String, String)> = known
        .iter()
        .map(|e| (e.registry.clone(), e.name.clone()))
        .collect();

    // Fan out search across all matching registry clients concurrently
    let search_futures: Vec<_> = clients_to_search
        .iter()
        .map(|(reg_name, client)| {
            let reg = reg_name.clone();
            let q = query.name.clone();
            let lim = query.limit;
            let client = Arc::clone(client);
            async move {
                let results = match client.search_packages(&q, lim).await {
                    Ok(r) => {
                        tracing::info!(registry = %reg, count = r.len(), "upstream search: got results");
                        r
                    }
                    Err(e) => {
                        tracing::warn!(registry = %reg, error = %e, "upstream search: client error");
                        vec![]
                    }
                };
                results
                    .into_iter()
                    .map(move |p| (reg.clone(), p))
                    .collect::<Vec<_>>()
            }
        })
        .collect();

    let results: Vec<_> = futures::future::join_all(search_futures)
        .await
        .into_iter()
        .flatten()
        .map(|(registry, pkg)| {
            let already_cached = known_set.contains(&(registry.clone(), pkg.name.clone()));
            UpstreamPackageDto {
                registry,
                name: pkg.name,
                latest_version: pkg.latest_version,
                description: pkg.description,
                already_cached,
            }
        })
        .collect();

    Ok(web::Json(UpstreamSearchResponse { items: results }))
}
