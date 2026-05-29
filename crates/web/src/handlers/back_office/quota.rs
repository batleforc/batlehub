use std::sync::Arc;

use actix_web::{delete, get, web, HttpResponse, Responder};
use serde::Serialize;
use utoipa::ToSchema;

use batlehub_core::{entities::Role, ports::QuotaUsage, services::QuotaService};

use crate::{error::AppError, extractors::AuthIdentity};

fn require_admin(identity: &AuthIdentity) -> Result<(), AppError> {
    if identity.role != Role::Admin {
        Err(AppError::forbidden("admin role required"))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use batlehub_core::{entities::{Identity, Role}, ports::QuotaUsage};
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
    }

    #[test]
    fn quota_usage_dto_conversion() {
        let usage = QuotaUsage {
            user_id: "alice".into(),
            registry: "cargo".into(),
            bytes_published: 1024,
            packages_count: 3,
        };
        let dto = QuotaUsageDto::from(usage);
        assert_eq!(dto.user_id, "alice");
        assert_eq!(dto.registry, "cargo");
        assert_eq!(dto.bytes_published, 1024);
        assert_eq!(dto.packages_count, 3);
    }
}

#[derive(Serialize, ToSchema)]
pub struct QuotaUsageDto {
    pub user_id: String,
    pub registry: String,
    pub bytes_published: u64,
    pub packages_count: u32,
}

impl From<QuotaUsage> for QuotaUsageDto {
    fn from(u: QuotaUsage) -> Self {
        Self {
            user_id: u.user_id,
            registry: u.registry,
            bytes_published: u.bytes_published,
            packages_count: u.packages_count,
        }
    }
}

/// List quota usage for all users across all registries.
#[utoipa::path(
    get,
    path = "/api/v1/admin/quota",
    tag = "back-office",
    responses(
        (status = 200, description = "All quota usage rows"),
        (status = 403, description = "Admin role required"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/api/v1/admin/quota")]
pub async fn list_quota(
    identity: AuthIdentity,
    quota_svc: web::Data<Arc<QuotaService>>,
) -> Result<impl Responder, AppError> {
    require_admin(&identity)?;
    let rows = quota_svc.list_usage(None).await.map_err(AppError::from)?;
    let dtos: Vec<QuotaUsageDto> = rows.into_iter().map(Into::into).collect();
    Ok(HttpResponse::Ok().json(dtos))
}

/// List quota usage for all users in a specific registry.
#[utoipa::path(
    get,
    path = "/api/v1/admin/quota/{registry}",
    tag = "back-office",
    params(("registry" = String, Path, description = "Registry name")),
    responses(
        (status = 200, description = "Quota usage rows for registry"),
        (status = 403, description = "Admin role required"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/api/v1/admin/quota/{registry}")]
pub async fn list_quota_for_registry(
    path: web::Path<String>,
    identity: AuthIdentity,
    quota_svc: web::Data<Arc<QuotaService>>,
) -> Result<impl Responder, AppError> {
    require_admin(&identity)?;
    let registry = path.into_inner();
    let rows = quota_svc
        .list_usage(Some(&registry))
        .await
        .map_err(AppError::from)?;
    let dtos: Vec<QuotaUsageDto> = rows.into_iter().map(Into::into).collect();
    Ok(HttpResponse::Ok().json(dtos))
}

/// Get quota usage for a specific user in a registry.
#[utoipa::path(
    get,
    path = "/api/v1/admin/quota/{registry}/{user_id}",
    tag = "back-office",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("user_id"  = String, Path, description = "User identifier"),
    ),
    responses(
        (status = 200, description = "Quota usage for the user"),
        (status = 403, description = "Admin role required"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/api/v1/admin/quota/{registry}/{user_id}")]
pub async fn get_quota_for_user(
    path: web::Path<(String, String)>,
    identity: AuthIdentity,
    quota_svc: web::Data<Arc<QuotaService>>,
) -> Result<impl Responder, AppError> {
    require_admin(&identity)?;
    let (registry, user_id) = path.into_inner();
    let usage = quota_svc
        .get_usage(&user_id, &registry)
        .await
        .map_err(AppError::from)?;
    Ok(HttpResponse::Ok().json(QuotaUsageDto::from(usage)))
}

/// Reset quota usage for a specific user in a registry.
#[utoipa::path(
    delete,
    path = "/api/v1/admin/quota/{registry}/{user_id}",
    tag = "back-office",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("user_id"  = String, Path, description = "User identifier"),
    ),
    responses(
        (status = 200, description = "Quota reset"),
        (status = 403, description = "Admin role required"),
    ),
    security(("bearer_token" = [])),
)]
#[delete("/api/v1/admin/quota/{registry}/{user_id}")]
pub async fn reset_quota_for_user(
    path: web::Path<(String, String)>,
    identity: AuthIdentity,
    quota_svc: web::Data<Arc<QuotaService>>,
) -> Result<impl Responder, AppError> {
    require_admin(&identity)?;
    let (registry, user_id) = path.into_inner();
    quota_svc
        .reset(&user_id, &registry)
        .await
        .map_err(AppError::from)?;
    Ok(HttpResponse::Ok().json(serde_json::json!({ "ok": true })))
}
