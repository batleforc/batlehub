use std::sync::Arc;

use actix_web::{HttpResponse, Responder, delete, get, post, web};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use batlehub_core::{entities::{Role, TeamNamespace}, ports::TeamNamespacePort};

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
    let group_id = body.group_id.replace(' ', "");
    store
        .claim_namespace(TeamNamespace {
            registry,
            prefix: body.prefix.clone(),
            group_id,
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

// ── User-facing endpoints ────────────────────────────────────────────────────

/// List all namespace claims owned by the caller's groups (across all registries).
#[utoipa::path(
    get,
    path = "/api/v1/me/namespaces",
    tag = "user",
    responses(
        (status = 200, description = "Namespaces owned by the caller's groups"),
        (status = 403, description = "Authentication required"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/api/v1/me/namespaces")]
pub async fn my_namespaces(
    identity: AuthIdentity,
    store: web::Data<Arc<dyn TeamNamespacePort>>,
) -> Result<impl Responder, AppError> {
    if !identity.has_role_at_least(&Role::User) {
        return Err(AppError::forbidden("authentication required"));
    }
    let normalized_groups: Vec<String> = identity.groups.iter().map(|g| g.replace(' ', "")).collect();
    let namespaces: Vec<TeamNamespaceDto> = store
        .list_namespaces_for_groups(&normalized_groups)
        .await
        .map_err(AppError::from)?
        .into_iter()
        .map(TeamNamespaceDto::from)
        .collect();
    Ok(HttpResponse::Ok().json(namespaces))
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct NamespacePackagesQuery {
    #[serde(default)]
    pub page: u64,
    #[serde(default = "default_per_page")]
    pub per_page: u64,
}

fn default_per_page() -> u64 {
    50
}

#[derive(Debug, Serialize, ToSchema)]
pub struct NamespacePackageDto {
    pub name: String,
    pub version: String,
    pub visibility: String,
    pub published_by: String,
    pub published_at: DateTime<Utc>,
    pub yanked: bool,
}

/// List published packages under a namespace prefix.
///
/// Accessible by admins and members of the group owning the namespace prefix.
#[utoipa::path(
    get,
    path = "/api/v1/me/namespaces/{registry}/{prefix}/packages",
    tag = "user",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("prefix"   = String, Path, description = "Namespace prefix (may contain slashes)"),
        NamespacePackagesQuery,
    ),
    responses(
        (status = 200, description = "Package list"),
        (status = 403, description = "Authentication or group membership required"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/api/v1/me/namespaces/{registry}/{prefix:.*}/packages")]
pub async fn my_namespace_packages(
    path: web::Path<(String, String)>,
    query: web::Query<NamespacePackagesQuery>,
    identity: AuthIdentity,
    store: web::Data<Arc<dyn TeamNamespacePort>>,
) -> Result<impl Responder, AppError> {
    if !identity.has_role_at_least(&Role::User) {
        return Err(AppError::forbidden("authentication required"));
    }
    let (registry, prefix) = path.into_inner();

    // Admins can query any namespace; regular users must be in the owning group.
    if identity.role != Role::Admin {
        let ns = store
            .find_namespace(&registry, &prefix)
            .await
            .map_err(AppError::from)?;
        match ns {
            Some(ns) if identity.groups.iter().any(|g| g.replace(' ', "") == ns.group_id.replace(' ', "")) => {}
            Some(ns) => {
                return Err(AppError::forbidden(format!(
                    "namespace '{}' is owned by group '{}'; you are not a member",
                    ns.prefix, ns.group_id
                )));
            }
            None => return Err(AppError::forbidden("admin role required")),
        }
    }

    let limit = query.per_page;
    let offset = query.page * query.per_page;
    let packages: Vec<NamespacePackageDto> = store
        .list_packages_in_namespace(&registry, &prefix, limit, offset)
        .await
        .map_err(AppError::from)?
        .into_iter()
        .map(|p| NamespacePackageDto {
            name: p.name,
            version: p.version,
            visibility: p.visibility.to_string(),
            published_by: p.published_by,
            published_at: p.published_at,
            yanked: p.yanked,
        })
        .collect();
    Ok(HttpResponse::Ok().json(packages))
}
