use std::sync::Arc;

use actix_web::{HttpResponse, Responder, get, web};
use bytes::Bytes;
use futures::StreamExt;

use proxy_cache_core::{
    entities::PackageId,
    services::{ProxyRequest, ProxyResponse, ProxyService},
};

use crate::{RegistryMap, error::AppError, extractors::AuthIdentity};

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
    proxy_stream(svc, pkg, identity, "releases:read").await
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
    proxy_stream(svc, pkg, identity, "releases:read").await
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
    proxy_stream(svc, pkg, identity, "source:read").await
}

// ── Shared stream helper ──────────────────────────────────────────────────────

async fn proxy_stream(
    svc: web::Data<Arc<ProxyService>>,
    pkg: PackageId,
    identity: AuthIdentity,
    resource_type: &str,
) -> Result<HttpResponse, AppError> {
    let req = ProxyRequest {
        package_id: pkg,
        identity: identity.0.clone(),
        resource_type: resource_type.to_owned(),
    };
    match svc.handle(req).await.map_err(AppError::from)? {
        ProxyResponse::Denied { reason } => Err(AppError::forbidden(reason)),
        ProxyResponse::Stream(stream) => {
            let body = stream.filter_map(|chunk| async move {
                chunk.ok().map(Ok::<Bytes, actix_web::Error>)
            });
            Ok(HttpResponse::Ok().streaming(body))
        }
    }
}
