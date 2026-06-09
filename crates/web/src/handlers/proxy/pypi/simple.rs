use std::sync::Arc;

use actix_web::{get, web, HttpRequest, HttpResponse, Responder};

use batlehub_config::schema::RegistryMode as Mode;
use batlehub_core::{
    entities::PackageId,
    error::CoreError,
    services::{LocalRegistryService, ProxyService},
};

use crate::handlers::proxy::common::{proxy_stream, require_registry_type};
use crate::{error::AppError, extractors::AuthIdentity, RegistryMap, RegistryModeMap, UpstreamMap};

use super::parse_pypi_filename;

// ── Proxy routes ──────────────────────────────────────────────────────────────

/// Proxy the PyPI Simple Repository API root index.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/simple/",
    tag = "proxy/pypi",
    params(("registry" = String, Path, description = "Registry name")),
    responses(
        (status = 200, description = "Simple index HTML"),
        (status = 404, description = "Registry not found"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/simple/")]
pub async fn pypi_simple_root(
    path: web::Path<String>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    map: web::Data<RegistryMap>,
) -> Result<impl Responder, AppError> {
    let registry = path.into_inner();
    require_registry_type(&registry, "pypi", &map)?;

    // Represent the root index as a special sentinel PackageId.
    let pkg = PackageId::new(&registry, "__simple__", "__root__");
    proxy_stream(svc, pkg, identity, "releases:read", Some("text/html")).await
}

/// Proxy the PyPI Simple Repository API for a specific package, rewriting file
/// URLs so artifacts are downloaded through the batlehub cache.
///
/// In local/hybrid mode, the page is generated from locally-published packages.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/simple/{package}/",
    tag = "proxy/pypi",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("package"  = String, Path, description = "Package name"),
    ),
    responses(
        (status = 200, description = "Simple index page with rewritten file URLs"),
        (status = 404, description = "Package not found"),
    ),
    security(("bearer_token" = [])),
)]
#[allow(clippy::too_many_arguments)]
#[get("/proxy/{registry}/simple/{package}/")]
pub async fn pypi_simple_package(
    path: web::Path<(String, String)>,
    req: HttpRequest,
    identity: AuthIdentity,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
    upstream_map: web::Data<UpstreamMap>,
    http_client: web::Data<reqwest::Client>,
) -> Result<impl Responder, AppError> {
    let (registry, package) = path.into_inner();
    require_registry_type(&registry, "pypi", &map)?;

    let mode = mode_map.get(&registry);
    let normalized = batlehub_adapters::registry::pypi::normalize_name(&package);

    let proxy_base = {
        let conn_info = req.connection_info();
        format!("{}://{}", conn_info.scheme(), conn_info.host())
    };

    if mode == Mode::Local {
        let html = local_svc
            .get_pypi_simple_page(&registry, &normalized, &proxy_base, &identity.0)
            .await
            .map_err(AppError::from)?;
        return Ok(HttpResponse::Ok()
            .content_type("text/html; charset=utf-8")
            .body(html));
    }

    if mode == Mode::Hybrid {
        // Try local first; fall through to upstream if not found.
        match local_svc
            .get_pypi_simple_page(&registry, &normalized, &proxy_base, &identity.0)
            .await
        {
            Ok(html) => {
                return Ok(HttpResponse::Ok()
                    .content_type("text/html; charset=utf-8")
                    .body(html));
            }
            Err(batlehub_core::error::CoreError::NotFound(_)) => {}
            Err(e) => return Err(AppError::from(e)),
        }
    }

    let upstream = upstream_map
        .upstream_for(&registry)
        .ok_or_else(|| AppError::not_found(format!("no upstream for registry '{registry}'")))?;

    let accept = req
        .headers()
        .get(actix_web::http::header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);

    let (body, content_type) = batlehub_adapters::registry::pypi::fetch_simple_page(
        &http_client,
        &upstream,
        &package,
        None,
        accept.as_deref(),
    )
    .await
    .map_err(AppError::from)?;

    let rewritten = batlehub_adapters::registry::pypi::rewrite_simple_page(
        &body,
        content_type.as_deref(),
        &registry,
        &proxy_base,
    );

    let ct = content_type.unwrap_or_else(|| "text/html; charset=utf-8".to_owned());
    Ok(HttpResponse::Ok().content_type(ct).body(rewritten))
}

/// Download a PyPI distribution file through the proxy cache.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/packages/{filename}",
    tag = "proxy/pypi",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("filename" = String, Path, description = "Distribution filename"),
    ),
    responses(
        (status = 200, description = "Distribution bytes"),
        (status = 404, description = "File not found"),
        (status = 422, description = "Cannot parse filename"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/packages/{filename}")]
pub async fn pypi_file_download(
    path: web::Path<(String, String)>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
) -> Result<impl Responder, AppError> {
    let (registry, filename) = path.into_inner();
    require_registry_type(&registry, "pypi", &map)?;

    let mode = mode_map.get(&registry);

    if mode == batlehub_config::schema::RegistryMode::Local {
        let (name, version) = parse_pypi_filename(&filename).ok_or_else(|| {
            AppError::unprocessable(format!("cannot parse PyPI filename: {filename}"))
        })?;
        let bytes = local_svc
            .get_artifact(&registry, &name, &version, &identity)
            .await
            .map_err(AppError::from)?;
        return Ok(HttpResponse::Ok()
            .content_type("application/octet-stream")
            .body(bytes));
    }

    if mode == batlehub_config::schema::RegistryMode::Hybrid {
        if let Some((name, version)) = parse_pypi_filename(&filename) {
            match local_svc
                .get_artifact(&registry, &name, &version, &identity)
                .await
            {
                Ok(bytes) => {
                    return Ok(HttpResponse::Ok()
                        .content_type("application/octet-stream")
                        .body(bytes));
                }
                Err(CoreError::NotFound(_)) => {}
                Err(e) => return Err(AppError::from(e)),
            }
        }
    }

    let (name, version) = parse_pypi_filename(&filename).ok_or_else(|| {
        AppError::unprocessable(format!("cannot parse PyPI filename: {filename}"))
    })?;

    let pkg = PackageId::new(&registry, &name, &version).with_artifact(filename);
    proxy_stream(
        svc,
        pkg,
        identity,
        "releases:read",
        Some("application/octet-stream"),
    )
    .await
}
