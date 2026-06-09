use std::collections::HashMap;
use std::sync::Arc;

use actix_web::{get, post, web, Responder};
use chrono::{DateTime, Utc};
use futures::future::join_all;
use serde::Serialize;
use sqlx::{PgPool, Row};
use utoipa::ToSchema;

use batlehub_core::services::{AdminService, ProxyService};

use super::require_admin;
use crate::{error::AppError, extractors::AuthIdentity, RegistryMap};

#[derive(Serialize, ToSchema)]
pub struct RegistryAccessInfo {
    /// Role names (e.g. "anonymous", "user", "admin") that can access this registry.
    pub roles: Vec<String>,
    /// Group names configured with access to this registry.
    pub groups: Vec<String>,
}

#[derive(Serialize, ToSchema)]
pub struct RecentErrorDto {
    pub timestamp: DateTime<Utc>,
    pub user_id: Option<String>,
    pub package_name: String,
    pub version: String,
    /// "denied" (blocked / RBAC) or "error" (upstream proxy failure).
    pub error_type: String,
    pub reason: String,
}

#[derive(Serialize, ToSchema)]
pub struct RegistryHealthDto {
    pub registry: String,
    pub registry_type: String,
    /// Distinct packages tracked in the DB.
    pub package_count: i64,
    /// Cached artifact files on disk/storage.
    pub cached_artifact_count: i64,
    /// Sum of artifact sizes in bytes (null when no size data yet).
    pub total_size_bytes: Option<i64>,
    /// Timestamp of the most recent successful download.
    pub last_pull_at: Option<DateTime<Utc>>,
    /// Successful downloads in the last hour.
    pub pulls_last_hour: i64,
    /// Successful downloads in the last 24 hours.
    pub pulls_last_day: i64,
    /// Denied-access and upstream-error events in the last 24 h (newest first, max 10).
    pub recent_errors: Vec<RecentErrorDto>,
    /// Which roles and groups can access this registry.
    pub access: RegistryAccessInfo,
}

/// Get health information for all registries (admin).
#[utoipa::path(
    get,
    path = "/api/v1/admin/health",
    tag = "back-office",
    responses(
        (status = 200, description = "Registry health for all registries", body = Vec<RegistryHealthDto>),
        (status = 403, description = "Admin role required"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/api/v1/admin/health")]
pub async fn registry_health(
    identity: AuthIdentity,
    registry_map: web::Data<RegistryMap>,
    access_config: web::Data<crate::AccessConfigLock>,
    pool: Option<web::Data<PgPool>>,
    _admin_svc: web::Data<Arc<AdminService>>,
    proxy_svc: web::Data<Arc<ProxyService>>,
) -> Result<impl Responder, AppError> {
    require_admin(&identity)?;

    let pool = match pool {
        Some(p) => p,
        None => {
            // No DB configured — return empty list
            return Ok(web::Json(Vec::<RegistryHealthDto>::new()));
        }
    };
    let pool = pool.get_ref();

    let mut registries: Vec<(String, String)> = registry_map
        .0
        .read()
        .expect("registry map lock poisoned")
        .iter()
        .map(|(name, rtype)| (name.clone(), rtype.clone()))
        .collect();
    registries.sort_by(|a, b| a.0.cmp(&b.0));

    if registries.is_empty() {
        return Ok(web::Json(Vec::<RegistryHealthDto>::new()));
    }

    let registry_names: Vec<String> = registries.iter().map(|(n, _)| n.clone()).collect();

    // ── Snapshot access config once (avoid N repeated lock acquisitions) ──────
    let (anon_set, user_set, admin_set, groups_map) = {
        let ac = access_config.read().await;
        (
            ac.anonymous.clone(),
            ac.user.clone(),
            ac.admin.clone(),
            ac.groups.clone(),
        )
    };

    // ── Batch query 1: package counts for all registries in one round-trip ────
    let pkg_rows = sqlx::query(
        "SELECT registry, COUNT(DISTINCT package_name) AS cnt
         FROM package_statuses
         WHERE registry = ANY($1)
         GROUP BY registry",
    )
    .bind(&registry_names)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::internal(format!("health query failed: {e}")))?;

    let pkg_counts: HashMap<String, i64> = pkg_rows
        .into_iter()
        .map(|r| (r.get::<String, _>("registry"), r.get::<i64, _>("cnt")))
        .collect();

    // ── Batch query 2: event stats for all registries in one round-trip ───────
    let event_rows = sqlx::query(
        r#"SELECT
               registry,
               MAX(created_at) FILTER (WHERE action = 'download' AND outcome = 'allowed')
                   AS last_pull_at,
               COUNT(*) FILTER (
                   WHERE action = 'download' AND outcome = 'allowed'
                   AND created_at > NOW() - INTERVAL '1 hour'
               ) AS pulls_last_hour,
               COUNT(*) FILTER (
                   WHERE action = 'download' AND outcome = 'allowed'
                   AND created_at > NOW() - INTERVAL '1 day'
               ) AS pulls_last_day
           FROM access_events
           WHERE registry = ANY($1)
           GROUP BY registry"#,
    )
    .bind(&registry_names)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::internal(format!("health query failed: {e}")))?;

    let mut event_stats: HashMap<String, (Option<DateTime<Utc>>, i64, i64)> = event_rows
        .into_iter()
        .map(|r| {
            (
                r.get::<String, _>("registry"),
                (
                    r.try_get("last_pull_at").unwrap_or(None),
                    r.try_get("pulls_last_hour").unwrap_or(0),
                    r.try_get("pulls_last_day").unwrap_or(0),
                ),
            )
        })
        .collect();

    // ── Per-registry: storage stats + recent errors — run all concurrently ────
    let per_reg_futures = registries.iter().map(|(registry, _)| {
        let pool = pool.clone();
        let storage = Arc::clone(&proxy_svc.storage);
        let registry = registry.clone();
        async move {
            let prefix = format!("artifact:{}/", registry);
            let storage_stat = storage.stat_by_prefix(&prefix).await.ok();
            let (cached_artifact_count, total_size_bytes) = match storage_stat {
                Some((count, bytes)) => (
                    count as i64,
                    if count > 0 { Some(bytes as i64) } else { None },
                ),
                None => (0, None),
            };

            let error_rows = sqlx::query(
                r#"SELECT created_at, user_id, package_name, package_version, outcome, deny_reason
                   FROM access_events
                   WHERE registry = $1 AND outcome IN ('denied', 'error')
                   AND created_at > NOW() - INTERVAL '24 hours'
                   ORDER BY created_at DESC LIMIT 10"#,
            )
            .bind(&registry)
            .fetch_all(&pool)
            .await
            .unwrap_or_default();

            let recent_errors: Vec<RecentErrorDto> = error_rows
                .into_iter()
                .map(|r| RecentErrorDto {
                    timestamp: r.get("created_at"),
                    user_id: r.get("user_id"),
                    package_name: r.get("package_name"),
                    version: r.get("package_version"),
                    error_type: r.get::<String, _>("outcome"),
                    reason: r
                        .get::<Option<String>, _>("deny_reason")
                        .unwrap_or_default(),
                })
                .collect();

            (
                registry,
                cached_artifact_count,
                total_size_bytes,
                recent_errors,
            )
        }
    });

    let per_reg_results: Vec<(String, i64, Option<i64>, Vec<RecentErrorDto>)> =
        join_all(per_reg_futures).await;

    let mut per_reg_map: HashMap<String, (i64, Option<i64>, Vec<RecentErrorDto>)> = per_reg_results
        .into_iter()
        .map(|(reg, art_cnt, size, errors)| (reg, (art_cnt, size, errors)))
        .collect();

    // ── Assemble final response ───────────────────────────────────────────────
    let mut result: Vec<RegistryHealthDto> = registries
        .into_iter()
        .map(|(registry, registry_type)| {
            let package_count = pkg_counts.get(&registry).copied().unwrap_or(0);
            let (last_pull_at, pulls_last_hour, pulls_last_day) =
                event_stats.remove(&registry).unwrap_or((None, 0, 0));
            let (cached_artifact_count, total_size_bytes, recent_errors) =
                per_reg_map.remove(&registry).unwrap_or((0, None, vec![]));

            let mut roles = Vec::new();
            if anon_set.contains(&registry) {
                roles.push("anonymous".to_string());
            }
            if user_set.contains(&registry) {
                roles.push("user".to_string());
            }
            if admin_set.contains(&registry) {
                roles.push("admin".to_string());
            }
            let groups: Vec<String> = groups_map
                .iter()
                .filter(|(_, regs)| regs.contains(&registry))
                .map(|(g, _)| g.clone())
                .collect();

            RegistryHealthDto {
                registry,
                registry_type,
                package_count,
                cached_artifact_count,
                total_size_bytes,
                last_pull_at,
                pulls_last_hour,
                pulls_last_day,
                recent_errors,
                access: RegistryAccessInfo { roles, groups },
            }
        })
        .collect();

    result.sort_by(|a, b| a.registry.cmp(&b.registry));
    Ok(web::Json(result))
}

