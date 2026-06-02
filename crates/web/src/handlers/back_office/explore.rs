use std::sync::Arc;

use actix_web::{post, web, Responder};
use serde::Deserialize;
use utoipa::ToSchema;

use batlehub_core::services::AdminService;

use super::require_admin;
use crate::{error::AppError, extractors::AuthIdentity};

#[derive(Deserialize, ToSchema)]
pub struct ExploreInvalidateRequest {
    /// Registry to invalidate. When absent, the entire explore cache is cleared.
    pub registry: Option<String>,
}

/// Invalidate the explore cache for a registry (or the entire cache when no registry is given).
///
/// Forces the next explore request to re-query the database instead of returning
/// cached results. Any in-flight stale entries are discarded.
#[utoipa::path(
    post,
    path = "/api/v1/admin/explore/invalidate",
    tag = "back-office",
    request_body = ExploreInvalidateRequest,
    responses(
        (status = 200, description = "Cache invalidated"),
        (status = 403, description = "Admin role required"),
    ),
    security(("bearer_token" = [])),
)]
#[post("/api/v1/admin/explore/invalidate")]
pub async fn invalidate_explore_cache(
    body: web::Json<ExploreInvalidateRequest>,
    identity: AuthIdentity,
    admin_svc: web::Data<Arc<AdminService>>,
) -> Result<impl Responder, AppError> {
    require_admin(&identity)?;
    admin_svc
        .explore_cache
        .invalidate(body.registry.as_deref())
        .await;
    Ok(web::Json(serde_json::json!({ "ok": true })))
}
