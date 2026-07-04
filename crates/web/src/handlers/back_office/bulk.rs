use std::sync::Arc;

use actix_web::{post, web, HttpResponse, Responder};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use batlehub_core::{
    entities::{AccessAction, Identity},
    ports::BulkResult,
    services::LocalRegistryService,
};

use super::require_admin;
use crate::{error::AppError, extractors::AuthIdentity};

/// Flatten a `BulkPackageRequest` into the `(name, version)` pairs expected by
/// `LocalRegistryBackend::bulk_*`.
fn bulk_items(body: web::Json<BulkPackageRequest>) -> Vec<(String, String)> {
    body.into_inner()
        .packages
        .into_iter()
        .map(|p| (p.name, p.version))
        .collect()
}

/// Record one lifecycle audit event per item that `result` reports as
/// succeeded. `LocalRegistryBackend::bulk_*` only returns a success *count*,
/// not which coordinates succeeded, so the succeeded set is `items` minus the
/// ones `result.failed` names — mirrors the single-item yank/unyank/deprecate/
/// unlist handlers, which already audit through `LocalRegistryService::record_lifecycle_action`.
async fn record_bulk_lifecycle_audit(
    local_svc: &LocalRegistryService,
    registry: &str,
    items: &[(String, String)],
    result: &BulkResult,
    action: AccessAction,
    identity: &Identity,
) {
    let failed: std::collections::HashSet<(&str, &str)> = result
        .failed
        .iter()
        .map(|(name, version, _)| (name.as_str(), version.as_str()))
        .collect();
    for (name, version) in items {
        if failed.contains(&(name.as_str(), version.as_str())) {
            continue;
        }
        local_svc
            .record_lifecycle_action(registry, name, version, action.clone(), identity)
            .await;
    }
}

/// Convert a `BulkResult` into the API response DTO.
fn bulk_response(result: BulkResult) -> BulkPackageResponse {
    BulkPackageResponse {
        processed: result.processed,
        succeeded: result.succeeded,
        failed: result
            .failed
            .into_iter()
            .map(|(name, version, error)| BulkPackageFailureDto {
                name,
                version,
                error,
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::require_admin;
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
    fn require_admin_fails_for_user() {
        assert!(require_admin(&id(Role::User)).is_err());
    }

    #[test]
    fn require_admin_fails_for_anonymous() {
        assert!(require_admin(&id(Role::Anonymous)).is_err());
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct BulkPackageItem {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct BulkPackageRequest {
    pub packages: Vec<BulkPackageItem>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct BulkPackageFailureDto {
    pub name: String,
    pub version: String,
    pub error: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct BulkPackageResponse {
    pub processed: usize,
    pub succeeded: usize,
    pub failed: Vec<BulkPackageFailureDto>,
}

/// Bulk-yank versions in a local/hybrid registry (admin).
#[utoipa::path(
    post,
    path = "/api/v1/admin/registries/{registry}/bulk-yank",
    tag = "back-office",
    params(("registry" = String, Path, description = "Registry name")),
    request_body = BulkPackageRequest,
    responses(
        (status = 200, description = "Bulk yank result", body = BulkPackageResponse),
        (status = 403, description = "Admin role required"),
    ),
    security(("bearer_token" = [])),
)]
#[post("/api/v1/admin/registries/{registry}/bulk-yank")]
pub async fn bulk_yank(
    path: web::Path<String>,
    body: web::Json<BulkPackageRequest>,
    identity: AuthIdentity,
    local_svc: web::Data<Arc<LocalRegistryService>>,
) -> Result<impl Responder, AppError> {
    require_admin(&identity)?;
    let registry = path.into_inner();
    let items = bulk_items(body);
    let result = local_svc
        .backend
        .bulk_yank(&registry, &items)
        .await
        .map_err(AppError::from)?;
    record_bulk_lifecycle_audit(
        &local_svc,
        &registry,
        &items,
        &result,
        AccessAction::Yank,
        &identity.0,
    )
    .await;
    Ok(HttpResponse::Ok().json(bulk_response(result)))
}

/// Bulk-unyank versions in a local/hybrid registry (admin).
#[utoipa::path(
    post,
    path = "/api/v1/admin/registries/{registry}/bulk-unyank",
    tag = "back-office",
    params(("registry" = String, Path, description = "Registry name")),
    request_body = BulkPackageRequest,
    responses(
        (status = 200, description = "Bulk unyank result", body = BulkPackageResponse),
        (status = 403, description = "Admin role required"),
    ),
    security(("bearer_token" = [])),
)]
#[post("/api/v1/admin/registries/{registry}/bulk-unyank")]
pub async fn bulk_unyank(
    path: web::Path<String>,
    body: web::Json<BulkPackageRequest>,
    identity: AuthIdentity,
    local_svc: web::Data<Arc<LocalRegistryService>>,
) -> Result<impl Responder, AppError> {
    require_admin(&identity)?;
    let registry = path.into_inner();
    let items = bulk_items(body);
    let result = local_svc
        .backend
        .bulk_unyank(&registry, &items)
        .await
        .map_err(AppError::from)?;
    record_bulk_lifecycle_audit(
        &local_svc,
        &registry,
        &items,
        &result,
        AccessAction::Unyank,
        &identity.0,
    )
    .await;
    Ok(HttpResponse::Ok().json(bulk_response(result)))
}

/// Bulk-delete versions from a local/hybrid registry (admin).
#[utoipa::path(
    post,
    path = "/api/v1/admin/registries/{registry}/bulk-delete",
    tag = "back-office",
    params(("registry" = String, Path, description = "Registry name")),
    request_body = BulkPackageRequest,
    responses(
        (status = 200, description = "Bulk delete result", body = BulkPackageResponse),
        (status = 403, description = "Admin role required"),
    ),
    security(("bearer_token" = [])),
)]
#[post("/api/v1/admin/registries/{registry}/bulk-delete")]
pub async fn bulk_delete(
    path: web::Path<String>,
    body: web::Json<BulkPackageRequest>,
    identity: AuthIdentity,
    local_svc: web::Data<Arc<LocalRegistryService>>,
) -> Result<impl Responder, AppError> {
    require_admin(&identity)?;
    let registry = path.into_inner();
    let items = bulk_items(body);
    let result = local_svc
        .backend
        .bulk_remove_versions(&registry, &items)
        .await
        .map_err(AppError::from)?;
    record_bulk_lifecycle_audit(
        &local_svc,
        &registry,
        &items,
        &result,
        AccessAction::Delete,
        &identity.0,
    )
    .await;
    Ok(HttpResponse::Ok().json(bulk_response(result)))
}

// ── Deprecation & unlisting (single version) ────────────────────────────────

#[derive(Debug, Deserialize, ToSchema)]
pub struct PackageVersionRequest {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct DeprecateRequest {
    pub name: String,
    pub version: String,
    /// Optional human-readable deprecation message (mirrored into npm's native
    /// `deprecated` field for npm registries).
    #[serde(default)]
    pub message: Option<String>,
}

/// Flag a version as deprecated. It stays listed and downloadable (admin).
#[utoipa::path(
    post,
    path = "/api/v1/admin/registries/{registry}/deprecate",
    tag = "back-office",
    params(("registry" = String, Path, description = "Registry name")),
    request_body = DeprecateRequest,
    responses(
        (status = 200, description = "Version deprecated"),
        (status = 403, description = "Admin role required"),
    ),
    security(("bearer_token" = [])),
)]
#[post("/api/v1/admin/registries/{registry}/deprecate")]
pub async fn deprecate(
    path: web::Path<String>,
    body: web::Json<DeprecateRequest>,
    identity: AuthIdentity,
    local_svc: web::Data<Arc<LocalRegistryService>>,
) -> Result<impl Responder, AppError> {
    require_admin(&identity)?;
    let registry = path.into_inner();
    let body = body.into_inner();
    local_svc
        .deprecate(
            &registry,
            &body.name,
            &body.version,
            body.message.as_deref(),
            &identity.0,
        )
        .await
        .map_err(AppError::from)?;
    Ok(HttpResponse::Ok().finish())
}

/// Reverse a deprecation (admin).
#[utoipa::path(
    post,
    path = "/api/v1/admin/registries/{registry}/undeprecate",
    tag = "back-office",
    params(("registry" = String, Path, description = "Registry name")),
    request_body = PackageVersionRequest,
    responses(
        (status = 200, description = "Version undeprecated"),
        (status = 403, description = "Admin role required"),
    ),
    security(("bearer_token" = [])),
)]
#[post("/api/v1/admin/registries/{registry}/undeprecate")]
pub async fn undeprecate(
    path: web::Path<String>,
    body: web::Json<PackageVersionRequest>,
    identity: AuthIdentity,
    local_svc: web::Data<Arc<LocalRegistryService>>,
) -> Result<impl Responder, AppError> {
    require_admin(&identity)?;
    let registry = path.into_inner();
    let body = body.into_inner();
    local_svc
        .undeprecate(&registry, &body.name, &body.version, &identity.0)
        .await
        .map_err(AppError::from)?;
    Ok(HttpResponse::Ok().finish())
}

