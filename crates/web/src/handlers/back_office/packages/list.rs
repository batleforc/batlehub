use super::{
    default_per_page, get, post, require_admin, web, ActionResponse, AdminService, AppError, Arc,
    AuthIdentity, Deserialize, IntoParams, PackageFilter, PackageId, ProxyService, Responder,
    ToSchema,
};

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

// ── Block / unblock ───────────────────────────────────────────────────────────

#[derive(Deserialize, ToSchema)]
pub struct BlockRequest {
    pub registry: String,
    pub name: String,
    pub version: String,
    pub artifact: Option<String>,
    pub reason: String,
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
