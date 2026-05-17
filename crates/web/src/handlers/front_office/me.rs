use actix_web::{Responder, get, web};
use serde::Serialize;
use utoipa::ToSchema;

use proxy_cache_core::entities::Role;

use crate::{AccessConfig, extractors::AuthIdentity};

#[derive(Serialize, ToSchema)]
pub struct MeResponse {
    pub user_id: Option<String>,
    pub role: Role,
    pub auth_provider: Option<String>,
    /// Whether the current user's role has access to at least one configured registry.
    pub has_registry_access: bool,
}

/// Return the current caller's identity and role.
#[utoipa::path(
    get,
    path = "/api/v1/me",
    tag = "front-office",
    responses(
        (status = 200, description = "Current identity", body = MeResponse),
    ),
    security(("bearer_token" = [])),
)]
#[get("/api/v1/me")]
pub async fn me(identity: AuthIdentity, access: web::Data<AccessConfig>) -> impl Responder {
    web::Json(MeResponse {
        user_id: identity.user_id.clone(),
        role: identity.role.clone(),
        auth_provider: identity.auth_provider.clone(),
        has_registry_access: access.has_registry_access(&identity.role),
    })
}
