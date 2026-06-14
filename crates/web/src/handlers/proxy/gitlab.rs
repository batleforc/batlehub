use std::sync::Arc;

use actix_web::{get, web, Responder};

use batlehub_core::{entities::PackageId, services::ProxyService};

use super::common::proxy_stream;
use crate::{error::AppError, extractors::AuthIdentity, RegistryMap};

fn require_gitlab(registry: &str, map: &RegistryMap) -> Result<(), AppError> {
    match map.type_of(registry).as_deref() {
        Some("gitlab") => Ok(()),
        Some(_) => Err(AppError::not_found(format!(
            "registry '{registry}' is not a gitlab registry"
        ))),
        None => Err(AppError::not_found(format!(
            "unknown registry '{registry}'"
        ))),
    }
}

/// Validate the registry + project path, build the `PackageId`, and stream the
/// resource from upstream/cache. The GitLab project path may contain nested
/// groups (`group/subgroup/project`); each segment is validated at the edge.
#[allow(clippy::too_many_arguments)]
async fn gitlab_proxy(
    registry: &str,
    project: String,
    pkg_ref: impl Into<String>,
    artifact: Option<String>,
    scope: &str,
    svc: web::Data<Arc<ProxyService>>,
    identity: AuthIdentity,
    map: &RegistryMap,
) -> Result<impl Responder, AppError> {
    require_gitlab(registry, map)?;
    // Defence in depth: reject traversal in the project path before it reaches
    // the cache/storage key. ProxyService re-validates the coordinate too.
    batlehub_core::services::validate_path_safe("project", &project).map_err(AppError::from)?;
    let mut pkg = PackageId::new(registry, project, pkg_ref);
    if let Some(artifact) = artifact {
        pkg = pkg.with_artifact(artifact);
    }
    proxy_stream(svc, pkg, identity, scope, None).await
}

/// Map a GitLab archive filename suffix to an archive format selector.
fn archive_format(filename: &str) -> &'static str {
    if filename.ends_with(".tar.gz") || filename.ends_with(".tgz") {
        "tar.gz"
    } else if filename.ends_with(".tar.bz2") {
        "tar.bz2"
    } else if filename.ends_with(".zip") {
        "zip"
    } else if filename.ends_with(".tar") {
        "tar"
    } else {
        "tar.gz"
    }
}

/// List GitLab releases for a project.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/{project}/-/releases",
    tag = "proxy/gitlab",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("project"  = String, Path, description = "Full project path (group/subgroup/project)"),
    ),
    responses(
        (status = 200, description = "Release list (GitLab API JSON)"),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Unknown registry"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/{project:.+}/-/releases")]
pub async fn gl_list_releases(
    path: web::Path<(String, String)>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    map: web::Data<RegistryMap>,
) -> Result<impl Responder, AppError> {
    let (registry, project) = path.into_inner();
    gitlab_proxy(
        &registry,
        project,
        "releases",
        None,
        "releases:read",
        svc,
        identity,
        &map,
    )
    .await
}

/// Get a specific GitLab release by tag.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/{project}/-/releases/{tag}",
    tag = "proxy/gitlab",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("project"  = String, Path, description = "Full project path"),
        ("tag"      = String, Path, description = "Release tag"),
    ),
    responses(
        (status = 200, description = "Release metadata"),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Unknown registry"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/{project:.+}/-/releases/{tag}")]
pub async fn gl_get_release(
    path: web::Path<(String, String, String)>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    map: web::Data<RegistryMap>,
) -> Result<impl Responder, AppError> {
    let (registry, project, tag) = path.into_inner();
    gitlab_proxy(
        &registry,
        project,
        tag,
        None,
        "releases:read",
        svc,
        identity,
        &map,
    )
    .await
}

/// Download a GitLab release link asset (matched by link name).
#[utoipa::path(
    get,
    path = "/proxy/{registry}/{project}/-/releases/{tag}/downloads/{name}",
    tag = "proxy/gitlab",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("project"  = String, Path, description = "Full project path"),
        ("tag"      = String, Path, description = "Release tag"),
        ("name"     = String, Path, description = "Release link name"),
    ),
    responses(
        (status = 200, description = "Asset binary stream"),
        (status = 404, description = "Asset not found or unknown registry"),
        (status = 403, description = "Access denied"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/{project:.+}/-/releases/{tag}/downloads/{name:.*}")]
pub async fn gl_download_link(
    path: web::Path<(String, String, String, String)>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    map: web::Data<RegistryMap>,
) -> Result<impl Responder, AppError> {
    let (registry, project, tag, name) = path.into_inner();
    gitlab_proxy(
        &registry,
        project,
        tag,
        Some(format!("link/{name}")),
        "releases:read",
        svc,
        identity,
        &map,
    )
    .await
}

/// Download a GitLab source archive for a tag.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/{project}/-/archive/{tag}/{filename}",
    tag = "proxy/gitlab",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("project"  = String, Path, description = "Full project path"),
        ("tag"      = String, Path, description = "Git ref (tag/branch/SHA)"),
        ("filename" = String, Path, description = "Archive filename (format inferred from suffix)"),
    ),
    responses(
        (status = 200, description = "Source archive stream"),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Unknown registry"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/{project:.+}/-/archive/{tag}/{filename}")]
pub async fn gl_download_archive(
    path: web::Path<(String, String, String, String)>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    map: web::Data<RegistryMap>,
) -> Result<impl Responder, AppError> {
    let (registry, project, tag, filename) = path.into_inner();
    let format = archive_format(&filename);
    gitlab_proxy(
        &registry,
        project,
        tag,
        Some(format!("source/{format}")),
        "source:read",
        svc,
        identity,
        &map,
    )
    .await
}
