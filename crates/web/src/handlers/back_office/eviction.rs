use std::collections::HashMap;
use std::sync::Arc;

use actix_web::{post, web, Responder};
use serde::Serialize;
use utoipa::ToSchema;

use batlehub_core::services::EvictionService;

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
