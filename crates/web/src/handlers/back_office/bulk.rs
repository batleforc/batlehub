use std::sync::Arc;

use actix_web::{HttpResponse, Responder, post, web};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use batlehub_core::{entities::Role, services::LocalRegistryService};

use crate::{error::AppError, extractors::AuthIdentity};

fn require_admin(identity: &AuthIdentity) -> Result<(), AppError> {
    if identity.role != Role::Admin {
        Err(AppError::forbidden("admin role required"))
    } else {
        Ok(())
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
pub struct BulkFailureDto {
    pub name: String,
    pub version: String,
    pub error: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct BulkPackageResponse {
    pub processed: usize,
    pub succeeded: usize,
    pub failed: Vec<BulkFailureDto>,
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
    let items: Vec<(String, String)> = body
        .into_inner()
        .packages
        .into_iter()
        .map(|p| (p.name, p.version))
        .collect();
    let result = local_svc
        .backend
        .bulk_yank(&registry, &items)
        .await
        .map_err(AppError::from)?;
    Ok(HttpResponse::Ok().json(BulkPackageResponse {
        processed: result.processed,
        succeeded: result.succeeded,
        failed: result
            .failed
            .into_iter()
            .map(|(name, version, error)| BulkFailureDto { name, version, error })
            .collect(),
    }))
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
    let items: Vec<(String, String)> = body
        .into_inner()
        .packages
        .into_iter()
        .map(|p| (p.name, p.version))
        .collect();
    let result = local_svc
        .backend
        .bulk_unyank(&registry, &items)
        .await
        .map_err(AppError::from)?;
    Ok(HttpResponse::Ok().json(BulkPackageResponse {
        processed: result.processed,
        succeeded: result.succeeded,
        failed: result
            .failed
            .into_iter()
            .map(|(name, version, error)| BulkFailureDto { name, version, error })
            .collect(),
    }))
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
    let items: Vec<(String, String)> = body
        .into_inner()
        .packages
        .into_iter()
        .map(|p| (p.name, p.version))
        .collect();
    let result = local_svc
        .backend
        .bulk_remove_versions(&registry, &items)
        .await
        .map_err(AppError::from)?;
    Ok(HttpResponse::Ok().json(BulkPackageResponse {
        processed: result.processed,
        succeeded: result.succeeded,
        failed: result
            .failed
            .into_iter()
            .map(|(name, version, error)| BulkFailureDto { name, version, error })
            .collect(),
    }))
}
