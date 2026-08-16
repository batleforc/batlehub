//! The OpenVSX native REST API — what `ovsx` and openvsx-native clients speak.
//!
//! A second client protocol over the same [`super::source`] entries, so it
//! hides the same blocked versions and points at the same asset routes as the
//! VS Code gallery does. Only the document shape differs.

use std::sync::Arc;

use actix_web::{get, post, web, HttpRequest, HttpResponse, Responder};

use batlehub_core::services::{LocalRegistryService, ProxyService};
use sha2::Digest as _;

use super::protocol::GalleryQuery;
use super::render::{openvsx_extension_json, openvsx_search_json, GalleryUrls};
use super::{require_single_segment, require_vsx, source, vsx_kind};
use crate::handlers::proxy::common::{collect_payload, registry_public_base, require_local_mode};
use crate::handlers::schemas::{ArtifactBytes, ProtocolDocument, UpstreamDocument};
use crate::{error::AppError, extractors::AuthIdentity, RegistryMap, RegistryModeMap};

/// `ovsx publish` — `POST /api/-/publish`.
///
/// RFC 0009 §7.6 said this was what `ovsx publish` calls and that BatleHub
/// served only `PUT …/{ext}/{version}/vsix`, "which no tool sends". Measured
/// against **ovsx 1.1.1** (§12.6): it sends exactly
/// `POST /api/-/publish?token=…`, and got a `404`.
///
/// Two things the URL does not carry, both of which shape this handler:
///
/// - **No coordinate.** The extension id and version come from the VSIX's own
///   `extension/package.json`. `vsix_publish` prefers the URL when the two
///   disagree; here there is nothing to prefer, so an unreadable manifest is a
///   `400` rather than a degraded publish.
/// - **The token is a query parameter**, not an `Authorization` header. The
///   auth middleware only reads the header, so a bare `ovsx publish` arrives
///   anonymous. That is left to the middleware rather than worked around here —
///   see the note on `PublishToken`.
#[utoipa::path(
    post,
    path = "/proxy/{registry}/api/-/publish",
    tag = "proxy/openvsx",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("token" = Option<String>, Query, description = "Publish token, as `ovsx` sends it"),
    ),
    request_body(content_type = "application/octet-stream", description = "VSIX bytes"),
    responses(
        (status = 201, description = "Extension published", body = ProtocolDocument),
        (status = 400, description = "Not a readable VSIX"),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Unknown registry, or not in local/hybrid mode"),
    ),
    security(("bearer_token" = [])),
)]
#[post("/proxy/{registry}/api/-/publish")]
pub async fn openvsx_publish(
    req: HttpRequest,
    path: web::Path<String>,
    payload: web::Payload,
    identity: AuthIdentity,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
    local_svc: web::Data<Arc<LocalRegistryService>>,
) -> Result<impl Responder, AppError> {
    let registry = path.into_inner();
    require_vsx(&registry, &map)?;
    require_local_mode(&registry, &mode_map)?;

    let vsix_bytes = collect_payload(payload).await?;

    // The coordinate is only in the archive here, so a manifest we cannot read
    // means we do not know what is being published — refuse rather than invent.
    let manifest = super::archive::parse_manifest(&vsix_bytes).ok_or_else(|| {
        AppError::bad_request(
            "could not read extension/package.json from the uploaded VSIX:              /api/-/publish carries no coordinate in its URL, so the manifest is              the only thing that names what is being published"
                .to_owned(),
        )
    })?;

    let extension_id = manifest.extension_id();
    let version = manifest.version.clone();
    let checksum = hex::encode(sha2::Sha256::digest(&vsix_bytes));

    let mut index_metadata = serde_json::json!({
        "id": extension_id,
        "version": version,
        "publisher": manifest.publisher,
    });
    if let Some(obj) = index_metadata.as_object_mut() {
        for (k, v) in [
            ("displayName", manifest.display_name.clone()),
            ("description", manifest.description.clone()),
            ("engine", manifest.engines.vscode.clone()),
            ("icon", manifest.icon.clone()),
        ] {
            if let Some(v) = v.filter(|s| !s.is_empty()) {
                obj.insert(k.to_owned(), serde_json::Value::String(v));
            }
        }
        for (key, list) in [
            ("categories", &manifest.categories),
            ("keywords", &manifest.keywords),
            ("extensionPack", &manifest.extension_pack),
            ("extensionDependencies", &manifest.extension_dependencies),
        ] {
            if !list.is_empty() {
                obj.insert(key.to_owned(), serde_json::json!(list));
            }
        }
    }

    let (signature_bytes, signature_type) =
        crate::handlers::proxy::common::extract_signature_headers(&req);

    let quota = local_svc
        .publish(batlehub_core::services::PublishRequest {
            unlisted: false,
            registry,
            name: extension_id.clone(),
            version: version.clone(),
            artifact: vsix_bytes,
            checksum,
            index_metadata,
            publisher: identity.0.clone(),
            signature_bytes,
            signature_type,
        })
        .await
        .map_err(AppError::from)?;

    let mut resp = HttpResponse::Created();
    for (name, value) in quota.headers() {
        resp.insert_header((name, value));
    }
    Ok(resp.json(serde_json::json!({
        "namespace": manifest.publisher,
        "name": manifest.name,
        "version": version,
        // `ovsx` prints `@{targetPlatform}` for anything that is not
        // `"universal"`, so omitting the field made a successful publish report
        // itself as `Published e2eorg.e2eprobe v1.0.0@undefined` — measured with
        // ovsx 1.1.1 (RFC 0009 §12.14). Publishing a platform-specific VSIX is
        // not supported here, and universal is what it is.
        "targetPlatform": "universal",
    })))
}

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

