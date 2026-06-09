use std::sync::Arc;

use actix_web::{post, web, Responder};
use sqlx::PgPool;

use batlehub_core::services::ProxyService;

use crate::handlers::back_office::require_admin;
use crate::{error::AppError, extractors::AuthIdentity, RegistryMap};

use super::system::ClearCacheResponse;

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
