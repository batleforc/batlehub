use std::sync::Arc;

use actix_multipart::Multipart;
use actix_web::{get, post, web, HttpRequest, HttpResponse, Responder};
use bytes::BytesMut;
use futures::StreamExt;
use sha2::{Digest, Sha256};

use batlehub_config::schema::RegistryMode;
use batlehub_core::{
    entities::PackageId,
    error::CoreError,
    services::{LocalRegistryService, ProxyService, PublishRequest},
};

use super::common::{
    extract_signature_headers, proxy_stream, require_local_mode, require_registry_type,
};
use crate::{
    error::AppError, extractors::AuthIdentity, services::NotificationService, RegistryMap,
    RegistryModeMap, UpstreamMap,
};
use batlehub_core::entities::NotificationEventType;

use batlehub_config::schema::RegistryMode as Mode;

/// Parse a PyPI distribution filename into `(normalized_name, version)`.
///
/// Handles wheel (`name-version-py-abi-platform.whl`) and sdist
/// (`name-version.tar.gz`, `name-version.zip`) formats.  Returns `None` if
/// the filename cannot be parsed.
pub fn parse_pypi_filename(filename: &str) -> Option<(String, String)> {
    // Strip known extensions to get the stem
    let stem = filename
        .strip_suffix(".whl")
        .or_else(|| filename.strip_suffix(".tar.gz"))
        .or_else(|| filename.strip_suffix(".tar.bz2"))
        .or_else(|| filename.strip_suffix(".zip"))?;

    // Split on '-' and find the first segment that starts with a digit — that's the version
    let parts: Vec<&str> = stem.split('-').collect();
    for i in 1..parts.len() {
        if parts[i].starts_with(|c: char| c.is_ascii_digit()) {
            let name = batlehub_adapters::registry::pypi::normalize_name(&parts[..i].join("-"));
            let version = parts[i].to_owned();
            return Some((name, version));
        }
    }
    None
}

// ── Proxy routes ──────────────────────────────────────────────────────────────

/// Proxy the PyPI Simple Repository API root index.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/simple/",
    tag = "proxy/pypi",
    params(("registry" = String, Path, description = "Registry name")),
    responses(
        (status = 200, description = "Simple index HTML"),
        (status = 404, description = "Registry not found"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/simple/")]
pub async fn pypi_simple_root(
    path: web::Path<String>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    map: web::Data<RegistryMap>,
) -> Result<impl Responder, AppError> {
    let registry = path.into_inner();
    require_registry_type(&registry, "pypi", &map)?;

    // Represent the root index as a special sentinel PackageId.
    let pkg = PackageId::new(&registry, "__simple__", "__root__");
    proxy_stream(svc, pkg, identity, "releases:read", Some("text/html")).await
}

/// Proxy the PyPI Simple Repository API for a specific package, rewriting file
/// URLs so artifacts are downloaded through the batlehub cache.
///
/// In local/hybrid mode, the page is generated from locally-published packages.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/simple/{package}/",
    tag = "proxy/pypi",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("package"  = String, Path, description = "Package name"),
    ),
    responses(
        (status = 200, description = "Simple index page with rewritten file URLs"),
        (status = 404, description = "Package not found"),
    ),
    security(("bearer_token" = [])),
)]
#[allow(clippy::too_many_arguments)]
#[get("/proxy/{registry}/simple/{package}/")]
pub async fn pypi_simple_package(
    path: web::Path<(String, String)>,
    req: HttpRequest,
    identity: AuthIdentity,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
    upstream_map: web::Data<UpstreamMap>,
    http_client: web::Data<reqwest::Client>,
) -> Result<impl Responder, AppError> {
    let (registry, package) = path.into_inner();
    require_registry_type(&registry, "pypi", &map)?;

    let mode = mode_map.get(&registry);
    let normalized = batlehub_adapters::registry::pypi::normalize_name(&package);

    let proxy_base = {
        let conn_info = req.connection_info();
        format!("{}://{}", conn_info.scheme(), conn_info.host())
    };

    if mode == Mode::Local {
        let html = local_svc
            .get_pypi_simple_page(&registry, &normalized, &proxy_base, &identity.0)
            .await
            .map_err(AppError::from)?;
        return Ok(HttpResponse::Ok()
            .content_type("text/html; charset=utf-8")
            .body(html));
    }

    if mode == Mode::Hybrid {
        // Try local first; fall through to upstream if not found.
        match local_svc
            .get_pypi_simple_page(&registry, &normalized, &proxy_base, &identity.0)
            .await
        {
            Ok(html) => {
                return Ok(HttpResponse::Ok()
                    .content_type("text/html; charset=utf-8")
                    .body(html));
            }
            Err(batlehub_core::error::CoreError::NotFound(_)) => {}
            Err(e) => return Err(AppError::from(e)),
        }
    }

    let upstream = upstream_map
        .upstream_for(&registry)
        .ok_or_else(|| AppError::not_found(format!("no upstream for registry '{registry}'")))?;

    let accept = req
        .headers()
        .get(actix_web::http::header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);

    let (body, content_type) = batlehub_adapters::registry::pypi::fetch_simple_page(
        &http_client,
        &upstream,
        &package,
        None,
        accept.as_deref(),
    )
    .await
    .map_err(AppError::from)?;

    let rewritten = batlehub_adapters::registry::pypi::rewrite_simple_page(
        &body,
        content_type.as_deref(),
        &registry,
        &proxy_base,
    );

    let ct = content_type.unwrap_or_else(|| "text/html; charset=utf-8".to_owned());
    Ok(HttpResponse::Ok().content_type(ct).body(rewritten))
}

