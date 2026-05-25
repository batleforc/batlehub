use std::collections::HashMap;
use std::sync::Arc;

use actix_web::{Responder, post, web};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use batlehub_core::{entities::Role, services::WarmingService};

use crate::{RegistryMap, error::AppError, extractors::AuthIdentity};

/// Map of registry name → WarmingService, injected as app data.
pub type WarmingServiceMap = HashMap<String, Arc<WarmingService>>;

#[derive(Debug, Deserialize, ToSchema)]
pub struct WarmRequest {
    /// Package name to warm, optionally with a pinned version (`"lodash"` or `"lodash@4.17.21"`).
    pub package: String,
    /// Override the number of most-recent versions to warm. Falls back to the registry's
    /// `warm_latest_n` config when absent.
    pub versions: Option<usize>,
}

#[derive(Serialize, ToSchema)]
pub struct WarmResponse {
    pub warmed: usize,
    pub skipped: usize,
    pub errors: usize,
}

/// Pre-warm cached artifacts for a specific package in a registry (admin).
#[utoipa::path(
    post,
    path = "/api/v1/admin/registries/{registry}/warm",
    tag = "back-office",
    params(
        ("registry" = String, Path, description = "Registry name"),
    ),
    request_body = WarmRequest,
    responses(
        (status = 200, description = "Warming completed", body = WarmResponse),
        (status = 403, description = "Admin role required"),
        (status = 404, description = "Registry not found or warming not configured"),
    ),
    security(("bearer_token" = [])),
)]
#[post("/api/v1/admin/registries/{registry}/warm")]
pub async fn warm_registry(
    identity: AuthIdentity,
    path: web::Path<String>,
    body: web::Json<WarmRequest>,
    registry_map: web::Data<RegistryMap>,
    warming_map: web::Data<WarmingServiceMap>,
) -> Result<impl Responder, AppError> {
    if identity.role != Role::Admin {
        return Err(AppError::forbidden("admin role required"));
    }

    let registry = path.into_inner();

    if !registry_map.0.contains_key(&registry) {
        return Err(AppError::not_found("registry not found"));
    }

    let svc = warming_map
        .get(&registry)
        .ok_or_else(|| AppError::not_found("warming not configured for this registry"))?;

    // Use caller-specified version count when provided; fall back to configured default.
    let report = if let Some(n) = body.versions {
        svc.with_latest_n(n).warm_package(&body.package).await
    } else {
        svc.warm_package(&body.package).await
    };

    Ok(web::Json(WarmResponse {
        warmed: report.warmed,
        skipped: report.skipped,
        errors: report.errors,
    }))
}
