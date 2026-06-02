use actix_web::{get, web, Responder};
use serde::Serialize;
use utoipa::ToSchema;

use batlehub_core::entities::Role;

use crate::{extractors::AuthIdentity};

#[derive(Serialize, ToSchema)]
pub struct MeResponse {
    pub user_id: Option<String>,
    pub role: Role,
    pub auth_provider: Option<String>,
    /// Whether the current user has access to at least one configured registry.
    pub has_registry_access: bool,
    /// Dynamic groups the user belongs to (populated by OIDC providers).
    pub groups: Vec<String>,
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
pub async fn me(identity: AuthIdentity, access: web::Data<crate::AccessConfigLock>) -> impl Responder {
    web::Json(MeResponse {
        user_id: identity.user_id.clone(),
        role: identity.role.clone(),
        auth_provider: identity.auth_provider.clone(),
        has_registry_access: access.read().await.has_registry_access(&identity),
        groups: identity.groups.clone(),
    })
}