/// Download a PyPI distribution file through the proxy cache.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/packages/{filename}",
    tag = "proxy/pypi",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("filename" = String, Path, description = "Distribution filename"),
    ),
    responses(
        (status = 200, description = "Distribution bytes"),
        (status = 404, description = "File not found"),
        (status = 422, description = "Cannot parse filename"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/packages/{filename}")]
pub async fn pypi_file_download(
    path: web::Path<(String, String)>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
) -> Result<impl Responder, AppError> {
    let (registry, filename) = path.into_inner();
    require_registry_type(&registry, "pypi", &map)?;

    let mode = mode_map.get(&registry);

    if mode == RegistryMode::Local {
        let (name, version) = parse_pypi_filename(&filename).ok_or_else(|| {
            AppError::unprocessable(format!("cannot parse PyPI filename: {filename}"))
        })?;
        let bytes = local_svc
            .get_artifact(&registry, &name, &version, &identity)
            .await
            .map_err(AppError::from)?;
        return Ok(HttpResponse::Ok()
            .content_type("application/octet-stream")
            .body(bytes));
    }

    if mode == RegistryMode::Hybrid {
        if let Some((name, version)) = parse_pypi_filename(&filename) {
            match local_svc
                .get_artifact(&registry, &name, &version, &identity)
                .await
            {
                Ok(bytes) => {
                    return Ok(HttpResponse::Ok()
                        .content_type("application/octet-stream")
                        .body(bytes));
                }
                Err(CoreError::NotFound(_)) => {}
                Err(e) => return Err(AppError::from(e)),
            }
        }
    }

    let (name, version) = parse_pypi_filename(&filename).ok_or_else(|| {
        AppError::unprocessable(format!("cannot parse PyPI filename: {filename}"))
    })?;

    let pkg = PackageId::new(&registry, &name, &version).with_artifact(filename);
    proxy_stream(
        svc,
        pkg,
        identity,
        "releases:read",
        Some("application/octet-stream"),
    )
    .await
}

// ── Publish route (twine-compatible) ─────────────────────────────────────────

/// Publish a Python distribution (local/hybrid registries only).
///
/// Accepts `multipart/form-data` as produced by `twine upload`.
#[utoipa::path(
    post,
    path = "/proxy/{registry}/legacy/",
    tag = "proxy/pypi",
    params(("registry" = String, Path, description = "Registry name")),
    responses(
        (status = 200, description = "File uploaded"),
        (status = 403, description = "Access denied or quota exceeded"),
        (status = 409, description = "Version already published"),
        (status = 422, description = "Invalid payload"),
    ),
    security(("bearer_token" = [])),
)]
#[allow(clippy::too_many_arguments)]
#[post("/proxy/{registry}/legacy/")]
pub async fn pypi_publish(
    req: HttpRequest,
    path: web::Path<String>,
    mut multipart: Multipart,
    identity: AuthIdentity,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
    notification_svc: web::Data<Option<Arc<NotificationService>>>,
) -> Result<impl Responder, AppError> {
    let registry = path.into_inner();
    require_registry_type(&registry, "pypi", &map)?;
    require_local_mode(&registry, &mode_map)?;

    let mut action: Option<String> = None;
    let mut name: Option<String> = None;
    let mut version: Option<String> = None;
    let mut sha2: Option<String> = None;
    let mut content: Option<bytes::Bytes> = None;
    let mut filename: Option<String> = None;

    while let Some(field_result) = multipart.next().await {
        let mut field =
            field_result.map_err(|e| AppError::bad_request(format!("multipart error: {e}")))?;

        let field_name = field.name().unwrap_or("").to_owned();
        let file_name = field
            .content_disposition()
            .and_then(|cd| cd.get_filename())
            .map(str::to_owned);

        let mut buf = BytesMut::new();
        while let Some(chunk) = field.next().await {
            let chunk = chunk.map_err(|e| AppError::bad_request(format!("chunk error: {e}")))?;
            buf.extend_from_slice(&chunk);
        }
        let bytes = buf.freeze();

        match field_name.as_str() {
            ":action" => action = Some(String::from_utf8_lossy(&bytes).into_owned()),
            "name" => name = Some(String::from_utf8_lossy(&bytes).into_owned()),
            "version" => version = Some(String::from_utf8_lossy(&bytes).into_owned()),
            "sha2" => sha2 = Some(String::from_utf8_lossy(&bytes).into_owned()),
            "content" => {
                filename = file_name;
                content = Some(bytes);
            }
            _ => {}
        }
    }

    let action = action.unwrap_or_default();
    if action != "file_upload" {
        return Err(AppError::bad_request(format!(
            "unsupported :action '{action}'; expected 'file_upload'"
        )));
    }

    let name = name.ok_or_else(|| AppError::bad_request("missing 'name' field".to_owned()))?;
    let version =
        version.ok_or_else(|| AppError::bad_request("missing 'version' field".to_owned()))?;
    let content =
        content.ok_or_else(|| AppError::bad_request("missing 'content' field".to_owned()))?;
    let filename = filename.unwrap_or_else(|| format!("{name}-{version}.tar.gz"));

    let computed_checksum = hex::encode(Sha256::digest(&content));

    if let Some(ref client_sha2) = sha2 {
        if client_sha2 != &computed_checksum {
            return Err(AppError::bad_request("sha2 checksum mismatch".to_owned()));
        }
    }

    let index_metadata = serde_json::json!({
        "name": name,
        "version": version,
        "filename": filename,
        "sha256": computed_checksum,
    });

    let (signature_bytes, signature_type) = extract_signature_headers(&req);
    let actor = identity.0.user_id.clone().unwrap_or_default();

    let quota_check = local_svc
        .publish(PublishRequest {
            registry: registry.clone(),
            name: name.clone(),
            version: version.clone(),
            artifact: content,
            checksum: computed_checksum,
            index_metadata,
            publisher: identity.0,
            signature_bytes,
            signature_type,
        })
        .await
        .map_err(AppError::from)?;

    super::common::dispatch_notification(
        &notification_svc,
        NotificationEventType::PackagePublished,
        &registry,
        &name,
        Some(version),
        &actor,
    );

    let mut resp = HttpResponse::Ok();
    for (header, value) in quota_check.headers() {
        resp.insert_header((header, value));
    }
    Ok(resp.json(serde_json::json!({
        "message": format!("File uploaded: {filename}")
    })))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_wheel_filename() {
        let (name, version) = parse_pypi_filename("requests-2.28.0-py3-none-any.whl").unwrap();
        assert_eq!(name, "requests");
        assert_eq!(version, "2.28.0");
    }

    #[test]
    fn parse_sdist_tar_gz() {
        let (name, version) = parse_pypi_filename("requests-2.28.0.tar.gz").unwrap();
        assert_eq!(name, "requests");
        assert_eq!(version, "2.28.0");
    }

    #[test]
    fn parse_hyphenated_package_name() {
        let (name, version) = parse_pypi_filename("my-cool-package-1.0.0.tar.gz").unwrap();
        assert_eq!(name, "my-cool-package");
        assert_eq!(version, "1.0.0");
    }

    #[test]
    fn parse_invalid_filename_returns_none() {
        assert!(parse_pypi_filename("notapackage.exe").is_none());
    }
}
