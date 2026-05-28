use std::sync::Arc;

use actix_web::{HttpResponse, Responder, delete, get, post, web};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use batlehub_core::{entities::TeamNamespace, ports::TeamNamespacePort};

use crate::{error::AppError, extractors::AuthIdentity};
use super::require_admin;

#[derive(Debug, Serialize, ToSchema)]
pub struct TeamNamespaceDto {
    pub registry: String,
    pub prefix: String,
    pub group_id: String,
    pub claimed_by: Option<String>,
}

impl From<TeamNamespace> for TeamNamespaceDto {
    fn from(ns: TeamNamespace) -> Self {
        Self {
            registry: ns.registry,
            prefix: ns.prefix,
            group_id: ns.group_id,
            claimed_by: ns.claimed_by,
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ClaimNamespaceRequest {
    pub prefix: String,
    pub group_id: String,
    pub claimed_by: Option<String>,
}

/// List team namespace claims for a registry.
#[utoipa::path(
    get,
    path = "/api/v1/admin/registries/{registry}/namespaces",
    tag = "back-office",
    params(("registry" = String, Path, description = "Registry name")),
    responses(
        (status = 200, description = "Namespace list"),
        (status = 403, description = "Admin role required"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/api/v1/admin/registries/{registry}/namespaces")]
pub async fn list_namespaces(
    path: web::Path<(String,)>,
    identity: AuthIdentity,
    store: web::Data<Arc<dyn TeamNamespacePort>>,
) -> Result<impl Responder, AppError> {
    require_admin(&identity)?;
    let (registry,) = path.into_inner();
    let namespaces: Vec<TeamNamespaceDto> = store
        .list_namespaces(&registry)
        .await
        .map_err(AppError::from)?
        .into_iter()
        .map(TeamNamespaceDto::from)
        .collect();
    Ok(HttpResponse::Ok().json(namespaces))
}

/// Claim a namespace prefix for a team group.
#[utoipa::path(
    post,
    path = "/api/v1/admin/registries/{registry}/namespaces",
    tag = "back-office",
    params(("registry" = String, Path, description = "Registry name")),
    request_body = ClaimNamespaceRequest,
    responses(
        (status = 204, description = "Namespace claimed"),
        (status = 403, description = "Admin role required"),
        (status = 409, description = "Prefix already claimed"),
    ),
    security(("bearer_token" = [])),
)]
#[post("/api/v1/admin/registries/{registry}/namespaces")]
pub async fn claim_namespace(
    path: web::Path<(String,)>,
    body: web::Json<ClaimNamespaceRequest>,
    identity: AuthIdentity,
    store: web::Data<Arc<dyn TeamNamespacePort>>,
) -> Result<impl Responder, AppError> {
    require_admin(&identity)?;
    let (registry,) = path.into_inner();
    if body.prefix.is_empty() {
        return Err(AppError::bad_request("prefix must not be empty"));
    }
    if body.group_id.is_empty() {
        return Err(AppError::bad_request("group_id must not be empty"));
    }
    store
        .claim_namespace(TeamNamespace {
            registry,
            prefix: body.prefix.clone(),
            group_id: body.group_id.clone(),
            claimed_by: body.claimed_by.clone(),
        })
        .await
        .map_err(AppError::from)?;
    Ok(HttpResponse::NoContent().finish())
}

/// Release a team namespace claim.
///
/// The prefix may contain slashes (e.g. `"frontend/libs"`).
#[utoipa::path(
    delete,
    path = "/api/v1/admin/registries/{registry}/namespaces/{prefix}",
    tag = "back-office",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("prefix"   = String, Path, description = "Namespace prefix (may contain slashes)"),
    ),
    responses(
        (status = 204, description = "Namespace released (or did not exist)"),
        (status = 403, description = "Admin role required"),
    ),
    security(("bearer_token" = [])),
)]
#[delete("/api/v1/admin/registries/{registry}/namespaces/{prefix:.*}")]
pub async fn release_namespace(
    path: web::Path<(String, String)>,
    identity: AuthIdentity,
    store: web::Data<Arc<dyn TeamNamespacePort>>,
) -> Result<impl Responder, AppError> {
    require_admin(&identity)?;
    let (registry, prefix) = path.into_inner();
    store
        .release_namespace(&registry, &prefix)
        .await
        .map_err(AppError::from)?;
    Ok(HttpResponse::NoContent().finish())
}
