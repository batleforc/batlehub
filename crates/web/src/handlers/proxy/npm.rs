use std::sync::Arc;

use actix_web::{HttpRequest, HttpResponse, Responder, get, post, put, web};
use base64::Engine as _;
use bytes::Bytes;
use sha2::{Digest, Sha256};

use batlehub_config::schema::RegistryMode;
use batlehub_core::{
    entities::PackageId,
    error::CoreError,
    services::{LocalRegistryService, ProxyService, PublishRequest},
};

use crate::{RegistryMap, RegistryModeMap, UpstreamMap, error::AppError, extractors::AuthIdentity};
use super::common::{collect_payload, proxy_stream, require_local_mode};

fn require_npm_or_cargo(registry: &str, map: &RegistryMap) -> Result<(), AppError> {
    match map.type_of(registry) {
        Some("npm") | Some("cargo") | Some("openvsx") => Ok(()),
        Some(_) => Err(AppError::not_found(format!(
            "registry '{registry}' is not an npm, cargo, or openvsx registry"
        ))),
        None => Err(AppError::not_found(format!("unknown registry '{registry}'"))),
    }
}

fn require_npm(registry: &str, map: &RegistryMap) -> Result<(), AppError> {
    match map.type_of(registry) {
        Some("npm") => Ok(()),
        Some(_) => Err(AppError::not_found(format!("registry '{registry}' is not an npm registry"))),
        None => Err(AppError::not_found(format!("unknown registry '{registry}'"))),
    }
}

fn base_url(req: &HttpRequest) -> String {
    let info = req.connection_info();
    format!("{}://{}", info.scheme(), info.host())
}

/// Fetch package metadata (all versions / packument).
#[utoipa::path(
    get,
    path = "/proxy/{registry}/{package}",
    tag = "proxy/npm",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("package"  = String, Path, description = "Package / crate name"),
    ),
    responses(
        (status = 200, description = "Package metadata JSON"),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Unknown registry"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/{package}")]
pub async fn get_packument(
    path: web::Path<(String, String)>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
    req: HttpRequest,
) -> Result<impl Responder, AppError> {
    let (registry, package) = path.into_inner();
    require_npm_or_cargo(&registry, &map)?;

    let mode = mode_map.get(&registry);

    if map.is_type(&registry, "npm") && matches!(mode, RegistryMode::Local | RegistryMode::Hybrid) {
        let url = base_url(&req);
        match local_svc.get_npm_packument(&registry, &package, &url).await {
            Ok(packument) => {
                return Ok(HttpResponse::Ok().json(packument));
            }
            Err(CoreError::NotFound(_)) if matches!(mode, RegistryMode::Hybrid) => {
                // fall through to proxy
            }
            Err(CoreError::NotFound(_)) => {
                return Err(AppError::not_found(format!("package '{package}' not found")));
            }
            Err(e) => return Err(AppError::from(e)),
        }
    }

    let pkg = PackageId::new(&registry, &package, "latest");
    proxy_stream(svc, pkg, identity, "releases:read", None).await
}

/// Fetch package version metadata.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/{package}/{version}",
    tag = "proxy/npm",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("package"  = String, Path, description = "Package / crate name"),
        ("version"  = String, Path, description = "Version"),
    ),
    responses(
        (status = 200, description = "Version metadata JSON"),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Unknown registry"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/{package}/{version}")]
pub async fn get_version(
    path: web::Path<(String, String, String)>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
    req: HttpRequest,
) -> Result<impl Responder, AppError> {
    let (registry, package, version) = path.into_inner();
    require_npm_or_cargo(&registry, &map)?;

    let mode = mode_map.get(&registry);

    if map.is_type(&registry, "npm") && matches!(mode, RegistryMode::Local | RegistryMode::Hybrid) {
        let url = base_url(&req);
        match local_svc.get_npm_version(&registry, &package, &version, &url).await {
            Ok(meta) => return Ok(HttpResponse::Ok().json(meta)),
            Err(CoreError::NotFound(_)) if matches!(mode, RegistryMode::Hybrid) => {}
            Err(CoreError::NotFound(_)) => {
                return Err(AppError::not_found(format!("{package}@{version} not found")));
            }
            Err(e) => return Err(AppError::from(e)),
        }
    }

    let pkg = PackageId::new(&registry, &package, &version);
    proxy_stream(svc, pkg, identity, "releases:read", None).await
}

/// Download npm package tarball for a specific version.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/{package}/{version}/tarball",
    tag = "proxy/npm",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("package"  = String, Path, description = "Package name"),
        ("version"  = String, Path, description = "Version"),
    ),
    responses(
        (status = 200, description = "npm .tgz tarball"),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Unknown registry"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/{package}/{version}/tarball")]
pub async fn download_tarball(
    path: web::Path<(String, String, String)>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
) -> Result<impl Responder, AppError> {
    let (registry, package, version) = path.into_inner();
    require_npm(&registry, &map)?;

    let mode = mode_map.get(&registry);

    if matches!(mode, RegistryMode::Local) {
        let bytes = local_svc
            .get_artifact(&registry, &package, &version)
            .await
            .map_err(AppError::from)?;
        return Ok(HttpResponse::Ok()
            .content_type("application/octet-stream")
            .body(bytes));
    }

    if matches!(mode, RegistryMode::Hybrid) {
        match local_svc.get_artifact(&registry, &package, &version).await {
            Ok(bytes) => {
                return Ok(HttpResponse::Ok()
                    .content_type("application/octet-stream")
                    .body(bytes));
            }
            Err(CoreError::NotFound(_)) => {}
            Err(e) => return Err(AppError::from(e)),
        }
    }

    let pkg = PackageId::new(&registry, &package, &version).with_artifact("tarball");
    proxy_stream(svc, pkg, identity, "source:read", None).await
}

