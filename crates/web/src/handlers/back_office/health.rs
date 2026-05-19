use std::sync::Arc;

use actix_web::{Responder, get, post, web};
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::{PgPool, Row};
use utoipa::ToSchema;

use proxy_cache_core::{
    entities::Role,
    services::{AdminService, ProxyService},
};

use crate::{AccessConfig, RegistryMap, error::AppError, extractors::AuthIdentity};

fn require_admin(identity: &AuthIdentity) -> Result<(), AppError> {
    if identity.role != Role::Admin {
        Err(AppError::forbidden("admin role required"))
    } else {
        Ok(())
    }
}

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
    access_config: web::Data<AccessConfig>,
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

    let mut result: Vec<RegistryHealthDto> = Vec::new();

    let mut registries: Vec<(String, String)> = registry_map
        .0
        .iter()
        .map(|(name, rtype)| (name.clone(), rtype.clone()))
        .collect();
    registries.sort_by(|a, b| a.0.cmp(&b.0));

    for (registry, registry_type) in registries {
        // ── Package count ─────────────────────────────────────────────────────
        let package_count: i64 = sqlx::query(
            "SELECT COUNT(DISTINCT package_name) FROM package_statuses WHERE registry = $1",
        )
        .bind(&registry)
        .fetch_one(pool)
        .await
        .map(|r| r.try_get::<i64, _>(0).unwrap_or(0))
        .unwrap_or(0);

        // ── Artifact count + total size (from storage directly) ───────────────
        let prefix = format!("artifact:{}/", registry);
        let (cached_artifact_count, total_size_bytes): (i64, Option<i64>) =
            match proxy_svc.storage.stat_by_prefix(&prefix).await {
                Ok((count, bytes)) => (
                    count as i64,
                    if count > 0 { Some(bytes as i64) } else { None },
                ),
                Err(_) => (0, None),
            };

        // ── Last pull ─────────────────────────────────────────────────────────
        let last_pull_at: Option<DateTime<Utc>> = sqlx::query(
            r#"SELECT MAX(created_at) FROM access_events
               WHERE registry = $1 AND action = 'download' AND outcome = 'allowed'"#,
        )
        .bind(&registry)
        .fetch_one(pool)
        .await
        .map(|r| r.try_get::<Option<DateTime<Utc>>, _>(0).unwrap_or(None))
        .unwrap_or(None);

        // ── Pulls last hour ───────────────────────────────────────────────────
        let pulls_last_hour: i64 = sqlx::query(
            r#"SELECT COUNT(*) FROM access_events
               WHERE registry = $1 AND action = 'download' AND outcome = 'allowed'
               AND created_at > NOW() - INTERVAL '1 hour'"#,
        )
        .bind(&registry)
        .fetch_one(pool)
        .await
        .map(|r| r.try_get::<i64, _>(0).unwrap_or(0))
        .unwrap_or(0);

        // ── Pulls last day ────────────────────────────────────────────────────
        let pulls_last_day: i64 = sqlx::query(
            r#"SELECT COUNT(*) FROM access_events
               WHERE registry = $1 AND action = 'download' AND outcome = 'allowed'
               AND created_at > NOW() - INTERVAL '1 day'"#,
        )
        .bind(&registry)
        .fetch_one(pool)
        .await
        .map(|r| r.try_get::<i64, _>(0).unwrap_or(0))
        .unwrap_or(0);

        // ── Recent errors (denied + proxy errors) ─────────────────────────────
        let error_rows = sqlx::query(
            r#"SELECT created_at, user_id, package_name, package_version, outcome, deny_reason
               FROM access_events
               WHERE registry = $1 AND outcome IN ('denied', 'error')
               AND created_at > NOW() - INTERVAL '24 hours'
               ORDER BY created_at DESC LIMIT 10"#,
        )
        .bind(&registry)
        .fetch_all(pool)
        .await
        .unwrap_or_default();

        let recent_errors = error_rows
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

        // ── Access info ───────────────────────────────────────────────────────
        let mut roles = Vec::new();
        if access_config.anonymous.contains(&registry) {
            roles.push("anonymous".to_string());
        }
        if access_config.user.contains(&registry) {
            roles.push("user".to_string());
        }
        if access_config.admin.contains(&registry) {
            roles.push("admin".to_string());
        }

        let groups: Vec<String> = access_config
            .groups
            .iter()
            .filter(|(_, registries)| registries.contains(&registry))
            .map(|(group, _)| group.clone())
            .collect();

        result.push(RegistryHealthDto {
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
        });
    }

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

    if !registry_map.0.contains_key(&registry) {
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
