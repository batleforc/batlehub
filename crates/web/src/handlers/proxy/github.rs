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

// ── Path extractors ───────────────────────────────────────────────────────────

#[derive(Deserialize, IntoParams)]
pub struct OwnerRepoParts {
    owner: String,
    repo: String,
}

#[derive(Deserialize, IntoParams)]
pub struct OwnerRepoAssetParts {
    owner: String,
    repo: String,
    asset_id: String,
}

#[derive(Deserialize, IntoParams)]
pub struct OwnerRepoTagParts {
    owner: String,
    repo: String,
    tag: String,
}

// ── Handlers ──────────────────────────────────────────────────────────────────

/// List GitHub releases for a repository.
#[utoipa::path(
    get,
    path = "/proxy/github/{owner}/{repo}/releases",
    tag = "proxy",
    params(OwnerRepoParts),
    responses(
        (status = 200, description = "Release list (GitHub API JSON)"),
        (status = 403, description = "Access denied"),
        (status = 500, description = "Internal error"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/github/{owner}/{repo}/releases")]
pub async fn list_releases(
    path: web::Path<OwnerRepoParts>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
) -> Result<impl Responder, AppError> {
    let pkg = PackageId::new("github", format!("{}/{}", path.owner, path.repo), "releases");
    proxy_stream(svc, pkg, identity, "releases:read").await
}

/// Get a specific GitHub release by tag.
#[utoipa::path(
    get,
    path = "/proxy/github/{owner}/{repo}/releases/tags/{tag}",
    tag = "proxy",
    params(OwnerRepoTagParts),
    responses(
        (status = 200, description = "Release metadata"),
        (status = 403, description = "Access denied"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/github/{owner}/{repo}/releases/tags/{tag}")]
pub async fn get_release(
    path: web::Path<OwnerRepoTagParts>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
) -> Result<impl Responder, AppError> {
    let pkg = PackageId::new("github", format!("{}/{}", path.owner, path.repo), &path.tag);
    proxy_stream(svc, pkg, identity, "releases:read").await
}

/// Download a GitHub release asset.
#[utoipa::path(
    get,
    path = "/proxy/github/{owner}/{repo}/releases/assets/{asset_id}",
    tag = "proxy",
    params(OwnerRepoAssetParts),
    responses(
        (status = 200, description = "Asset binary stream"),
        (status = 403, description = "Access denied"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/github/{owner}/{repo}/releases/assets/{asset_id}")]
pub async fn download_asset(
    path: web::Path<OwnerRepoAssetParts>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    req: HttpRequest,
) -> Result<impl Responder, AppError> {
    // Determine the release tag from a query param so the age gate works.
    // e.g. ?tag=v1.80.0
    let tag = web::Query::<std::collections::HashMap<String, String>>::from_query(req.query_string())
        .ok()
        .and_then(|q| q.into_inner().remove("tag"))
        .unwrap_or_else(|| "unknown".to_owned());

    let pkg = PackageId::new("github", format!("{}/{}", path.owner, path.repo), tag)
        .with_artifact(&path.asset_id);

    proxy_stream(svc, pkg, identity, "releases:read").await
}

/// Download a GitHub release asset by filename.
///
/// Mirrors the `github.com/{owner}/{repo}/releases/download/{tag}/{filename}` URL
/// so mise URL replacements can redirect binary asset downloads through the proxy.
#[utoipa::path(
    get,
    path = "/proxy/github/{owner}/{repo}/releases/download/{tag}/{filename}",
    tag = "proxy",
    params(
        ("owner"    = String, Path, description = "Repository owner"),
        ("repo"     = String, Path, description = "Repository name"),
        ("tag"      = String, Path, description = "Release tag"),
        ("filename" = String, Path, description = "Asset filename"),
    ),
    responses(
        (status = 200, description = "Asset binary stream"),
        (status = 404, description = "Asset not found"),
        (status = 403, description = "Access denied"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/github/{owner}/{repo}/releases/download/{tag}/{filename}")]
pub async fn download_asset_by_name(
    params: web::Path<(String, String, String, String)>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
) -> Result<impl Responder, AppError> {
    let (owner, repo, tag, filename) = params.into_inner();
    let pkg = PackageId::new("github", format!("{owner}/{repo}"), tag)
        .with_artifact(format!("filename/{filename}"));
    proxy_stream(svc, pkg, identity, "releases:read").await
}

/// Download a GitHub source tarball.
#[utoipa::path(
    get,
    path = "/proxy/github/{owner}/{repo}/tarball/{tag}",
    tag = "proxy",
    params(OwnerRepoTagParts),
    responses(
        (status = 200, description = "Source tarball stream"),
        (status = 403, description = "Access denied"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/github/{owner}/{repo}/tarball/{tag}")]
pub async fn download_tarball(
    path: web::Path<OwnerRepoTagParts>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
) -> Result<impl Responder, AppError> {
    let pkg =
        PackageId::new("github", format!("{}/{}", path.owner, path.repo), &path.tag)
            .with_artifact(format!("tarball/{}", path.tag));

    proxy_stream(svc, pkg, identity, "source:read").await
}

/// Download a GitHub zip archive of a repository at a given ref.
#[utoipa::path(
    get,
    path = "/proxy/github/{owner}/{repo}/zipball/{tag}",
    tag = "proxy",
    params(OwnerRepoTagParts),
    responses(
        (status = 200, description = "Zip archive stream"),
        (status = 403, description = "Access denied"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/github/{owner}/{repo}/zipball/{tag}")]
pub async fn download_zipball(
    path: web::Path<OwnerRepoTagParts>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
) -> Result<impl Responder, AppError> {
    let pkg =
        PackageId::new("github", format!("{}/{}", path.owner, path.repo), &path.tag)
            .with_artifact("zipball");

    proxy_stream(svc, pkg, identity, "source:read").await
}

/// Download a raw file from a GitHub repository at a given ref.
///
/// Equivalent to `raw.githubusercontent.com/{owner}/{repo}/{ref}/{path}`.
#[utoipa::path(
    get,
    path = "/proxy/github/{owner}/{repo}/raw/{git_ref}/{path}",
    tag = "proxy",
    params(
        ("owner" = String, Path, description = "Repository owner"),
        ("repo"  = String, Path, description = "Repository name"),
        ("git_ref" = String, Path, description = "Branch, tag, or commit SHA"),
        ("path"  = String, Path, description = "File path within the repository"),
    ),
    responses(
        (status = 200, description = "Raw file content"),
        (status = 404, description = "File not found"),
        (status = 403, description = "Access denied"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/github/{owner}/{repo}/raw/{git_ref}/{path:.*}")]
pub async fn download_raw(
    params: web::Path<(String, String, String, String)>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
) -> Result<impl Responder, AppError> {
    let (owner, repo, git_ref, file_path) = params.into_inner();
    let pkg = PackageId::new("github", format!("{owner}/{repo}"), git_ref)
        .with_artifact(format!("raw/{file_path}"));

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
