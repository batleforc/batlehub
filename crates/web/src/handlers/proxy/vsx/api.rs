//! The OpenVSX native REST API — what `ovsx` and openvsx-native clients speak.
//!
//! A second client protocol over the same [`super::source`] entries, so it
//! hides the same blocked versions and points at the same asset routes as the
//! VS Code gallery does. Only the document shape differs.

use std::sync::Arc;

use actix_web::{get, web, HttpRequest, HttpResponse, Responder};

use batlehub_core::services::{LocalRegistryService, ProxyService};

use super::protocol::GalleryQuery;
use super::render::{openvsx_extension_json, openvsx_search_json, GalleryUrls};
use super::{require_single_segment, require_vsx, source, vsx_kind};
use crate::handlers::proxy::common::registry_public_base;
use crate::handlers::schemas::{ArtifactBytes, UpstreamDocument};
use crate::{error::AppError, extractors::AuthIdentity, RegistryMap, RegistryModeMap};

/// Search the registry — `GET …/api/-/search`.
///
/// Registered **before** the `{namespace}` routes below, or `-` is taken for a
/// publisher name.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/api/-/search",
    tag = "proxy/openvsx",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("query"  = Option<String>, Query, description = "Free-text query"),
        ("size"   = Option<usize>,  Query, description = "Page size"),
        ("offset" = Option<usize>,  Query, description = "Result offset"),
    ),
    responses(
        (status = 200, description = "Search results", body = UpstreamDocument),
        (status = 404, description = "Unknown registry or wrong type"),
    ),
    security(("bearer_token" = [])),
)]
#[allow(clippy::too_many_arguments)]
#[get("/proxy/{registry}/api/-/search")]
pub async fn openvsx_search(
    req: HttpRequest,
    path: web::Path<String>,
    query: web::Query<SearchQuery>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
) -> Result<impl Responder, AppError> {
    let registry = path.into_inner();
    require_vsx(&registry, &map)?;

    let size = query.size.unwrap_or(18).clamp(1, 200);
    let offset = query.offset.unwrap_or(0);
    let gallery_query = GalleryQuery {
        search_text: query.query.clone().filter(|q| !q.trim().is_empty()),
        // The OpenVSX API pages by offset; the shared query type pages by
        // number, so convert rather than teach it a second scheme.
        page_number: offset / size + 1,
        page_size: size,
        ..Default::default()
    };

    let (entries, total) = source::search_entries(
        &svc,
        &local_svc,
        mode_map.get(&registry),
        &registry,
        vsx_kind(&registry, &map),
        &gallery_query,
        &identity,
    )
    .await?;

    let urls = GalleryUrls::new(&registry_public_base(&req, &registry));
    Ok(HttpResponse::Ok()
        .content_type("application/json")
        .json(openvsx_search_json(&entries, total, &urls)))
}

#[derive(Debug, serde::Deserialize, utoipa::IntoParams)]
pub struct SearchQuery {
    pub query: Option<String>,
    pub size: Option<usize>,
    pub offset: Option<usize>,
}