/// Hide a version from registry-protocol listings; still downloadable by exact
/// coordinate (admin).
#[utoipa::path(
    post,
    path = "/api/v1/admin/registries/{registry}/unlist",
    tag = "back-office",
    params(("registry" = String, Path, description = "Registry name")),
    request_body = PackageVersionRequest,
    responses(
        (status = 200, description = "Version unlisted"),
        (status = 403, description = "Admin role required"),
    ),
    security(("bearer_token" = [])),
)]
#[post("/api/v1/admin/registries/{registry}/unlist")]
pub async fn unlist(
    path: web::Path<String>,
    body: web::Json<PackageVersionRequest>,
    identity: AuthIdentity,
    local_svc: web::Data<Arc<LocalRegistryService>>,
) -> Result<impl Responder, AppError> {
    require_admin(&identity)?;
    let registry = path.into_inner();
    let body = body.into_inner();
    local_svc
        .unlist(&registry, &body.name, &body.version, &identity.0)
        .await
        .map_err(AppError::from)?;
    Ok(HttpResponse::Ok().finish())
}

/// Make an unlisted version visible in listings again (admin).
#[utoipa::path(
    post,
    path = "/api/v1/admin/registries/{registry}/relist",
    tag = "back-office",
    params(("registry" = String, Path, description = "Registry name")),
    request_body = PackageVersionRequest,
    responses(
        (status = 200, description = "Version relisted"),
        (status = 403, description = "Admin role required"),
    ),
    security(("bearer_token" = [])),
)]
#[post("/api/v1/admin/registries/{registry}/relist")]
pub async fn relist(
    path: web::Path<String>,
    body: web::Json<PackageVersionRequest>,
    identity: AuthIdentity,
    local_svc: web::Data<Arc<LocalRegistryService>>,
) -> Result<impl Responder, AppError> {
    require_admin(&identity)?;
    let registry = path.into_inner();
    let body = body.into_inner();
    local_svc
        .relist(&registry, &body.name, &body.version, &identity.0)
        .await
        .map_err(AppError::from)?;
    Ok(HttpResponse::Ok().finish())
}