/// `GET /api/version` — the registry's own version document.
///
/// `ovsx` and the Open VSX web UI read it to decide which API shape they are
/// talking to. Trivial, and its absence makes a client assume the oldest
/// behaviour it supports.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/api/version",
    tag = "proxy/openvsx",
    params(("registry" = String, Path, description = "Registry name")),
    responses(
        (status = 200, description = "Registry API version", body = ProtocolDocument),
        (status = 404, description = "Unknown or non-extension registry"),
    ),
)]
#[get("/proxy/{registry}/api/version")]
pub async fn openvsx_version(
    path: web::Path<String>,
    map: web::Data<RegistryMap>,
) -> Result<impl Responder, AppError> {
    let registry = path.into_inner();
    require_vsx(&registry, &map)?;
    Ok(HttpResponse::Ok()
        .content_type("application/json")
        .json(serde_json::json!({
            "version": env!("CARGO_PKG_VERSION"),
        })))
}

/// `GET /api/{namespace}` — what a publisher has here.
///
/// A namespace document, not an extension one: `ovsx` reads it to check a
/// namespace exists before publishing into it, and the web UI lists a
/// publisher's extensions from it.
///
/// Built from the same filtered entry list the gallery and the extension
/// document use, so a blocked version cannot be visible through this route
/// while hidden in the other two.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/api/{namespace}",
    tag = "proxy/openvsx",
    params(
        ("registry"  = String, Path, description = "Registry name"),
        ("namespace" = String, Path, description = "Publisher namespace"),
    ),
    responses(
        (status = 200, description = "Namespace document", body = ProtocolDocument),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Unknown registry or namespace"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/api/{namespace}")]
pub async fn openvsx_namespace(
    req: HttpRequest,
    path: web::Path<(String, String)>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
) -> Result<impl Responder, AppError> {
    let (registry, namespace) = path.into_inner();
    require_vsx(&registry, &map)?;
    require_single_segment("namespace", &namespace)?;

    let base = registry_public_base(&req, &registry);

    // A namespace listing is a search restricted to one publisher, so it goes
    // through the same source — one filter, three documents that agree.
    let query = GalleryQuery {
        search_text: Some(namespace.clone()),
        page_size: 100,
        ..Default::default()
    };
    let (entries, _total) = source::search_entries(
        &svc,
        &local_svc,
        mode_map.get(&registry),
        &registry,
        vsx_kind(&registry, &map),
        &query,
        &identity,
    )
    .await?;

    let extensions: serde_json::Map<String, serde_json::Value> = entries
        .iter()
        .filter(|e| e.publisher.eq_ignore_ascii_case(&namespace))
        .map(|e| {
            (
                e.extension_name.clone(),
                serde_json::json!(format!(
                    "{}/api/{}/{}",
                    base.trim_end_matches('/'),
                    e.publisher,
                    e.extension_name
                )),
            )
        })
        .collect();

    if extensions.is_empty() {
        return Err(AppError::not_found(format!(
            "namespace '{namespace}' has no extensions in this registry"
        )));
    }

    Ok(HttpResponse::Ok()
        .content_type("application/json")
        .json(serde_json::json!({
            "name": namespace,
            "extensions": extensions,
            // BatleHub has no namespace-ownership model of its own, and saying
            // a namespace is verified when nothing verified it would be a claim
            // about provenance we cannot back.
            "verified": false,
        })))
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
