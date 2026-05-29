use std::sync::Arc;

use actix_web::{get, web, HttpRequest, Responder};

use batlehub_core::{entities::PackageId, services::ProxyService};

use super::common::proxy_stream;
use crate::{error::AppError, extractors::AuthIdentity, RegistryMap};

fn require_github(registry: &str, map: &RegistryMap) -> Result<(), AppError> {
    match map.type_of(registry) {
        Some("github") => Ok(()),
        Some(_) => Err(AppError::not_found(format!(
            "registry '{registry}' is not a github registry"
        ))),
        None => Err(AppError::not_found(format!(
            "unknown registry '{registry}'"
        ))),
    }
}

/// List GitHub releases for a repository.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/{owner}/{repo}/releases",
    tag = "proxy/github",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("owner"    = String, Path, description = "Repository owner"),
        ("repo"     = String, Path, description = "Repository name"),
    ),
    responses(
        (status = 200, description = "Release list (GitHub API JSON)"),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Unknown registry"),
        (status = 500, description = "Internal error"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/{owner}/{repo}/releases")]
pub async fn list_releases(
    path: web::Path<(String, String, String)>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    map: web::Data<RegistryMap>,
) -> Result<impl Responder, AppError> {
    let (registry, owner, repo) = path.into_inner();
    require_github(&registry, &map)?;
    let pkg = PackageId::new(&registry, format!("{owner}/{repo}"), "releases");
    proxy_stream(svc, pkg, identity, "releases:read", None).await
}

/// Get a specific GitHub release by tag.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/{owner}/{repo}/releases/tags/{tag}",
    tag = "proxy/github",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("owner"    = String, Path, description = "Repository owner"),
        ("repo"     = String, Path, description = "Repository name"),
        ("tag"      = String, Path, description = "Release tag"),
    ),
    responses(
        (status = 200, description = "Release metadata"),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Unknown registry"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/{owner}/{repo}/releases/tags/{tag}")]
pub async fn get_release(
    path: web::Path<(String, String, String, String)>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    map: web::Data<RegistryMap>,
) -> Result<impl Responder, AppError> {
    let (registry, owner, repo, tag) = path.into_inner();
    require_github(&registry, &map)?;
    let pkg = PackageId::new(&registry, format!("{owner}/{repo}"), tag);
    proxy_stream(svc, pkg, identity, "releases:read", None).await
}

/// Download a GitHub release asset by ID.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/{owner}/{repo}/releases/assets/{asset_id}",
    tag = "proxy/github",
    params(
        ("registry"  = String, Path, description = "Registry name"),
        ("owner"     = String, Path, description = "Repository owner"),
        ("repo"      = String, Path, description = "Repository name"),
        ("asset_id"  = String, Path, description = "Asset ID"),
    ),
    responses(
        (status = 200, description = "Asset binary stream"),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Unknown registry"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/{owner}/{repo}/releases/assets/{asset_id}")]
pub async fn download_asset(
    path: web::Path<(String, String, String, String)>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    map: web::Data<RegistryMap>,
    req: HttpRequest,
) -> Result<impl Responder, AppError> {
    let (registry, owner, repo, asset_id) = path.into_inner();
    require_github(&registry, &map)?;
    let tag =
        web::Query::<std::collections::HashMap<String, String>>::from_query(req.query_string())
            .ok()
            .and_then(|q| q.into_inner().remove("tag"))
            .unwrap_or_else(|| "unknown".to_owned());
    let pkg = PackageId::new(&registry, format!("{owner}/{repo}"), tag).with_artifact(&asset_id);
    proxy_stream(svc, pkg, identity, "releases:read", None).await
}

