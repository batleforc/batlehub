use std::sync::Arc;

use actix_web::{get, put, web, HttpRequest, HttpResponse, Responder};
use sha2::{Digest, Sha256};

use batlehub_config::schema::RegistryMode;
use batlehub_core::{
    entities::PackageId,
    error::CoreError,
    services::{LocalRegistryService, ProxyService, PublishRequest},
};

use super::common::{collect_payload, extract_signature_headers, proxy_stream, require_local_mode};
use crate::{error::AppError, extractors::AuthIdentity, RegistryMap, RegistryModeMap};

pub fn require_openvsx(registry: &str, map: &RegistryMap) -> Result<(), AppError> {
    match map.type_of(registry) {
        Some("openvsx") | Some("vscode-marketplace") => Ok(()),
        Some(_) => Err(AppError::not_found(format!(
            "registry '{registry}' is not an openvsx or vscode-marketplace registry"
        ))),
        None => Err(AppError::not_found(format!(
            "unknown registry '{registry}'"
        ))),
    }
}

/// Download a VS Code extension VSIX package.
///
/// Extension IDs follow the `{publisher}.{name}` convention (e.g. `ms-python.python`).
#[utoipa::path(
    get,
    path = "/proxy/{registry}/{extension_id}/{version}/vsix",
    tag = "proxy/openvsx",
    params(
        ("registry"     = String, Path, description = "Registry name"),
        ("extension_id" = String, Path, description = "Extension ID in publisher.name format"),
        ("version"      = String, Path, description = "Version"),
    ),
    responses(
        (status = 200, description = "VS Code extension VSIX package"),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Unknown registry or extension"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/{extension_id}/{version}/vsix")]
pub async fn download_vsix(
    path: web::Path<(String, String, String)>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
) -> Result<impl Responder, AppError> {
    let (registry, extension_id, version) = path.into_inner();
    require_openvsx(&registry, &map)?;

    let mode = mode_map.get(&registry);

    if matches!(mode, RegistryMode::Local) {
        local_svc
            .check_prerelease_access(&registry, &version, &identity)
            .await
            .map_err(AppError::from)?;
        let bytes = local_svc
            .get_artifact(&registry, &extension_id, &version, &identity)
            .await
            .map_err(AppError::from)?;
        return Ok(HttpResponse::Ok()
            .content_type("application/octet-stream")
            .body(bytes));
    }

    if matches!(mode, RegistryMode::Hybrid) {
        if let Err(e) = local_svc
            .check_prerelease_access(&registry, &version, &identity)
            .await
        {
            if !matches!(e, CoreError::NotFound(_)) {
                return Err(AppError::from(e));
            }
            // pre-release gated; fall through to proxy
        } else {
            match local_svc
                .get_artifact(&registry, &extension_id, &version, &identity)
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

    let pkg = PackageId::new(&registry, &extension_id, &version).with_artifact("vsix");
    proxy_stream(svc, pkg, identity, "source:read", None).await
}

/// Upload a VS Code extension VSIX package.
///
/// Accepts raw VSIX bytes (ZIP archive). The extension ID follows the
/// `{publisher}.{name}` convention (e.g. `my-org.my-extension`).
#[utoipa::path(
    put,
    path = "/proxy/{registry}/{extension_id}/{version}/vsix",
    tag = "proxy/openvsx",
    params(
        ("registry"     = String, Path, description = "Registry name"),
        ("extension_id" = String, Path, description = "Extension ID in publisher.name format"),
        ("version"      = String, Path, description = "Version"),
    ),
    request_body(content_type = "application/octet-stream", description = "Raw VSIX bytes"),
    responses(
        (status = 200, description = "Extension published"),
        (status = 400, description = "Invalid payload"),
        (status = 403, description = "Access denied"),
        (status = 409, description = "Version already published"),
    ),
    security(("bearer_token" = [])),
)]
#[put("/proxy/{registry}/{extension_id}/{version}/vsix")]
pub async fn vsix_publish(
    req: HttpRequest,
    path: web::Path<(String, String, String)>,
    payload: web::Payload,
    identity: AuthIdentity,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
    local_svc: web::Data<Arc<LocalRegistryService>>,
) -> Result<impl Responder, AppError> {
    let (registry, extension_id, version) = path.into_inner();
    require_openvsx(&registry, &map)?;
    require_local_mode(&registry, &mode_map)?;

    let vsix_bytes = collect_payload(payload).await?;
    let checksum = hex::encode(Sha256::digest(&vsix_bytes));

    let publisher = extension_id
        .split('.')
        .next()
        .unwrap_or(&extension_id)
        .to_owned();
    let index_metadata = serde_json::json!({
        "id": extension_id,
        "version": version,
        "publisher": publisher
    });

    let (signature_bytes, signature_type) = extract_signature_headers(&req);

    let quota = local_svc
        .publish(PublishRequest {
            registry,
            name: extension_id,
            version,
            artifact: vsix_bytes,
            checksum,
            index_metadata,
            publisher: identity.0.clone(),
            signature_bytes,
            signature_type,
        })
        .await
        .map_err(AppError::from)?;

    let mut resp = HttpResponse::Ok();
    for (name, value) in quota.headers() {
        resp.insert_header((name, value));
    }
    Ok(resp.json(serde_json::json!({ "ok": true })))
}
