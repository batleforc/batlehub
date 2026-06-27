use super::{
    map_bulk_failures, post, require_admin, web, AdminService, AppError, Arc, AuthIdentity,
    BulkActionResponse, BulkBlockItem, Deserialize, PackageId, ProxyService, Responder, ToSchema,
};

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

// ── Bulk delete ───────────────────────────────────────────────────────────────

#[derive(Deserialize, ToSchema)]
pub struct BulkDeleteRequestItem {
    pub registry: String,
    pub name: String,
    pub version: String,
    pub artifact: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct BulkDeleteRequest {
    pub items: Vec<BulkDeleteRequestItem>,
}

/// Bulk-delete package records and purge their cached artifacts (admin).
#[utoipa::path(
    post,
    path = "/api/v1/admin/packages/bulk-delete",
    tag = "back-office",
    request_body = BulkDeleteRequest,
    responses(
        (status = 200, description = "Bulk delete result", body = BulkActionResponse),
        (status = 403, description = "Admin role required"),
    ),
    security(("bearer_token" = [])),
)]
#[post("/api/v1/admin/packages/bulk-delete")]
pub async fn bulk_delete_packages(
    identity: AuthIdentity,
    body: web::Json<BulkDeleteRequest>,
    admin_svc: web::Data<Arc<AdminService>>,
    proxy_svc: web::Data<Arc<ProxyService>>,
) -> Result<impl Responder, AppError> {
    require_admin(&identity)?;

    let items: Vec<PackageId> = body
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

    let result = admin_svc.bulk_delete_packages(items, &identity.0).await;

    // Best-effort: purge cached artifacts only for packages successfully removed from the DB.
    for pkg in &result.succeeded {
        let storage_key = format!("artifact:{}", pkg.cache_key());
        let meta_key = format!("meta:{}", pkg.cache_key());
        let _ = proxy_svc.storage.delete(&storage_key).await;
        let _ = proxy_svc.artifact_meta.delete_artifact_meta(&storage_key).await;
        let _ = proxy_svc.cache.invalidate(&meta_key).await;
    }

    Ok(web::Json(BulkActionResponse {
        succeeded_count: result.succeeded.len(),
        failed_count: result.failed.len(),
        failures: map_bulk_failures(result.failed),
    }))
}
