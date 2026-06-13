use super::{
    get, proxy_gem_specs, web, AppError, Arc, AuthIdentity, ProxyService, RegistryMap,
    RegistryModeMap, Responder,
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
    proxy_gem_specs(&registry, "specs", svc, identity, &map, &mode_map).await
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
    proxy_gem_specs(&registry, "latest_specs", svc, identity, &map, &mode_map).await
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
    proxy_gem_specs(
        &registry,
        "prerelease_specs",
        svc,
        identity,
        &map,
        &mode_map,
    )
    .await
}
