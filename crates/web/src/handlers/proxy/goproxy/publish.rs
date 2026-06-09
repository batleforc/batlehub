use std::sync::Arc;

use actix_web::{put, web, HttpRequest, HttpResponse, Responder};
use sha2::{Digest, Sha256};

use batlehub_core::services::{LocalRegistryService, PublishRequest};

use crate::handlers::proxy::common::{
    collect_payload, extract_signature_headers, require_local_mode, require_registry_type,
};
use crate::{error::AppError, extractors::AuthIdentity, RegistryMap, RegistryModeMap};

use super::read::extract_go_mod;

/// Publish a Go module version by uploading its zip archive.
///
/// The zip must follow the Go module zip format: all entries prefixed with
/// `{module}@{version}/`. The `go.mod` is extracted automatically from the
/// archive. Version metadata (`.info`) is generated from the version string
/// and the current timestamp.
///
/// The module path is inferred from the URL; the version from the filename
/// (`{version}.zip`).
#[utoipa::path(
    put,
    path = "/proxy/{registry}/{module}/@v/{filename}",
    tag = "proxy/goproxy",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("module"   = String, Path, description = "Go module path (may contain slashes)"),
        ("filename" = String, Path, description = "Version zip: {version}.zip"),
    ),
    request_body(content_type = "application/zip", description = "Go module zip archive"),
    responses(
        (status = 200, description = "Module published"),
        (status = 400, description = "Invalid payload or filename"),
        (status = 403, description = "Access denied"),
        (status = 409, description = "Version already published"),
    ),
    security(("bearer_token" = [])),
)]
#[put("/proxy/{registry}/{module:[^@]+}@v/{filename}")]
pub async fn goproxy_publish(
    req: HttpRequest,
    path: web::Path<(String, String, String)>,
    payload: web::Payload,
    identity: AuthIdentity,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
    local_svc: web::Data<Arc<LocalRegistryService>>,
) -> Result<impl Responder, AppError> {
    let (registry, raw_module, filename) = path.into_inner();
    require_registry_type(&registry, "goproxy", &map)?;
    require_local_mode(&registry, &mode_map)?;
    let module = raw_module.trim_end_matches('/');

    let (version, ext) = filename
        .rsplit_once('.')
        .ok_or_else(|| AppError::bad_request(format!("invalid filename '{filename}'")))?;
    if ext != "zip" {
        return Err(AppError::bad_request(format!(
            "only .zip uploads are supported (got '.{ext}')"
        )));
    }

    let zip_bytes = collect_payload(payload).await?;
    let checksum = hex::encode(Sha256::digest(&zip_bytes));

    let go_mod = extract_go_mod(&zip_bytes, module, version);
    let now = chrono::Utc::now().to_rfc3339();
    let index_metadata = serde_json::json!({
        "Version": version,
        "Time": now,
        "go_mod": go_mod
    });

    let (signature_bytes, signature_type) = extract_signature_headers(&req);

    let quota = local_svc
        .publish(PublishRequest {
            registry,
            name: module.to_owned(),
            version: version.to_owned(),
            artifact: zip_bytes,
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
