use std::sync::Arc;

use actix_web::{HttpResponse, Responder, get, post, web};

use batlehub_core::{
    entities::PackageId,
    services::ProxyService,
};

use crate::{RegistryMap, UpstreamMap, error::AppError, extractors::AuthIdentity};
use super::common::proxy_stream;

fn require_npm_or_cargo(registry: &str, map: &RegistryMap) -> Result<(), AppError> {
    match map.type_of(registry) {
        Some("npm") | Some("cargo") | Some("openvsx") => Ok(()),
        Some(_) => Err(AppError::not_found(format!(
            "registry '{registry}' is not an npm, cargo, or openvsx registry"
        ))),
        None => Err(AppError::not_found(format!("unknown registry '{registry}'"))),
    }
}

fn require_npm(registry: &str, map: &RegistryMap) -> Result<(), AppError> {
    match map.type_of(registry) {
        Some("npm") => Ok(()),
        Some(_) => Err(AppError::not_found(format!("registry '{registry}' is not an npm registry"))),
        None => Err(AppError::not_found(format!("unknown registry '{registry}'"))),
    }
}

/// Fetch package metadata (all versions / packument for npm, crate info for cargo).
///
/// Shared handler for npm and cargo registries.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/{package}",
    tag = "proxy/npm",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("package"  = String, Path, description = "Package / crate name"),
    ),
    responses(
        (status = 200, description = "Package metadata JSON"),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Unknown registry"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/{package}")]
pub async fn get_packument(
    path: web::Path<(String, String)>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    map: web::Data<RegistryMap>,
) -> Result<impl Responder, AppError> {
    let (registry, package) = path.into_inner();
    require_npm_or_cargo(&registry, &map)?;
    let pkg = PackageId::new(&registry, &package, "latest");
    proxy_stream(svc, pkg, identity, "releases:read", None).await
}

/// Fetch package version metadata.
///
/// Shared handler for npm and cargo registries.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/{package}/{version}",
    tag = "proxy/npm",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("package"  = String, Path, description = "Package / crate name"),
        ("version"  = String, Path, description = "Version"),
    ),
    responses(
        (status = 200, description = "Version metadata JSON"),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Unknown registry"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/{package}/{version}")]
pub async fn get_version(
    path: web::Path<(String, String, String)>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    map: web::Data<RegistryMap>,
) -> Result<impl Responder, AppError> {
    let (registry, package, version) = path.into_inner();
    require_npm_or_cargo(&registry, &map)?;
    let pkg = PackageId::new(&registry, &package, &version);
    proxy_stream(svc, pkg, identity, "releases:read", None).await
}

/// Download npm package tarball for a specific version.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/{package}/{version}/tarball",
    tag = "proxy/npm",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("package"  = String, Path, description = "Package name"),
        ("version"  = String, Path, description = "Version"),
    ),
    responses(
        (status = 200, description = "npm .tgz tarball"),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Unknown registry"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/{package}/{version}/tarball")]
pub async fn download_tarball(
    path: web::Path<(String, String, String)>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    map: web::Data<RegistryMap>,
) -> Result<impl Responder, AppError> {
    let (registry, package, version) = path.into_inner();
    require_npm(&registry, &map)?;
    let pkg = PackageId::new(&registry, &package, &version).with_artifact("tarball");
    proxy_stream(svc, pkg, identity, "source:read", None).await
}

/// Proxy npm audit requests to the upstream npm registry.
///
/// npm sends `POST /-/npm/v1/audit/quick` when `npm audit` runs. Forwards the
/// request body to the configured upstream and returns the response, enabling
/// `npm audit` to work through the proxy without caching.
#[utoipa::path(
    post,
    path = "/proxy/{registry}/-/npm/v1/audit/quick",
    tag = "proxy/npm",
    params(
        ("registry" = String, Path, description = "npm registry name"),
    ),
    request_body = serde_json::Value,
    responses(
        (status = 200, description = "Audit advisory data from upstream"),
        (status = 404, description = "Unknown or non-npm registry"),
        (status = 502, description = "Upstream audit request failed"),
    ),
    security(("bearer_token" = [])),
)]
#[post("/proxy/{registry}/-/npm/v1/audit/quick")]
pub async fn audit_quick(
    path: web::Path<(String,)>,
    body: web::Json<serde_json::Value>,
    map: web::Data<RegistryMap>,
    upstream_map: web::Data<UpstreamMap>,
    client: web::Data<reqwest::Client>,
) -> Result<impl Responder, AppError> {
    let (registry,) = path.into_inner();
    require_npm(&registry, &map)?;

    let upstream = upstream_map
        .upstream_for(&registry)
        .ok_or_else(|| AppError::not_found(format!("no upstream configured for '{registry}'")))?;

    let url = format!("{upstream}/-/npm/v1/audit/quick");

    let resp = client
        .post(&url)
        .header("Content-Type", "application/json")
        .json(&body.into_inner())
        .send()
        .await
        .map_err(|e| AppError::internal(format!("upstream audit request failed: {e}")))?;

    let status = actix_web::http::StatusCode::from_u16(resp.status().as_u16())
        .unwrap_or(actix_web::http::StatusCode::INTERNAL_SERVER_ERROR);
    let response_body = resp
        .bytes()
        .await
        .map_err(|e| AppError::internal(format!("upstream audit response read failed: {e}")))?;

    Ok(HttpResponse::build(status)
        .content_type("application/json")
        .body(response_body))
}

