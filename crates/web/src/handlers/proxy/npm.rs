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
pub struct NpmPackageParts {
    package: String,
}

#[derive(Deserialize, IntoParams)]
pub struct NpmPackageVersionParts {
    package: String,
    version: String,
}

// ── Handlers ──────────────────────────────────────────────────────────────────

/// Fetch npm package packument (all versions + dist-tags).
#[utoipa::path(
    get,
    path = "/proxy/npm/{package}",
    tag = "proxy",
    params(NpmPackageParts),
    responses(
        (status = 200, description = "npm packument JSON"),
        (status = 403, description = "Access denied"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/npm/{package}")]
pub async fn get_packument(
    path: web::Path<NpmPackageParts>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
) -> Result<impl Responder, AppError> {
    let pkg = PackageId::new("npm", &path.package, "latest");
    proxy_stream(svc, pkg, identity, "releases:read").await
}

/// Fetch npm package metadata for a specific version.
#[utoipa::path(
    get,
    path = "/proxy/npm/{package}/{version}",
    tag = "proxy",
    params(NpmPackageVersionParts),
    responses(
        (status = 200, description = "npm version metadata JSON"),
        (status = 403, description = "Access denied"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/npm/{package}/{version}")]
pub async fn get_version(
    path: web::Path<NpmPackageVersionParts>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
) -> Result<impl Responder, AppError> {
    let pkg = PackageId::new("npm", &path.package, &path.version);
    proxy_stream(svc, pkg, identity, "releases:read").await
}

/// Download npm package tarball for a specific version.
#[utoipa::path(
    get,
    path = "/proxy/npm/{package}/{version}/tarball",
    tag = "proxy",
    params(NpmPackageVersionParts),
    responses(
        (status = 200, description = "npm .tgz tarball"),
        (status = 403, description = "Access denied"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/npm/{package}/{version}/tarball")]
pub async fn download_tarball(
    path: web::Path<NpmPackageVersionParts>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
) -> Result<impl Responder, AppError> {
    let pkg = PackageId::new("npm", &path.package, &path.version)
        .with_artifact("tarball");
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
