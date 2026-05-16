use std::sync::Arc;

use actix_web::{Responder, get, post, web};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use proxy_cache_core::{
    entities::{AccessAction, AccessResult, EventFilter, PackageFilter, PackageId, PackageStatus, Role},
    services::{AdminService, ProxyService},
};

use crate::{error::AppError, extractors::AuthIdentity};

fn require_admin(identity: &AuthIdentity) -> Result<(), AppError> {
    if identity.role != Role::Admin {
        Err(AppError::forbidden("admin role required"))
    } else {
        Ok(())
    }
}

// ── List all packages ─────────────────────────────────────────────────────────

#[derive(Deserialize, IntoParams)]
pub struct AdminPackageQuery {
    pub registry: Option<String>,
    pub name: Option<String>,
    pub blocked_only: Option<bool>,
    #[serde(default)]
    pub page: u64,
    #[serde(default = "default_per_page")]
    pub per_page: u64,
}

fn default_per_page() -> u64 {
    50
}

/// List all known packages (admin).
#[utoipa::path(
    get,
    path = "/api/v1/admin/packages",
    tag = "back-office",
    params(AdminPackageQuery),
    responses(
        (status = 200, description = "Full package listing with statuses"),
        (status = 403, description = "Admin role required"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/api/v1/admin/packages")]
pub async fn list_packages(
    query: web::Query<AdminPackageQuery>,
    identity: AuthIdentity,
    admin_svc: web::Data<Arc<AdminService>>,
) -> Result<impl Responder, AppError> {
    require_admin(&identity)?;

    let filter = PackageFilter {
        registry: query.registry.clone(),
        name_contains: query.name.clone(),
        name_exact: None,
        blocked_only: query.blocked_only.unwrap_or(false),
        limit: query.per_page,
        offset: query.page * query.per_page,
    };

    let packages = admin_svc.list_packages(filter).await.map_err(AppError::from)?;
    Ok(web::Json(packages))
}

// ── Block / unblock ───────────────────────────────────────────────────────────

#[derive(Deserialize, ToSchema)]
pub struct BlockRequest {
    pub registry: String,
    pub name: String,
    pub version: String,
    pub artifact: Option<String>,
    pub reason: String,
}

#[derive(Serialize, ToSchema)]
pub struct ActionResponse {
    pub success: bool,
    pub message: String,
}

/// Block a package (admin).
#[utoipa::path(
    post,
    path = "/api/v1/admin/packages/block",
    tag = "back-office",
    request_body = BlockRequest,
    responses(
        (status = 200, description = "Package blocked", body = ActionResponse),
        (status = 403, description = "Admin role required"),
    ),
    security(("bearer_token" = [])),
)]
#[post("/api/v1/admin/packages/block")]
pub async fn block_package(
    identity: AuthIdentity,
    body: web::Json<BlockRequest>,
    admin_svc: web::Data<Arc<AdminService>>,
) -> Result<impl Responder, AppError> {
    require_admin(&identity)?;

    let pkg = PackageId {
        registry: body.registry.clone(),
        name: body.name.clone(),
        version: body.version.clone(),
        artifact: body.artifact.clone(),
    };

    admin_svc
        .block_package(&pkg, body.reason.clone(), &identity.0)
        .await
        .map_err(AppError::from)?;

    Ok(web::Json(ActionResponse {
        success: true,
        message: format!("package '{}' has been blocked", pkg),
    }))
}

#[derive(Deserialize, ToSchema)]
pub struct UnblockRequest {
    pub registry: String,
    pub name: String,
    pub version: String,
    pub artifact: Option<String>,
}

/// Unblock a package (admin).
#[utoipa::path(
    post,
    path = "/api/v1/admin/packages/unblock",
    tag = "back-office",
    request_body = UnblockRequest,
    responses(
        (status = 200, description = "Package unblocked", body = ActionResponse),
        (status = 403, description = "Admin role required"),
    ),
    security(("bearer_token" = [])),
)]
#[post("/api/v1/admin/packages/unblock")]
pub async fn unblock_package(
    identity: AuthIdentity,
    body: web::Json<UnblockRequest>,
    admin_svc: web::Data<Arc<AdminService>>,
) -> Result<impl Responder, AppError> {
    require_admin(&identity)?;

    let pkg = PackageId {
        registry: body.registry.clone(),
        name: body.name.clone(),
        version: body.version.clone(),
        artifact: body.artifact.clone(),
    };

    admin_svc
        .unblock_package(&pkg, &identity.0)
        .await
        .map_err(AppError::from)?;

    Ok(web::Json(ActionResponse {
        success: true,
        message: format!("package '{}' has been unblocked", pkg),
    }))
}