/// The newest version of one extension — `GET …/api/{namespace}/{extension}`.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/api/{namespace}/{extension}",
    tag = "proxy/openvsx",
    params(
        ("registry"  = String, Path, description = "Registry name"),
        ("namespace" = String, Path, description = "Extension publisher"),
        ("extension" = String, Path, description = "Extension name"),
    ),
    responses(
        (status = 200, description = "Extension metadata", body = UpstreamDocument),
        (status = 404, description = "Unknown registry or extension"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/api/{namespace}/{extension}")]
pub async fn openvsx_extension(
    req: HttpRequest,
    path: web::Path<(String, String, String)>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
) -> Result<impl Responder, AppError> {
    let (registry, namespace, extension) = path.into_inner();
    serve_extension(
        req, registry, namespace, extension, None, identity, svc, local_svc, map, mode_map,
    )
    .await
}

/// One specific version — `GET …/api/{namespace}/{extension}/{version}`.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/api/{namespace}/{extension}/{version}",
    tag = "proxy/openvsx",
    params(
        ("registry"  = String, Path, description = "Registry name"),
        ("namespace" = String, Path, description = "Extension publisher"),
        ("extension" = String, Path, description = "Extension name"),
        ("version"   = String, Path, description = "Extension version"),
    ),
    responses(
        (status = 200, description = "Extension version metadata", body = UpstreamDocument),
        (status = 404, description = "Unknown registry, extension or version"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/api/{namespace}/{extension}/{version}")]
pub async fn openvsx_extension_version(
    req: HttpRequest,
    path: web::Path<(String, String, String, String)>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
) -> Result<impl Responder, AppError> {
    let (registry, namespace, extension, version) = path.into_inner();
    serve_extension(
        req,
        registry,
        namespace,
        extension,
        Some(version),
        identity,
        svc,
        local_svc,
        map,
        mode_map,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn serve_extension(
    req: HttpRequest,
    registry: String,
    namespace: String,
    extension: String,
    version: Option<String>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
) -> Result<impl Responder, AppError> {
    require_vsx(&registry, &map)?;
    require_single_segment("namespace", &namespace)?;
    require_single_segment("extension name", &extension)?;
    let extension_id = format!("{namespace}.{extension}");

    let entry = source::extension_entry(
        &svc,
        &local_svc,
        mode_map.get(&registry),
        &registry,
        vsx_kind(&registry, &map),
        &extension_id,
        &identity,
    )
    .await?
    .ok_or_else(|| AppError::not_found(format!("extension '{extension_id}' not found")))?;

    // `versions` is newest-first and already filtered, so "no version given"
    // means the newest one a caller may have — not the newest one that exists.
    let selected = match &version {
        Some(v) => entry.versions.iter().find(|x| &x.version == v),
        None => entry.versions.first(),
    }
    .ok_or_else(|| {
        AppError::not_found(match &version {
            Some(v) => format!("extension '{extension_id}' has no version '{v}'"),
            None => format!("extension '{extension_id}' has no available versions"),
        })
    })?;

    let urls = GalleryUrls::new(&registry_public_base(&req, &registry));
    Ok(HttpResponse::Ok()
        .content_type("application/json")
        .json(openvsx_extension_json(
            &entry,
            selected,
            &entry.versions,
            &urls,
        )))
}

/// One file out of an extension — `GET …/api/{ns}/{ext}/{version}/file/{name}`.
///
/// OpenVSX's own download URL shape. `ovsx get` resolves the extension through
/// the metadata route above and then fetches `files.download`, which this
/// server points here.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/api/{namespace}/{extension}/{version}/file/{filename}",
    tag = "proxy/openvsx",
    params(
        ("registry"  = String, Path, description = "Registry name"),
        ("namespace" = String, Path, description = "Extension publisher"),
        ("extension" = String, Path, description = "Extension name"),
        ("version"   = String, Path, description = "Extension version"),
        ("filename"  = String, Path, description = "File name, or a path inside the extension"),
    ),
    responses(
        (status = 200, description = "File bytes", body = ArtifactBytes, content_type = "application/octet-stream"),
        (status = 403, description = "Access denied"),
        (status = 404, description = "No such file"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/api/{namespace}/{extension}/{version}/file/{filename:.*}")]
pub async fn openvsx_file(
    path: web::Path<(String, String, String, String, String)>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
) -> Result<impl Responder, AppError> {
    let (registry, namespace, extension, version, filename) = path.into_inner();
    require_vsx(&registry, &map)?;
    require_single_segment("namespace", &namespace)?;
    require_single_segment("extension name", &extension)?;
    let extension_id = format!("{namespace}.{extension}");

    let bytes = super::assets::vsix_bytes(
        &svc,
        &local_svc,
        &mode_map,
        &registry,
        &extension_id,
        &version,
        &identity,
    )
    .await?;

    // OpenVSX names the package itself `{namespace}.{extension}-{version}.vsix`;
    // anything else is a path inside the archive.
    if filename == format!("{extension_id}-{version}.vsix") || filename.ends_with(".vsix") {
        return Ok(HttpResponse::Ok()
            .content_type("application/octet-stream")
            .body(bytes));
    }

    batlehub_core::services::validate_path_safe("extension file", &filename)
        .map_err(AppError::from)?;
    super::assets::serve_entry(&bytes, &filename)
}
