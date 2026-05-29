use std::sync::Arc;

use actix_web::{delete, get, post, web, HttpResponse, Responder};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use batlehub_core::{entities::Role, ports::OwnerEntry, services::LocalRegistryService};

use crate::{error::AppError, extractors::AuthIdentity};

fn require_admin(identity: &AuthIdentity) -> Result<(), AppError> {
    if identity.role != Role::Admin {
        Err(AppError::forbidden("admin role required"))
    } else {
        Ok(())
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct OwnerEntryDto {
    pub principal_type: String,
    pub principal_id: String,
    pub role: String,
    pub granted_by: Option<String>,
}

impl From<OwnerEntry> for OwnerEntryDto {
    fn from(e: OwnerEntry) -> Self {
        Self {
            principal_type: e.principal_type,
            principal_id: e.principal_id,
            role: e.role,
            granted_by: e.granted_by,
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AddOwnerRequest {
    pub principal_type: String,
    pub principal_id: String,
    #[serde(default = "default_role")]
    pub role: String,
    pub granted_by: Option<String>,
}

fn default_role() -> String {
    "maintainer".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use batlehub_core::{entities::{Identity, Role}, ports::OwnerEntry};
    use crate::extractors::AuthIdentity;

    fn id(role: Role) -> AuthIdentity {
        AuthIdentity(Identity { user_id: Some("u".into()), role, auth_provider: None, groups: vec![] })
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

    #[test]
    fn default_role_is_maintainer() {
        assert_eq!(default_role(), "maintainer");
    }

    #[test]
    fn owner_entry_dto_conversion() {
        let entry = OwnerEntry {
            principal_type: "user".into(),
            principal_id: "alice".into(),
            role: "admin".into(),
            granted_by: Some("bob".into()),
        };
        let dto = OwnerEntryDto::from(entry);
        assert_eq!(dto.principal_type, "user");
        assert_eq!(dto.principal_id, "alice");
        assert_eq!(dto.role, "admin");
        assert_eq!(dto.granted_by.as_deref(), Some("bob"));
    }

    #[test]
    fn owner_entry_dto_none_granted_by() {
        let entry = OwnerEntry {
            principal_type: "group".into(),
            principal_id: "devs".into(),
            role: "maintainer".into(),
            granted_by: None,
        };
        let dto = OwnerEntryDto::from(entry);
        assert!(dto.granted_by.is_none());
    }
}

/// List owners of a package.
#[utoipa::path(
    get,
    path = "/api/v1/admin/registries/{registry}/packages/{name}/owners",
    tag = "back-office",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("name" = String, Path, description = "Package name"),
    ),
    responses(
        (status = 200, description = "Owner list"),
        (status = 403, description = "Admin role required"),
        (status = 503, description = "Ownership not configured"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/api/v1/admin/registries/{registry}/packages/{name}/owners")]
pub async fn list_package_owners(
    path: web::Path<(String, String)>,
    identity: AuthIdentity,
    local_svc: web::Data<Arc<LocalRegistryService>>,
) -> Result<impl Responder, AppError> {
    require_admin(&identity)?;
    let (registry, name) = path.into_inner();
    let ownership = local_svc
        .ownership
        .as_ref()
        .ok_or_else(|| AppError::service_unavailable("ownership not configured"))?;
    let owners: Vec<OwnerEntryDto> = ownership
        .list_owners(&registry, &name)
        .await
        .map_err(AppError::from)?
        .into_iter()
        .map(OwnerEntryDto::from)
        .collect();
    Ok(HttpResponse::Ok().json(owners))
}

/// Add an owner to a package.
#[utoipa::path(
    post,
    path = "/api/v1/admin/registries/{registry}/packages/{name}/owners",
    tag = "back-office",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("name" = String, Path, description = "Package name"),
    ),
    request_body = AddOwnerRequest,
    responses(
        (status = 204, description = "Owner added"),
        (status = 403, description = "Admin role required"),
        (status = 409, description = "Already an owner"),
        (status = 503, description = "Ownership not configured"),
    ),
    security(("bearer_token" = [])),
)]
#[post("/api/v1/admin/registries/{registry}/packages/{name}/owners")]
pub async fn add_package_owner(
    path: web::Path<(String, String)>,
    body: web::Json<AddOwnerRequest>,
    identity: AuthIdentity,
    local_svc: web::Data<Arc<LocalRegistryService>>,
) -> Result<impl Responder, AppError> {
    require_admin(&identity)?;
    let (registry, name) = path.into_inner();
    let ownership = local_svc
        .ownership
        .as_ref()
        .ok_or_else(|| AppError::service_unavailable("ownership not configured"))?;
    ownership
        .add_owner(
            &registry,
            &name,
            OwnerEntry {
                principal_type: body.principal_type.clone(),
                principal_id: body.principal_id.clone(),
                role: body.role.clone(),
                granted_by: body.granted_by.clone(),
            },
        )
        .await
        .map_err(AppError::from)?;
    Ok(HttpResponse::NoContent().finish())
}

/// Remove an owner from a package.
#[utoipa::path(
    delete,
    path = "/api/v1/admin/registries/{registry}/packages/{name}/owners/{principal_type}/{principal_id}",
    tag = "back-office",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("name" = String, Path, description = "Package name"),
        ("principal_type" = String, Path, description = "\"user\" or \"group\""),
        ("principal_id" = String, Path, description = "User ID or group name"),
    ),
    responses(
        (status = 204, description = "Owner removed"),
        (status = 403, description = "Admin role required"),
        (status = 503, description = "Ownership not configured"),
    ),
    security(("bearer_token" = [])),
)]
#[delete(
    "/api/v1/admin/registries/{registry}/packages/{name}/owners/{principal_type}/{principal_id}"
)]
pub async fn remove_package_owner(
    path: web::Path<(String, String, String, String)>,
    identity: AuthIdentity,
    local_svc: web::Data<Arc<LocalRegistryService>>,
) -> Result<impl Responder, AppError> {
    require_admin(&identity)?;
    let (registry, name, principal_type, principal_id) = path.into_inner();
    let ownership = local_svc
        .ownership
        .as_ref()
        .ok_or_else(|| AppError::service_unavailable("ownership not configured"))?;
    ownership
        .remove_owner(&registry, &name, &principal_type, &principal_id)
        .await
        .map_err(AppError::from)?;
    Ok(HttpResponse::NoContent().finish())
}
