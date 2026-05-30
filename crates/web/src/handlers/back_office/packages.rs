use std::sync::Arc;

use actix_web::{get, post, web, Responder};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use batlehub_core::{
    entities::{AccessAction, AccessResult, EventFilter, PackageFilter, PackageId, PackageStatus},
    services::{AdminService, BulkBlockItem, ProxyService},
};

use super::require_admin;
use crate::{error::AppError, extractors::AuthIdentity};

// ── List all packages ─────────────────────────────────────────────────────────

#[derive(Deserialize, IntoParams)]
pub struct AdminPackageQuery {
    pub registry: Option<String>,
    pub name: Option<String>,
    #[serde(default)]
    pub blocked_only: bool,
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
        registries: vec![],
        name_contains: query.name.clone(),
        name_exact: None,
        blocked_only: query.blocked_only,
        limit: query.per_page,
        offset: query.page * query.per_page,
    };

    let packages = admin_svc
        .list_packages(filter)
        .await
        .map_err(AppError::from)?;
    Ok(web::Json(packages))
}

fn map_bulk_failures(failed: Vec<(PackageId, String)>) -> Vec<BulkFailureDto> {
    failed
        .into_iter()
        .map(|(pkg, error)| BulkFailureDto {
            registry: pkg.registry,
            name: pkg.name,
            version: pkg.version,
            artifact: pkg.artifact,
            error,
        })
        .collect()
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

// ── Bulk block / unblock ──────────────────────────────────────────────────────

#[derive(Deserialize, ToSchema)]
pub struct BulkBlockRequestItem {
    pub registry: String,
    pub name: String,
    pub version: String,
    pub artifact: Option<String>,
    pub reason: String,
}

#[derive(Deserialize, ToSchema)]
pub struct BulkBlockRequest {
    pub items: Vec<BulkBlockRequestItem>,
}

#[derive(Deserialize, ToSchema)]
pub struct BulkUnblockRequestItem {
    pub registry: String,
    pub name: String,
    pub version: String,
    pub artifact: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct BulkUnblockRequest {
    pub items: Vec<BulkUnblockRequestItem>,
}

#[derive(Serialize, ToSchema)]
pub struct BulkFailureDto {
    pub registry: String,
    pub name: String,
    pub version: String,
    pub artifact: Option<String>,
    pub error: String,
}

#[derive(Serialize, ToSchema)]
pub struct BulkActionResponse {
    pub succeeded_count: usize,
    pub failed_count: usize,
    pub failures: Vec<BulkFailureDto>,
}

/// Bulk-block packages (admin).
#[utoipa::path(
    post,
    path = "/api/v1/admin/packages/bulk-block",
    tag = "back-office",
    request_body = BulkBlockRequest,
    responses(
        (status = 200, description = "Bulk block result", body = BulkActionResponse),
        (status = 403, description = "Admin role required"),
    ),
    security(("bearer_token" = [])),
)]
#[post("/api/v1/admin/packages/bulk-block")]
pub async fn bulk_block_packages(
    identity: AuthIdentity,
    body: web::Json<BulkBlockRequest>,
    admin_svc: web::Data<Arc<AdminService>>,
) -> Result<impl Responder, AppError> {
    require_admin(&identity)?;

    let items = body
        .into_inner()
        .items
        .into_iter()
        .map(|i| BulkBlockItem {
            package_id: PackageId {
                registry: i.registry,
                name: i.name,
                version: i.version,
                artifact: i.artifact,
            },
            reason: i.reason,
        })
        .collect();

    let result = admin_svc.bulk_block_packages(items, &identity.0).await;

    Ok(web::Json(BulkActionResponse {
        succeeded_count: result.succeeded.len(),
        failed_count: result.failed.len(),
        failures: map_bulk_failures(result.failed),
    }))
}

/// Bulk-unblock packages (admin).
#[utoipa::path(
    post,
    path = "/api/v1/admin/packages/bulk-unblock",
    tag = "back-office",
    request_body = BulkUnblockRequest,
    responses(
        (status = 200, description = "Bulk unblock result", body = BulkActionResponse),
        (status = 403, description = "Admin role required"),
    ),
    security(("bearer_token" = [])),
)]
#[post("/api/v1/admin/packages/bulk-unblock")]
pub async fn bulk_unblock_packages(
    identity: AuthIdentity,
    body: web::Json<BulkUnblockRequest>,
    admin_svc: web::Data<Arc<AdminService>>,
) -> Result<impl Responder, AppError> {
    require_admin(&identity)?;

    let items = body
        .into_inner()
        .items
        .into_iter()
        .map(|i| PackageId {
            registry: i.registry,
            name: i.name,
            version: i.version,
            artifact: i.artifact,
        })
        .collect();

    let result = admin_svc.bulk_unblock_packages(items, &identity.0).await;

    Ok(web::Json(BulkActionResponse {
        succeeded_count: result.succeeded.len(),
        failed_count: result.failed.len(),
        failures: map_bulk_failures(result.failed),
    }))
}

