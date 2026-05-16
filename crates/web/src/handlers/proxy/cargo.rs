use std::sync::Arc;

use actix_web::{HttpResponse, Responder, get, web};
use bytes::Bytes;
use futures::StreamExt;
use serde::Deserialize;
use utoipa::IntoParams;

use proxy_cache_core::{
    entities::PackageId,
    services::{ProxyRequest, ProxyResponse, ProxyService},
};

use crate::{error::AppError, extractors::AuthIdentity};

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
