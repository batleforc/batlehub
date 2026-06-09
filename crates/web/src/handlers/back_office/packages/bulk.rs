use super::{
    map_bulk_failures, post, require_admin, web, AdminService, AppError, Arc, AuthIdentity,
    BulkActionResponse, BulkBlockItem, Deserialize, PackageId, Responder, ToSchema,
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
