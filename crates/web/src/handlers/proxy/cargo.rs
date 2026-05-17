use std::collections::HashMap;
use std::sync::Arc;

use actix_web::{HttpRequest, HttpResponse, Responder, get, web};
use bytes::Bytes;
use futures::StreamExt;

use proxy_cache_core::{
    entities::PackageId,
    services::{ProxyRequest, ProxyResponse, ProxyService},
};

use crate::{RegistryMap, error::AppError, extractors::AuthIdentity};

// ── Sparse index proxy ────────────────────────────────────────────────────────

/// HTTP client + upstream index URL for one cargo sparse index.
#[derive(Clone)]
pub struct CargoIndexProxy {
    pub http: reqwest::Client,
    /// Base URL of the upstream sparse index, e.g. `https://index.crates.io`.
    pub index_url: String,
}

fn require_cargo(registry: &str, map: &RegistryMap) -> Result<(), AppError> {
    match map.type_of(registry) {
        Some("cargo") => Ok(()),
        Some(_) => Err(AppError::not_found(format!("registry '{registry}' is not a cargo registry"))),
        None => Err(AppError::not_found(format!("unknown registry '{registry}'"))),
    }
}

/// Cargo sparse registry `config.json`.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/registry/config.json",
    tag = "proxy/cargo",
    params(("registry" = String, Path, description = "Registry name")),
    responses(
        (status = 200, description = "Sparse registry configuration"),
        (status = 404, description = "No cargo registry configured"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/registry/config.json")]
pub async fn cargo_registry_config(
    path: web::Path<String>,
    indexes: web::Data<HashMap<String, CargoIndexProxy>>,
    map: web::Data<RegistryMap>,
    req: HttpRequest,
) -> HttpResponse {
    let registry = path.into_inner();
    if !map.is_type(&registry, "cargo") {
        return HttpResponse::NotFound().body(format!("unknown cargo registry '{registry}'"));
    }
    let Some(_) = indexes.get(&registry) else {
        return HttpResponse::NotFound().body("no cargo registry configured");
    };
    let (scheme, host) = {
        let info = req.connection_info();
        (info.scheme().to_owned(), info.host().to_owned())
    };
    let dl = format!("{scheme}://{host}/proxy/{registry}/{{crate}}/{{version}}/download");
    HttpResponse::Ok()
        .content_type("application/json")
        .json(serde_json::json!({ "dl": dl }))
}

/// Cargo sparse registry index entries.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/registry/{path}",
    tag = "proxy/cargo",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("path"     = String, Path, description = "Crate index path, e.g. se/rd/serde"),
    ),
    responses(
        (status = 200, description = "Sparse index entry (newline-delimited JSON)"),
        (status = 404, description = "Crate not found in index"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/registry/{path:.*}")]
pub async fn cargo_registry_index(
    path: web::Path<(String, String)>,
    indexes: web::Data<HashMap<String, CargoIndexProxy>>,
    map: web::Data<RegistryMap>,
    _identity: AuthIdentity,
) -> HttpResponse {
    let (registry, index_path) = path.into_inner();
    if !map.is_type(&registry, "cargo") {
        return HttpResponse::NotFound().body(format!("unknown cargo registry '{registry}'"));
    }
    let Some(index) = indexes.get(&registry) else {
        return HttpResponse::NotFound().body("no cargo registry configured");
    };
    let url = format!("{}/{}", index.index_url.trim_end_matches('/'), index_path);
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

/// Download a `.crate` file for a specific version.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/{name}/{version}/download",
    tag = "proxy/cargo",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("name"     = String, Path, description = "Crate name"),
        ("version"  = String, Path, description = "Version"),
    ),
    responses(
        (status = 200, description = ".crate file stream"),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Unknown registry"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/{name}/{version}/download")]
pub async fn download_crate(
    path: web::Path<(String, String, String)>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    map: web::Data<RegistryMap>,
) -> Result<impl Responder, AppError> {
    let (registry, name, version) = path.into_inner();
    require_cargo(&registry, &map)?;
    let pkg = PackageId::new(&registry, &name, &version).with_artifact("dl");
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
