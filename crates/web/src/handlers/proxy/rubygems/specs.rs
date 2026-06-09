use super::{
    get, proxy_stream, require_registry_type, web, AppError, Arc, AuthIdentity, PackageId,
    ProxyService, RegistryMap, RegistryMode, RegistryModeMap, Responder,
};

/// Serve the full gem index (specs.4.8.gz).
#[utoipa::path(
    get,
    path = "/proxy/{registry}/specs.4.8.gz",
    tag = "proxy/rubygems",
    params(("registry" = String, Path, description = "Registry name")),
    responses(
        (status = 200, description = "Gzip-compressed binary specs index"),
        (status = 404, description = "Not available for local-only registries"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/specs.4.8.gz")]
pub async fn gem_specs_full(
    path: web::Path<String>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
) -> Result<impl Responder, AppError> {
    let registry = path.into_inner();
    require_registry_type(&registry, "rubygems", &map)?;

    if mode_map.get(&registry) == RegistryMode::Local {
        return Err(AppError::not_found(
            "binary specs index is not available for local-only registries; use /api/v1/versions/{name}.json".to_owned(),
        ));
    }

    let pkg = PackageId::new(&registry, "_index", "specs");
    proxy_stream(
        svc,
        pkg,
        identity,
        "releases:read",
        Some("application/octet-stream"),
    )
    .await
}

/// Serve the latest-versions gem index (latest_specs.4.8.gz).
#[utoipa::path(
    get,
    path = "/proxy/{registry}/latest_specs.4.8.gz",
    tag = "proxy/rubygems",
    params(("registry" = String, Path, description = "Registry name")),
    responses(
        (status = 200, description = "Gzip-compressed binary latest-specs index"),
        (status = 404, description = "Not available for local-only registries"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/latest_specs.4.8.gz")]
pub async fn gem_specs_latest(
    path: web::Path<String>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
) -> Result<impl Responder, AppError> {
    let registry = path.into_inner();
    require_registry_type(&registry, "rubygems", &map)?;

    if mode_map.get(&registry) == RegistryMode::Local {
        return Err(AppError::not_found(
            "binary specs index is not available for local-only registries; use /api/v1/versions/{name}.json".to_owned(),
        ));
    }

    let pkg = PackageId::new(&registry, "_index", "latest_specs");
    proxy_stream(
        svc,
        pkg,
        identity,
        "releases:read",
        Some("application/octet-stream"),
    )
    .await
}

/// Serve the prerelease gem index (prerelease_specs.4.8.gz).
#[utoipa::path(
    get,
    path = "/proxy/{registry}/prerelease_specs.4.8.gz",
    tag = "proxy/rubygems",
    params(("registry" = String, Path, description = "Registry name")),
    responses(
        (status = 200, description = "Gzip-compressed binary prerelease-specs index"),
        (status = 404, description = "Not available for local-only registries"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/prerelease_specs.4.8.gz")]
pub async fn gem_specs_prerelease(
    path: web::Path<String>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
) -> Result<impl Responder, AppError> {
    let registry = path.into_inner();
    require_registry_type(&registry, "rubygems", &map)?;

    if mode_map.get(&registry) == RegistryMode::Local {
        return Err(AppError::not_found(
            "binary specs index is not available for local-only registries; use /api/v1/versions/{name}.json".to_owned(),
        ));
    }

    let pkg = PackageId::new(&registry, "_index", "prerelease_specs");
    proxy_stream(
        svc,
        pkg,
        identity,
        "releases:read",
        Some("application/octet-stream"),
    )
    .await
}