// ── Cache invalidation ────────────────────────────────────────────────────────

#[derive(Deserialize, ToSchema)]
pub struct InvalidateRequest {
    pub registry: String,
    pub name: String,
    pub version: String,
    pub artifact: Option<String>,
}

/// Purge the cached artifact for a specific package version (admin).
///
/// Deletes the artifact from storage and clears the in-memory metadata cache.
/// The package block/unblock status is not changed.
#[utoipa::path(
    post,
    path = "/api/v1/admin/packages/invalidate",
    tag = "back-office",
    request_body = InvalidateRequest,
    responses(
        (status = 200, description = "Cache purged", body = ActionResponse),
        (status = 403, description = "Admin role required"),
    ),
    security(("bearer_token" = [])),
)]
#[post("/api/v1/admin/packages/invalidate")]
pub async fn invalidate_package(
    identity: AuthIdentity,
    body: web::Json<InvalidateRequest>,
    proxy_svc: web::Data<Arc<ProxyService>>,
) -> Result<impl Responder, AppError> {
    require_admin(&identity)?;

    let pkg = PackageId {
        registry: body.registry.clone(),
        name: body.name.clone(),
        version: body.version.clone(),
        artifact: body.artifact.clone(),
    };

    let storage_key = format!("artifact:{}", pkg.cache_key());
    let meta_key = format!("meta:{}", pkg.cache_key());

    proxy_svc
        .storage
        .delete(&storage_key)
        .await
        .map_err(AppError::from)?;
    proxy_svc
        .cache
        .invalidate(&meta_key)
        .await
        .map_err(AppError::from)?;

    Ok(web::Json(ActionResponse {
        success: true,
        message: format!("cache purged for '{}'", pkg),
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
    /// Name of the storage backend holding this artifact (null if not yet cached or pre-migration).
    pub storage_backend: Option<String>,
    /// When the artifact was first stored in the cache (null if not yet cached or pre-migration).
    pub cached_at: Option<DateTime<Utc>>,
    pub access_count: u64,
    pub last_accessed: Option<DateTime<Utc>>,
    pub last_accessed_by: Option<String>,
}

#[derive(Serialize, ToSchema)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum PackageStatusDetail {
    Available,
    Blocked {
        reason: String,
        blocked_by: String,
        blocked_at: DateTime<Utc>,
    },
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
    pool: Option<web::Data<PgPool>>,
) -> Result<impl Responder, AppError> {
    require_admin(&identity)?;

    let filter = PackageFilter {
        registry: Some(query.registry.clone()),
        registries: vec![],
        name_exact: Some(query.name.clone()),
        name_contains: None,
        blocked_only: false,
        limit: 200,
        offset: 0,
    };
    let summaries = admin_svc
        .list_packages(filter)
        .await
        .map_err(AppError::from)?;

    let mut versions = Vec::with_capacity(summaries.len());
    for s in summaries {
        let storage_key = format!("artifact:{}", s.package_id.cache_key());
        let cached = proxy_svc
            .storage
            .exists(&storage_key)
            .await
            .unwrap_or(false);
        let (storage_backend, cached_at) = if let Some(ref p) = pool {
            let row = sqlx::query(
                "SELECT backend_name, stored_at FROM artifact_storage WHERE storage_key = $1",
            )
            .bind(&storage_key)
            .fetch_optional(p.get_ref())
            .await
            .ok()
            .flatten();
            let backend = row
                .as_ref()
                .and_then(|r| r.try_get::<String, _>("backend_name").ok());
            let at = row.and_then(|r| r.try_get::<DateTime<Utc>, _>("stored_at").ok());
            (backend, at)
        } else {
            (None, None)
        };
        let status = match s.status {
            PackageStatus::Available => PackageStatusDetail::Available,
            PackageStatus::Blocked {
                reason,
                blocked_by,
                blocked_at,
            } => PackageStatusDetail::Blocked {
                reason,
                blocked_by,
                blocked_at,
            },
        };
        versions.push(PackageVersionDetail {
            id: s.id,
            version: s.package_id.version,
            artifact: s.package_id.artifact,
            status,
            storage_key,
            cached,
            storage_backend,
            cached_at,
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
    let events = admin_svc
        .list_events(event_filter)
        .await
        .map_err(AppError::from)?;

    let recent_events = events
        .into_iter()
        .map(|e| {
            let (outcome, deny_reason) = match e.result {
                AccessResult::Allowed => ("allowed".to_string(), None),
                AccessResult::Denied { reason } => ("denied".to_string(), Some(reason)),
                AccessResult::ProxyError { reason } => ("error".to_string(), Some(reason)),
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