/// Publish a new npm package version (`npm publish`).
///
/// Accepts the standard npm publish wire format: a JSON body containing the
/// package metadata under `versions` and the base64-encoded tarball under
/// `_attachments`.
#[utoipa::path(
    put,
    path = "/proxy/{registry}/{name}",
    tag = "proxy/npm",
    params(("registry" = String, Path, description = "Registry name"),
           ("name" = String, Path, description = "Package name")),
    request_body(content_type = "application/json", description = "npm publish payload"),
    responses(
        (status = 200, description = "Package published"),
        (status = 400, description = "Invalid payload"),
        (status = 403, description = "Access denied"),
        (status = 409, description = "Version already published"),
    ),
    security(("bearer_token" = [])),
)]
#[put("/proxy/{registry}/{name}")]
pub async fn npm_publish(
    path: web::Path<(String, String)>,
    payload: web::Payload,
    identity: AuthIdentity,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
    local_svc: web::Data<Arc<LocalRegistryService>>,
) -> Result<impl Responder, AppError> {
    let (registry, name) = path.into_inner();
    require_npm(&registry, &map)?;
    require_local_mode(&registry, &mode_map)?;

    let raw = collect_payload(payload).await?;

    let body: serde_json::Value = serde_json::from_slice(&raw)
        .map_err(|e| AppError::bad_request(format!("invalid JSON: {e}")))?;

    // npm publish sends exactly one version per request.
    let versions = body
        .get("versions")
        .and_then(|v| v.as_object())
        .ok_or_else(|| AppError::bad_request("missing 'versions' object"))?;
    let (version_str, version_meta) = versions
        .iter()
        .next()
        .ok_or_else(|| AppError::bad_request("'versions' is empty"))?;

    let attachments = body
        .get("_attachments")
        .and_then(|a| a.as_object())
        .ok_or_else(|| AppError::bad_request("missing '_attachments'"))?;
    let (_filename, attachment) = attachments
        .iter()
        .next()
        .ok_or_else(|| AppError::bad_request("'_attachments' is empty"))?;

    let data_b64 = attachment
        .get("data")
        .and_then(|d| d.as_str())
        .ok_or_else(|| AppError::bad_request("missing 'data' in attachment"))?;

    let tarball_bytes =
        base64::engine::general_purpose::STANDARD.decode(data_b64)
            .map_err(|e| AppError::bad_request(format!("invalid base64 in attachment: {e}")))?;
    let tarball_bytes = Bytes::from(tarball_bytes);

    let checksum = hex::encode(Sha256::digest(&tarball_bytes));

    // Strip the tarball URL — it will be rewritten dynamically when serving.
    let mut meta = version_meta.clone();
    if let Some(obj) = meta.as_object_mut() {
        if let Some(dist) = obj.get_mut("dist").and_then(|d| d.as_object_mut()) {
            dist.remove("tarball");
        }
    }

    let quota = local_svc
        .publish(PublishRequest {
            registry,
            name,
            version: version_str.clone(),
            artifact: tarball_bytes,
            checksum,
            index_metadata: meta,
            publisher: identity.0.clone(),
        })
        .await
        .map_err(AppError::from)?;

    let mut resp = HttpResponse::Ok();
    for (name, value) in quota.headers() {
        resp.insert_header((name, value));
    }
    Ok(resp.json(serde_json::json!({})))
}

/// Proxy npm audit requests to the upstream npm registry.
#[utoipa::path(
    post,
    path = "/proxy/{registry}/-/npm/v1/audit/quick",
    tag = "proxy/npm",
    params(
        ("registry" = String, Path, description = "npm registry name"),
    ),
    request_body = serde_json::Value,
    responses(
        (status = 200, description = "Audit advisory data from upstream"),
        (status = 404, description = "Unknown or non-npm registry"),
        (status = 502, description = "Upstream audit request failed"),
    ),
    security(("bearer_token" = [])),
)]
#[post("/proxy/{registry}/-/npm/v1/audit/quick")]
pub async fn audit_quick(
    path: web::Path<(String,)>,
    body: web::Json<serde_json::Value>,
    map: web::Data<RegistryMap>,
    upstream_map: web::Data<UpstreamMap>,
    client: web::Data<reqwest::Client>,
) -> Result<impl Responder, AppError> {
    let (registry,) = path.into_inner();
    require_npm(&registry, &map)?;

    let upstream = upstream_map
        .upstream_for(&registry)
        .ok_or_else(|| AppError::not_found(format!("no upstream configured for '{registry}'")))?;

    let url = format!("{upstream}/-/npm/v1/audit/quick");

    let resp = client
        .post(&url)
        .header("Content-Type", "application/json")
        .json(&body.into_inner())
        .send()
        .await
        .map_err(|e| AppError::internal(format!("upstream audit request failed: {e}")))?;

    let status = actix_web::http::StatusCode::from_u16(resp.status().as_u16())
        .unwrap_or(actix_web::http::StatusCode::INTERNAL_SERVER_ERROR);
    let response_body = resp
        .bytes()
        .await
        .map_err(|e| AppError::internal(format!("upstream audit response read failed: {e}")))?;

    Ok(HttpResponse::build(status)
        .content_type("application/json")
        .body(response_body))
}
