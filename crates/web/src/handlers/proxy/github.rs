use std::sync::Arc;

use actix_web::{get, web, HttpRequest, Responder};

use batlehub_core::{entities::PackageId, services::ProxyService};

use super::common::proxy_stream;
use crate::{error::AppError, extractors::AuthIdentity, RegistryMap};

fn require_github(registry: &str, map: &RegistryMap) -> Result<(), AppError> {
    match map.type_of(registry).as_deref() {
        Some("github") => Ok(()),
        Some(_) => Err(AppError::not_found(format!(
            "registry '{registry}' is not a github registry"
        ))),
        None => Err(AppError::not_found(format!(
            "unknown registry '{registry}'"
        ))),
    }
}

/// Validate the registry, build the `PackageId` for a GitHub resource, and stream it
/// from upstream/cache.
///
/// `artifact` is appended via `PackageId::with_artifact` when present.
#[allow(clippy::too_many_arguments)]
async fn github_proxy(
    registry: &str,
    repo: String,
    pkg_ref: impl Into<String>,
    artifact: Option<String>,
    scope: &str,
    svc: web::Data<Arc<ProxyService>>,
    identity: AuthIdentity,
    map: &RegistryMap,
) -> Result<impl Responder, AppError> {
    require_github(registry, map)?;
    let mut pkg = PackageId::new(registry, repo, pkg_ref);
    if let Some(artifact) = artifact {
        pkg = pkg.with_artifact(artifact);
    }
    proxy_stream(svc, pkg, identity, scope, None).await
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
    github_proxy(
        &registry,
        format!("{owner}/{repo}"),
        "releases",
        None,
        "releases:read",
        svc,
        identity,
        &map,
    )
    .await
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
    github_proxy(
        &registry,
        format!("{owner}/{repo}"),
        tag,
        None,
        "releases:read",
        svc,
        identity,
        &map,
    )
    .await
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
    let tag =
        web::Query::<std::collections::HashMap<String, String>>::from_query(req.query_string())
            .ok()
            .and_then(|q| q.into_inner().remove("tag"))
            .unwrap_or_else(|| "unknown".to_owned());
    github_proxy(
        &registry,
        format!("{owner}/{repo}"),
        tag,
        Some(asset_id),
        "releases:read",
        svc,
        identity,
        &map,
    )
    .await
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
    github_proxy(
        &registry,
        format!("{owner}/{repo}"),
        tag,
        Some(format!("filename/{filename}")),
        "releases:read",
        svc,
        identity,
        &map,
    )
    .await
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
    let artifact = format!("tarball/{tag}");
    github_proxy(
        &registry,
        format!("{owner}/{repo}"),
        tag,
        Some(artifact),
        "source:read",
        svc,
        identity,
        &map,
    )
    .await
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
    github_proxy(
        &registry,
        format!("{owner}/{repo}"),
        tag,
        Some("zipball".to_owned()),
        "source:read",
        svc,
        identity,
        &map,
    )
    .await
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
    let artifact = format!("raw/{file_path}");
    github_proxy(
        &registry,
        format!("{owner}/{repo}"),
        git_ref,
        Some(artifact),
        "source:read",
        svc,
        identity,
        &map,
    )
    .await
}
