use std::sync::Arc;

use actix_web::{get, web, Responder};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use batlehub_core::{
    entities::{ExploreFilter, ExploreSortBy, PackageFilter, PackageSource, PackageStatus},
    services::{AdminService, LocalRegistryService, ProxyService},
};

use crate::{error::AppError, extractors::AuthIdentity, AccessConfig};

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

fn default_per_page() -> u64 {
    20
}

#[derive(Serialize, ToSchema)]
pub struct ExplorePackageListResponse {
    pub items: Vec<ExploreEntryDto>,
    pub total: usize,
    pub page: u64,
    pub per_page: u64,
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
    access: web::Data<AccessConfig>,
) -> Result<impl Responder, AppError> {
    let accessible = access.explore_accessible_registries_for(&identity);

    if let Some(ref reg) = query.registry {
        if !accessible.contains(reg) {
            return Ok(web::Json(ExplorePackageListResponse {
                items: vec![],
                total: 0,
                page: query.page,
                per_page: query.per_page,
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

    let filter = ExploreFilter {
        registry: query.registry.clone(),
        registries: registries.clone(),
        name_contains: query.name.clone(),
        sort_by: sort_by.clone(),
        limit: query.per_page,
        offset: query.page * query.per_page,
    };
    let count_filter = ExploreFilter {
        registry: query.registry.clone(),
        registries,
        name_contains: query.name.clone(),
        sort_by,
        limit: 0,
        offset: 0,
    };

    let (packages, total) = tokio::try_join!(
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
        page: query.page,
        per_page: query.per_page,
    }))
}

// ── Registry stats ─────────────────────────────────────────────────────────────

#[derive(Serialize, ToSchema)]
pub struct RegistryStatDto {
    pub registry: String,
    pub package_count: u64,
    pub total_downloads: u64,
}

/// Per-registry package counts and download totals for the explorer sidebar.
#[utoipa::path(
    get,
    path = "/api/v1/explore/registries",
    tag = "explore",
    responses(
        (status = 200, description = "Registry statistics", body = Vec<RegistryStatDto>),
    ),
    security(("bearer_token" = [])),
)]
#[get("/api/v1/explore/registries")]
pub async fn explore_registry_stats(
    identity: AuthIdentity,
    admin_svc: web::Data<Arc<AdminService>>,
    access: web::Data<AccessConfig>,
) -> Result<impl Responder, AppError> {
    let accessible: Vec<String> = access
        .explore_accessible_registries_for(&identity)
        .into_iter()
        .collect();

    let stats = admin_svc
        .registry_explore_stats(&accessible)
        .await
        .map_err(AppError::from)?;

    let dtos: Vec<RegistryStatDto> = stats
        .into_iter()
        .map(|s| RegistryStatDto {
            registry: s.registry,
            package_count: s.package_count,
            total_downloads: s.total_downloads,
        })
        .collect();

    Ok(web::Json(dtos))
}

// ── Package detail ─────────────────────────────────────────────────────────────

#[derive(Deserialize, IntoParams)]
pub struct PackageDetailPath {
    pub registry: String,
    pub name: String,
}

#[derive(Serialize, ToSchema)]
pub struct ExplorePackageDetailResponse {
    pub registry: String,
    pub name: String,
    pub gate: GateDto,
    pub versions: Vec<ExploreVersionDto>,
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
    /// `"proxied"` | `"local"`
    pub source: String,
    pub firewall: FirewallDto,
    pub download_count: u64,
    pub last_accessed: Option<String>,
    pub published_at: Option<String>,
    pub is_prerelease: bool,
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
    params(PackageDetailPath),
    responses(
        (status = 200, description = "Package detail", body = ExplorePackageDetailResponse),
    ),
    security(("bearer_token" = [])),
)]
#[get("/api/v1/explore/packages/{registry}/{name}")]
pub async fn explore_package_detail(
    path: web::Path<PackageDetailPath>,
    identity: AuthIdentity,
    admin_svc: web::Data<Arc<AdminService>>,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    access: web::Data<AccessConfig>,
) -> Result<impl Responder, AppError> {
    let registry = &path.registry;
    let name = &path.name;

    // Gate: registry-level proxy access
    let registry_accessible = access
        .accessible_registries_for(&identity)
        .contains(registry);

    // Gate: beta channel membership
    let beta_member = if let Some(beta_port) = local_svc.beta_channel.get(registry) {
        beta_port
            .is_member(registry, &identity)
            .await
            .unwrap_or(false)
    } else {
        false
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
    let proxied_summaries = admin_svc
        .list_packages(proxied_filter)
        .await
        .map_err(AppError::from)?;

    // Local versions from local_packages
    let local_versions = local_svc
        .backend
        .get_versions(registry, name)
        .await
        .unwrap_or_default();

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
        versions.push(ExploreVersionDto {
            version: summary.package_id.version,
            source: "proxied".to_string(),
            firewall,
            download_count: summary.access_count,
            last_accessed: summary.last_accessed.map(format_dt),
            published_at: None,
            is_prerelease,
        });
    }

    for pkg in local_versions {
        let firewall = if pkg.yanked {
            FirewallDto::Yanked
        } else {
            FirewallDto::Clear
        };
        let is_prerelease = pkg.version.contains('-');
        versions.push(ExploreVersionDto {
            version: pkg.version,
            source: "local".to_string(),
            firewall,
            download_count: 0,
            last_accessed: None,
            published_at: Some(pkg.published_at.to_rfc3339()),
            is_prerelease,
        });
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
    }))
}

fn format_dt(dt: DateTime<Utc>) -> String {
    dt.to_rfc3339()
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
    access: web::Data<AccessConfig>,
) -> Result<impl Responder, AppError> {
    let accessible = access.explore_accessible_registries_for(&identity);

    tracing::info!(
        name = %query.name,
        accessible_registries = ?accessible,
        "upstream search: resolving clients"
    );

    // Collect registry clients to search
    let clients_to_search: Vec<(String, _)> = proxy_svc
        .registries
        .iter()
        .filter(|(name, _)| {
            if let Some(ref reg) = query.registry {
                name.as_str() == reg && accessible.contains(name.as_str())
            } else {
                accessible.contains(name.as_str())
            }
        })
        .map(|(name, client)| (name.clone(), Arc::clone(client)))
        .collect();

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
    let known = admin_svc
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
