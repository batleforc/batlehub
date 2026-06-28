use std::collections::HashMap;
use std::sync::Arc;

use actix_web::{delete, post, web, Responder};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use batlehub_core::services::{validate_path_safe, EvictionService, ProxyService};

use super::require_admin;
use crate::{error::AppError, extractors::AuthIdentity, RegistryMap};

/// Map of registry name → `EvictionService`, injected as app data.
pub type EvictionServiceMap = HashMap<String, Arc<EvictionService>>;

#[derive(Serialize, ToSchema)]
pub struct EvictResponse {
    pub total: usize,
    pub evicted_ttl: usize,
    pub evicted_idle: usize,
    pub evicted_old_versions: usize,
    pub evicted_lru: usize,
}

/// Run the configured eviction strategies for a registry's cache (admin).
#[utoipa::path(
    post,
    path = "/api/v1/admin/registries/{registry}/evict",
    tag = "back-office",
    params(
        ("registry" = String, Path, description = "Registry name"),
    ),
    responses(
        (status = 200, description = "Eviction completed", body = EvictResponse),
        (status = 403, description = "Admin role required"),
        (status = 404, description = "Registry not found or eviction not configured"),
    ),
    security(("bearer_token" = [])),
)]
#[post("/api/v1/admin/registries/{registry}/evict")]
pub async fn evict_registry(
    identity: AuthIdentity,
    path: web::Path<String>,
    registry_map: web::Data<RegistryMap>,
    eviction_map: web::Data<EvictionServiceMap>,
) -> Result<impl Responder, AppError> {
    require_admin(&identity)?;

    let registry = path.into_inner();

    if !registry_map.contains(&registry) {
        return Err(AppError::not_found("registry not found"));
    }

    let svc = eviction_map
        .get(&registry)
        .ok_or_else(|| AppError::not_found("eviction not configured for this registry"))?;

    let report = svc.run_all().await.map_err(AppError::from)?;

    Ok(web::Json(EvictResponse {
        total: report.total,
        evicted_ttl: report.evicted_ttl,
        evicted_idle: report.evicted_idle,
        evicted_old_versions: report.evicted_old_versions,
        evicted_lru: report.evicted_lru,
    }))
}

/// Request body for targeted proxy-cache artifact deletion.
#[derive(Deserialize, ToSchema)]
pub struct DeleteCacheRequest {
    /// Package name. Required for package-centric registries.
    pub name: Option<String>,
    /// Package version. Required for package-centric registries.
    pub version: Option<String>,
    /// Artifact path for path-addressed registries (deb/rpm/jetbrains),
    /// e.g. `"idea/ideaIC-2026.1.3.tar.gz"`. Takes precedence over name+version.
    pub path: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct DeleteCacheResponse {
    /// `true` if the artifact was present and removed; `false` if it was not cached.
    pub deleted: bool,
    /// The logical storage key that was targeted.
    pub artifact_key: String,
}

/// Delete a single proxy-cached artifact for a registry (admin).
///
/// Removes the artifact from storage and clears its cache metadata so the next
/// request re-downloads it from upstream. Use `path` for path-addressed registries
/// (deb/rpm/jetbrains); use `name` + `version` for all others.
#[utoipa::path(
    delete,
    path = "/api/v1/admin/registries/{registry}/cache",
    tag = "back-office",
    params(
        ("registry" = String, Path, description = "Registry name"),
    ),
    request_body = DeleteCacheRequest,
    responses(
        (status = 200, description = "Cache entry deleted (or was not present)", body = DeleteCacheResponse),
        (status = 400, description = "Invalid or missing name/version/path"),
        (status = 403, description = "Admin role required"),
        (status = 404, description = "Registry not found"),
    ),
    security(("bearer_token" = [])),
)]
#[delete("/api/v1/admin/registries/{registry}/cache")]
pub async fn delete_cached_artifact(
    identity: AuthIdentity,
    path: web::Path<String>,
    body: web::Json<DeleteCacheRequest>,
    registry_map: web::Data<RegistryMap>,
    proxy_svc: web::Data<Arc<ProxyService>>,
) -> Result<impl Responder, AppError> {
    require_admin(&identity)?;

    let registry = path.into_inner();

    if !registry_map.contains(&registry) {
        return Err(AppError::not_found("registry not found"));
    }

    validate_path_safe("registry", &registry).map_err(|e| AppError::bad_request(e.to_string()))?;

    let artifact_key = if let Some(p) = &body.path {
        if p.is_empty() {
            return Err(AppError::bad_request("path must not be empty"));
        }
        validate_path_safe("path", p).map_err(|e| AppError::bad_request(e.to_string()))?;
        format!("artifact:{registry}/repo/_/{p}")
    } else {
        let name = body
            .name
            .as_deref()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                AppError::bad_request("name is required for package-centric registries")
            })?;
        let version = body
            .version
            .as_deref()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                AppError::bad_request("version is required for package-centric registries")
            })?;
        validate_path_safe("name", name).map_err(|e| AppError::bad_request(e.to_string()))?;
        validate_path_safe("version", version).map_err(|e| AppError::bad_request(e.to_string()))?;
        format!("artifact:{registry}/{name}/{version}")
    };

    let deleted = proxy_svc
        .storage
        .delete(&artifact_key)
        .await
        .map_err(AppError::from)?;

    if deleted {
        if let Err(e) = proxy_svc
            .artifact_meta
            .delete_artifact_meta(&artifact_key)
            .await
        {
            tracing::warn!(key = %artifact_key, error = %e, "delete_cached_artifact: artifact_meta cleanup failed");
        }

        // Best-effort metadata cache invalidation so the next request re-resolves
        // versions from upstream instead of returning stale metadata.
        if let Some(meta_key) = artifact_key.strip_prefix("artifact:") {
            let cache_key = format!("meta:{meta_key}");
            if let Err(e) = proxy_svc.cache.invalidate(&cache_key).await {
                tracing::debug!(key = %cache_key, error = %e, "delete_cached_artifact: meta cache clear failed (non-fatal)");
            }
        }
    }

    Ok(web::Json(DeleteCacheResponse {
        deleted,
        artifact_key,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extractors::AuthIdentity;
    use batlehub_core::entities::{Identity, Role};

    fn id(role: Role) -> AuthIdentity {
        AuthIdentity(Identity {
            user_id: Some("u".into()),
            role,
            auth_provider: None,
            groups: vec![],
        })
    }

    #[test]
    fn require_admin_passes_for_admin() {
        assert!(require_admin(&id(Role::Admin)).is_ok());
    }

    #[test]
    fn require_admin_fails_for_non_admin() {
        assert!(require_admin(&id(Role::User)).is_err());
        assert!(require_admin(&id(Role::Anonymous)).is_err());
    }
}