#[derive(Serialize, ToSchema)]
pub struct ClearCacheResponse {
    /// Number of artifacts removed from storage.
    pub cleared: usize,
}

/// Clear all cached artifacts for a specific registry (admin).
#[utoipa::path(
    post,
    path = "/api/v1/admin/registries/{registry}/clear-cache",
    tag = "back-office",
    params(
        ("registry" = String, Path, description = "Registry name"),
    ),
    responses(
        (status = 200, description = "Artifacts cleared", body = ClearCacheResponse),
        (status = 403, description = "Admin role required"),
        (status = 404, description = "Registry not found"),
    ),
    security(("bearer_token" = [])),
)]
#[post("/api/v1/admin/registries/{registry}/clear-cache")]
pub async fn clear_registry_cache(
    identity: AuthIdentity,
    path: web::Path<String>,
    registry_map: web::Data<RegistryMap>,
    pool: Option<web::Data<PgPool>>,
    proxy_svc: web::Data<Arc<ProxyService>>,
) -> Result<impl Responder, AppError> {
    require_admin(&identity)?;

    let registry = path.into_inner();

    if !registry_map.contains(&registry) {
        return Err(AppError::not_found("registry not found"));
    }

    let prefix = format!("artifact:{}/", registry);

    tracing::info!(registry = %registry, prefix = %prefix, "clear_registry_cache: starting");

    // Delete all cached artifacts for the registry directly from storage.
    // This works regardless of whether artifact_storage has records (e.g. single-backend config).
    let cleared = proxy_svc
        .storage
        .delete_by_prefix(&prefix)
        .await
        .map_err(AppError::from)?;

    tracing::info!(registry = %registry, cleared, "clear_registry_cache: done");

    // Clean up any remaining artifact_storage records.
    if let Some(p) = pool {
        let _ = sqlx::query("DELETE FROM artifact_storage WHERE storage_key LIKE $1")
            .bind(format!("{prefix}%"))
            .execute(p.get_ref())
            .await;
    }

    Ok(web::Json(ClearCacheResponse { cleared }))
}