// ── Package detail ────────────────────────────────────────────────────────────

#[derive(Deserialize, IntoParams)]
pub struct PackageDetailQuery {
    pub registry: String,
    pub name: String,
}

#[derive(Serialize, ToSchema)]
pub struct PackageVersionDetail {
    pub id: Uuid,
    pub version: String,
    pub artifact: Option<String>,
    pub status: PackageStatusDetail,
    pub storage_key: String,
    pub cached: bool,
    pub access_count: u64,
    pub last_accessed: Option<DateTime<Utc>>,
    pub last_accessed_by: Option<String>,
}

#[derive(Serialize, ToSchema)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum PackageStatusDetail {
    Available,
    Blocked { reason: String, blocked_by: String, blocked_at: DateTime<Utc> },
}

#[derive(Serialize, ToSchema)]
pub struct PackageEventDto {
    pub id: Uuid,
    pub user_id: Option<String>,
    pub user_role: String,
    pub version: String,
    pub artifact: Option<String>,
    pub action: String,
    pub outcome: String,
    pub deny_reason: Option<String>,
    pub timestamp: DateTime<Utc>,
}

#[derive(Serialize, ToSchema)]
pub struct PackageDetailResponse {
    pub registry: String,
    pub name: String,
    pub versions: Vec<PackageVersionDetail>,
    pub recent_events: Vec<PackageEventDto>,
}

/// Get detailed information about a specific package (all versions, access history, cache status).
#[utoipa::path(
    get,
    path = "/api/v1/admin/packages/detail",
    tag = "back-office",
    params(PackageDetailQuery),
    responses(
        (status = 200, description = "Package detail", body = PackageDetailResponse),
        (status = 403, description = "Admin role required"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/api/v1/admin/packages/detail")]
pub async fn package_detail(
    query: web::Query<PackageDetailQuery>,
    identity: AuthIdentity,
    admin_svc: web::Data<Arc<AdminService>>,
    proxy_svc: web::Data<Arc<ProxyService>>,
) -> Result<impl Responder, AppError> {
    require_admin(&identity)?;

    let filter = PackageFilter {
        registry: Some(query.registry.clone()),
        name_exact: Some(query.name.clone()),
        name_contains: None,
        blocked_only: false,
        limit: 200,
        offset: 0,
    };
    let summaries = admin_svc.list_packages(filter).await.map_err(AppError::from)?;

    let mut versions = Vec::with_capacity(summaries.len());
    for s in summaries {
        let storage_key = format!("artifact:{}", s.package_id.cache_key());
        let cached = proxy_svc.storage.exists(&storage_key).await.unwrap_or(false);
        let status = match s.status {
            PackageStatus::Available => PackageStatusDetail::Available,
            PackageStatus::Blocked { reason, blocked_by, blocked_at } => {
                PackageStatusDetail::Blocked { reason, blocked_by, blocked_at }
            }
        };
        versions.push(PackageVersionDetail {
            id: s.id,
            version: s.package_id.version,
            artifact: s.package_id.artifact,
            status,
            storage_key,
            cached,
            access_count: s.access_count,
            last_accessed: s.last_accessed,
            last_accessed_by: s.last_accessed_by,
        });
    }

    let event_filter = EventFilter {
        registry: Some(query.registry.clone()),
        package_name: Some(query.name.clone()),
        user_id: None,
        from: None,
        to: None,
        denied_only: false,
        limit: 50,
        offset: 0,
    };
    let events = admin_svc.list_events(event_filter).await.map_err(AppError::from)?;

    let recent_events = events
        .into_iter()
        .map(|e| {
            let (outcome, deny_reason) = match e.result {
                AccessResult::Allowed => ("allowed".to_string(), None),
                AccessResult::Denied { reason } => ("denied".to_string(), Some(reason)),
            };
            let action = match e.action {
                AccessAction::Download => "download",
                AccessAction::ViewMetadata => "view_metadata",
                AccessAction::Block => "block",
                AccessAction::Unblock => "unblock",
            };
            PackageEventDto {
                id: e.id,
                user_id: e.user_id,
                user_role: e.user_role.to_string(),
                version: e.package_id.version,
                artifact: e.package_id.artifact,
                action: action.to_string(),
                outcome,
                deny_reason,
                timestamp: e.timestamp,
            }
        })
        .collect();

    Ok(web::Json(PackageDetailResponse {
        registry: query.registry.clone(),
        name: query.name.clone(),
        versions,
        recent_events,
    }))
}
