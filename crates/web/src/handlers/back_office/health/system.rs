use std::collections::HashMap;
use std::sync::Arc;

use actix_web::{get, web, Responder};
use chrono::{DateTime, Utc};
use futures::future::join_all;
use serde::Serialize;
use utoipa::ToSchema;

use batlehub_core::ports::RecentErrorRecord;
use batlehub_core::services::{AdminService, ProxyService};

use crate::handlers::back_office::require_admin;
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

impl From<RecentErrorRecord> for RecentErrorDto {
    fn from(r: RecentErrorRecord) -> Self {
        RecentErrorDto {
            timestamp: r.created_at,
            user_id: r.user_id,
            package_name: r.package_name,
            version: r.package_version,
            error_type: r.outcome,
            reason: r.deny_reason.unwrap_or_default(),
        }
    }
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

#[derive(Serialize, ToSchema)]
pub struct ClearCacheResponse {
    /// Number of artifacts removed from storage.
    pub cleared: usize,
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
    admin_svc: web::Data<Arc<AdminService>>,
    proxy_svc: web::Data<Arc<ProxyService>>,
) -> Result<impl Responder, AppError> {
    require_admin(&identity)?;

    let repo = &admin_svc.repo;

    let mut registries: Vec<(String, String)> = registry_map.entries();
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
    let pkg_counts: HashMap<String, i64> = repo
        .registry_package_counts(&registry_names)
        .await
        .map_err(|e| AppError::internal(format!("health query failed: {e}")))?;

    // ── Batch query 2: event stats for all registries in one round-trip ───────
    let mut event_stats: HashMap<String, (Option<DateTime<Utc>>, i64, i64)> = repo
        .registry_event_stats(&registry_names)
        .await
        .map_err(|e| AppError::internal(format!("health query failed: {e}")))?;

    // ── Per-registry: storage stats + recent errors — run all concurrently ────
    let per_reg_futures = registries.iter().map(|(registry, _)| {
        let storage = Arc::clone(&proxy_svc.storage);
        let repo = Arc::clone(repo);
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

            let recent_errors: Vec<RecentErrorDto> = repo
                .recent_registry_errors(&registry, 10)
                .await
                .unwrap_or_default()
                .into_iter()
                .map(RecentErrorDto::from)
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
