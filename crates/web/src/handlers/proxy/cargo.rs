use std::sync::Arc;

use actix_web::{HttpRequest, HttpResponse, Responder, get, web};
use bytes::Bytes;
use futures::StreamExt;
use serde::Deserialize;
use utoipa::IntoParams;

use proxy_cache_core::{
    entities::PackageId,
    services::{ProxyRequest, ProxyResponse, ProxyService},
};

use crate::{error::AppError, extractors::AuthIdentity};

// ── Sparse index proxy ────────────────────────────────────────────────────────

/// Holds the HTTP client and index URL for proxying the cargo sparse index.
#[derive(Clone)]
pub struct CargoIndexProxy {
    pub http: reqwest::Client,
    /// Base URL of the upstream sparse index, e.g. `https://index.crates.io`.
    pub index_url: String,
}

/// Cargo sparse registry config.json.
///
/// Tells cargo where to download `.crate` files and resolves the `dl` template
/// against the request's own host so the URL works in any environment.
#[utoipa::path(
    get,
    path = "/proxy/cargo/registry/config.json",
    tag = "proxy",
    responses(
        (status = 200, description = "Sparse registry configuration"),
        (status = 404, description = "No cargo registry configured"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/cargo/registry/config.json")]
pub async fn cargo_registry_config(
    index: Option<web::Data<CargoIndexProxy>>,
    req: HttpRequest,
) -> HttpResponse {
    if index.is_none() {
        return HttpResponse::NotFound().body("no cargo registry configured");
    }
    let (scheme, host) = {
        let info = req.connection_info();
        (info.scheme().to_owned(), info.host().to_owned())
    };
    let dl = format!("{scheme}://{host}/proxy/cargo/{{crate}}/{{version}}/download");
    HttpResponse::Ok()
        .content_type("application/json")
        .json(serde_json::json!({ "dl": dl }))
}

/// Cargo sparse registry index entries.
///
/// Proxies `{index_url}/{path}` (e.g. `index.crates.io/se/rd/serde`) and
/// streams the newline-delimited JSON directly to the cargo client.
#[utoipa::path(
    get,
    path = "/proxy/cargo/registry/{path}",
    tag = "proxy",
    params(("path" = String, Path, description = "Crate index path, e.g. se/rd/serde")),
    responses(
        (status = 200, description = "Sparse index entry (newline-delimited JSON)"),
        (status = 404, description = "Crate not found in index"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/cargo/registry/{path:.*}")]
pub async fn cargo_registry_index(
    path: web::Path<String>,
    index: Option<web::Data<CargoIndexProxy>>,
    _identity: AuthIdentity,
) -> HttpResponse {
    let Some(index) = index else {
        return HttpResponse::NotFound().body("no cargo registry configured");
    };
    let url = format!("{}/{}", index.index_url.trim_end_matches('/'), path.as_ref());
    tracing::debug!(url = %url, "fetching cargo sparse index entry");

    let resp = match index.http.get(&url).send().await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(url = %url, error = %e, "cargo index fetch failed");
            return HttpResponse::BadGateway().body(e.to_string());
        }
    };

    let status = actix_web::http::StatusCode::from_u16(resp.status().as_u16())
        .unwrap_or(actix_web::http::StatusCode::INTERNAL_SERVER_ERROR);

    match resp.bytes().await {
        Ok(bytes) => HttpResponse::build(status)
            .content_type("text/plain; charset=utf-8")
            .body(bytes),
        Err(e) => HttpResponse::BadGateway().body(e.to_string()),
    }
}

// ── Path extractors ───────────────────────────────────────────────────────────

#[derive(Deserialize, IntoParams)]
pub struct CrateParts {
    name: String,
}

#[derive(Deserialize, IntoParams)]
pub struct CrateVersionParts {
    name: String,
    version: String,
}

// ── Handlers ──────────────────────────────────────────────────────────────────

/// Fetch crate metadata (all versions).
#[utoipa::path(
    get,
    path = "/proxy/cargo/{name}",
    tag = "proxy",
    params(CrateParts),
    responses(
        (status = 200, description = "Crate info JSON (crates.io format)"),
        (status = 403, description = "Access denied"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/cargo/{name}")]
pub async fn get_crate(
    path: web::Path<CrateParts>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
) -> Result<impl Responder, AppError> {
    let pkg = PackageId::new("cargo", &path.name, "latest");
    proxy_stream(svc, pkg, identity, "releases:read").await
}

/// Fetch metadata for a specific crate version.
#[utoipa::path(
    get,
    path = "/proxy/cargo/{name}/{version}",
    tag = "proxy",
    params(CrateVersionParts),
    responses(
        (status = 200, description = "Crate version metadata JSON"),
        (status = 403, description = "Access denied"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/cargo/{name}/{version}")]
pub async fn get_version(
    path: web::Path<CrateVersionParts>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
) -> Result<impl Responder, AppError> {
    let pkg = PackageId::new("cargo", &path.name, &path.version);
    proxy_stream(svc, pkg, identity, "releases:read").await
}

/// Download a `.crate` file for a specific version.
#[utoipa::path(
    get,
    path = "/proxy/cargo/{name}/{version}/download",
    tag = "proxy",
    params(CrateVersionParts),
    responses(
        (status = 200, description = ".crate file stream"),
        (status = 403, description = "Access denied"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/cargo/{name}/{version}/download")]
pub async fn download_crate(
    path: web::Path<CrateVersionParts>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
) -> Result<impl Responder, AppError> {
    let pkg = PackageId::new("cargo", &path.name, &path.version)
        .with_artifact("dl");
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
                chunk.ok().map(|b| Ok::<Bytes, actix_web::Error>(b))
            });
            Ok(HttpResponse::Ok().streaming(body))
        }
    }
}
