use std::sync::Arc;

use actix_web::{post, web, Responder};
use serde::Deserialize;
use utoipa::ToSchema;

use batlehub_core::services::AdminService;

use crate::{error::AppError, extractors::AuthIdentity, handlers::schemas::OkResponse};

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
        (status = 200, description = "Cache invalidated", body = OkResponse),
        (status = 403, description = "`system:write` required"),
    ),
    security(("bearer_token" = [])),
)]
#[post("/api/v1/admin/explore/invalidate")]
pub async fn invalidate_explore_cache(
    body: web::Json<ExploreInvalidateRequest>,
    identity: AuthIdentity,
    admin_svc: web::Data<Arc<AdminService>>,
    hot: web::Data<batlehub_core::services::hot_config::HotConfigLock>,
) -> Result<impl Responder, AppError> {
    crate::handlers::back_office::require_verb(
        &identity,
        batlehub_core::entities::Action::SystemWrite,
        None,
        &hot,
    )
    .await?;
    admin_svc
        .explore_cache
        .invalidate(body.registry.as_deref())
        .await;
    Ok(web::Json(OkResponse::new()))
}