/// Download a GitHub release asset by filename.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/{owner}/{repo}/releases/download/{tag}/{filename}",
    tag = "proxy/github",
    params(
        ("registry"  = String, Path, description = "Registry name"),
        ("owner"     = String, Path, description = "Repository owner"),
        ("repo"      = String, Path, description = "Repository name"),
        ("tag"       = String, Path, description = "Release tag"),
        ("filename"  = String, Path, description = "Asset filename"),
    ),
    responses(
        (status = 200, description = "Asset binary stream"),
        (status = 404, description = "Asset not found or unknown registry"),
        (status = 403, description = "Access denied"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/{owner}/{repo}/releases/download/{tag}/{filename}")]
pub async fn download_asset_by_name(
    path: web::Path<(String, String, String, String, String)>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    map: web::Data<RegistryMap>,
) -> Result<impl Responder, AppError> {
    let (registry, owner, repo, tag, filename) = path.into_inner();
    require_github(&registry, &map)?;
    let pkg = PackageId::new(&registry, format!("{owner}/{repo}"), tag)
        .with_artifact(format!("filename/{filename}"));
    proxy_stream(svc, pkg, identity, "releases:read", None).await
}

/// Download a GitHub source tarball.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/{owner}/{repo}/tarball/{tag}",
    tag = "proxy/github",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("owner"    = String, Path, description = "Repository owner"),
        ("repo"     = String, Path, description = "Repository name"),
        ("tag"      = String, Path, description = "Release tag"),
    ),
    responses(
        (status = 200, description = "Source tarball stream"),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Unknown registry"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/{owner}/{repo}/tarball/{tag}")]
pub async fn download_tarball(
    path: web::Path<(String, String, String, String)>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    map: web::Data<RegistryMap>,
) -> Result<impl Responder, AppError> {
    let (registry, owner, repo, tag) = path.into_inner();
    require_github(&registry, &map)?;
    let pkg = PackageId::new(&registry, format!("{owner}/{repo}"), &tag)
        .with_artifact(format!("tarball/{tag}"));
    proxy_stream(svc, pkg, identity, "source:read", None).await
}

/// Download a GitHub zip archive.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/{owner}/{repo}/zipball/{tag}",
    tag = "proxy/github",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("owner"    = String, Path, description = "Repository owner"),
        ("repo"     = String, Path, description = "Repository name"),
        ("tag"      = String, Path, description = "Release tag"),
    ),
    responses(
        (status = 200, description = "Zip archive stream"),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Unknown registry"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/{owner}/{repo}/zipball/{tag}")]
pub async fn download_zipball(
    path: web::Path<(String, String, String, String)>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    map: web::Data<RegistryMap>,
) -> Result<impl Responder, AppError> {
    let (registry, owner, repo, tag) = path.into_inner();
    require_github(&registry, &map)?;
    let pkg = PackageId::new(&registry, format!("{owner}/{repo}"), &tag).with_artifact("zipball");
    proxy_stream(svc, pkg, identity, "source:read", None).await
}

/// Download a raw file from a GitHub repository.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/{owner}/{repo}/raw/{git_ref}/{path}",
    tag = "proxy/github",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("owner"    = String, Path, description = "Repository owner"),
        ("repo"     = String, Path, description = "Repository name"),
        ("git_ref"  = String, Path, description = "Branch, tag, or commit SHA"),
        ("path"     = String, Path, description = "File path within the repository"),
    ),
    responses(
        (status = 200, description = "Raw file content"),
        (status = 404, description = "File not found or unknown registry"),
        (status = 403, description = "Access denied"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/{owner}/{repo}/raw/{git_ref}/{path:.*}")]
pub async fn download_raw(
    path: web::Path<(String, String, String, String, String)>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    map: web::Data<RegistryMap>,
) -> Result<impl Responder, AppError> {
    let (registry, owner, repo, git_ref, file_path) = path.into_inner();
    require_github(&registry, &map)?;
    let pkg = PackageId::new(&registry, format!("{owner}/{repo}"), git_ref)
        .with_artifact(format!("raw/{file_path}"));
    proxy_stream(svc, pkg, identity, "source:read", None).await
}
