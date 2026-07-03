//! Forgejo/Gitea-specific handlers. Release endpoints are shared with the GitHub
//! handlers (identical URL scheme); this module adds the package-registry
//! passthrough, which is Forgejo-only.

use std::sync::Arc;

use actix_web::{get, web, Responder};

use batlehub_core::{entities::PackageId, services::ProxyService};

use super::common::proxy_stream;
use crate::{error::AppError, extractors::AuthIdentity, RegistryMap};

fn require_forgejo(registry: &str, map: &RegistryMap) -> Result<(), AppError> {
    match map.type_of(registry).as_deref() {
        Some("forgejo") => Ok(()),
        Some(_) => Err(AppError::not_found(format!(
            "registry '{registry}' is not a forgejo registry"
        ))),
        None => Err(AppError::not_found(format!(
            "unknown registry '{registry}'"
        ))),
    }
}

/// Proxy a Forgejo/Gitea package-registry path (`/api/packages/{owner}/…`).
///
/// Transparent passthrough/cache of the Forgejo Packages API — ideal for the
/// **generic** package registry (immutable file downloads). Ecosystem registries
/// (npm, Maven, PyPI, …) are better served by the matching typed adapter pointed at
/// the package endpoint, which rewrites metadata URLs.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/api/packages/{path}",
    tag = "proxy/forgejo",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("path" = String, Path, description = "Path under /api/packages/ (e.g. {owner}/generic/{name}/{version}/{file})"),
    ),
    responses(
        (status = 200, description = "Package file"),
        (status = 404, description = "Not found or unknown registry"),
        (status = 403, description = "Access denied"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/api/packages/{path:.*}")]
pub async fn fj_packages(
    path: web::Path<(String, String)>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    map: web::Data<RegistryMap>,
) -> Result<impl Responder, AppError> {
    let (registry, api_path) = path.into_inner();
    require_forgejo(&registry, &map)?;
    batlehub_core::services::validate_path_safe("path", &api_path).map_err(AppError::from)?;
    let pkg = PackageId::new(&registry, "_packages", "_")
        .with_artifact(format!("pkgpath/api/packages/{api_path}"));
    proxy_stream(
        svc,
        pkg,
        identity,
        batlehub_core::rules::resource_type::RELEASES_READ,
        None,
    )
    .await
}
