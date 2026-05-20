use std::sync::Arc;

use actix_web::{Responder, get, web};

use proxy_cache_core::{
    entities::PackageId,
    services::ProxyService,
};

use crate::{RegistryMap, error::AppError, extractors::AuthIdentity};
use super::common::proxy_stream;

pub fn require_goproxy(registry: &str, map: &RegistryMap) -> Result<(), AppError> {
    match map.type_of(registry) {
        Some("goproxy") => Ok(()),
        Some(_) => Err(AppError::not_found(format!(
            "registry '{registry}' is not a goproxy registry"
        ))),
        None => Err(AppError::not_found(format!("unknown registry '{registry}'"))),
    }
}

/// Fetch the latest version info for a Go module.
///
/// The module path may contain slashes (e.g. `golang.org/x/text`).
/// Returns a JSON object `{"Version":"v1.2.3","Time":"..."}`.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/{module}/@latest",
    tag = "proxy/goproxy",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("module"   = String, Path, description = "Go module path (may contain slashes, e.g. golang.org/x/text)"),
    ),
    responses(
        (status = 200, description = "Latest version info JSON"),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Module not found"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/{module:[^@]+}@latest")]
pub async fn goproxy_latest(
    path: web::Path<(String, String)>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    map: web::Data<RegistryMap>,
) -> Result<impl Responder, AppError> {
    let (registry, raw_module) = path.into_inner();
    require_goproxy(&registry, &map)?;
    let module = raw_module.trim_end_matches('/');
    let pkg = PackageId::new(&registry, module, "latest");

    proxy_stream(svc, pkg, identity, "releases:read", Some("application/json")).await
}

/// List known versions for a Go module.
///
/// Returns a newline-separated list of available versions.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/{module}/@v/list",
    tag = "proxy/goproxy",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("module"   = String, Path, description = "Go module path (may contain slashes)"),
    ),
    responses(
        (status = 200, description = "Newline-separated version list"),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Module not found"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/{module:[^@]+}@v/list")]
pub async fn goproxy_list(
    path: web::Path<(String, String)>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    map: web::Data<RegistryMap>,
) -> Result<impl Responder, AppError> {
    let (registry, raw_module) = path.into_inner();
    require_goproxy(&registry, &map)?;
    let module = raw_module.trim_end_matches('/');
    let pkg = PackageId::new(&registry, module, "latest").with_artifact("list");

    proxy_stream(svc, pkg, identity, "releases:read", Some("text/plain")).await
}

/// Fetch a versioned Go module file: `.info`, `.mod`, or `.zip`.
///
/// - `{version}.info` — version metadata JSON `{"Version":"v1.2.3","Time":"..."}`
/// - `{version}.mod`  — the module's `go.mod` file
/// - `{version}.zip`  — the module source zip archive
#[utoipa::path(
    get,
    path = "/proxy/{registry}/{module}/@v/{filename}",
    tag = "proxy/goproxy",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("module"   = String, Path, description = "Go module path (may contain slashes)"),
        ("filename" = String, Path, description = "Versioned file: {version}.info, {version}.mod, or {version}.zip"),
    ),
    responses(
        (status = 200, description = "Requested module file"),
        (status = 400, description = "Unknown file type"),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Module or version not found"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/{module:[^@]+}@v/{filename}")]
pub async fn goproxy_file(
    path: web::Path<(String, String, String)>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    map: web::Data<RegistryMap>,
) -> Result<impl Responder, AppError> {
    let (registry, raw_module, filename) = path.into_inner();
    require_goproxy(&registry, &map)?;
    let module = raw_module.trim_end_matches('/');

    // Parse filename: "{version}.{ext}" — split at the last '.'
    let (version, ext) = filename
        .rsplit_once('.')
        .ok_or_else(|| AppError::not_found(format!("unknown goproxy file '{filename}'")))?;

    let (pkg, content_type, resource_type) = match ext {
        "info" => (
            PackageId::new(&registry, module, version),
            "application/json",
            "releases:read",
        ),
        "mod" => (
            PackageId::new(&registry, module, version).with_artifact("mod"),
            "text/plain",
            "releases:read",
        ),
        "zip" => (
            PackageId::new(&registry, module, version).with_artifact("zip"),
            "application/zip",
            "source:read",
        ),
        _ => {
            return Err(AppError::not_found(format!(
                "unknown goproxy file extension '.{ext}'"
            )));
        }
    };

    proxy_stream(svc, pkg, identity, resource_type, Some(content_type)).await
}
