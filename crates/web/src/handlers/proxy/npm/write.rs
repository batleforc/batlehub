use std::sync::Arc;

use actix_web::{put, web, HttpResponse, Responder};
use base64::Engine as _;
use bytes::Bytes;
use sha2::{Digest, Sha256};

use batlehub_core::{
    entities::NotificationEventType,
    services::{LocalRegistryService, PublishRequest},
};

use crate::handlers::proxy::common::{
    collect_payload, dispatch_notification, extract_signature_headers, require_local_mode,
};
use crate::{
    error::AppError, extractors::AuthIdentity, services::NotificationService, RegistryMap,
    RegistryModeMap,
};

use super::require_npm;

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
#[allow(clippy::too_many_arguments)]
#[put("/proxy/{registry}/{name}")]
pub async fn npm_publish(
    req: actix_web::HttpRequest,
    path: web::Path<(String, String)>,
    payload: web::Payload,
    identity: AuthIdentity,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    notification_svc: web::Data<Option<Arc<NotificationService>>>,
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

    let tarball_bytes = base64::engine::general_purpose::STANDARD
        .decode(data_b64)
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

    let (signature_bytes, signature_type) = extract_signature_headers(&req);
    let actor = identity.0.user_id.clone().unwrap_or_default();

    let quota = local_svc
        .publish(PublishRequest {
            registry: registry.clone(),
            name: name.clone(),
            version: version_str.clone(),
            artifact: tarball_bytes,
            checksum,
            index_metadata: meta,
            publisher: identity.0.clone(),
            signature_bytes,
            signature_type,
        })
        .await
        .map_err(AppError::from)?;

    dispatch_notification(
        &notification_svc,
        NotificationEventType::PackagePublished,
        &registry,
        &name,
        Some(version_str.clone()),
        &actor,
    );

    let mut resp = HttpResponse::Ok();
    for (k, v) in quota.headers() {
        resp.insert_header((k, v));
    }
    Ok(resp.json(serde_json::json!({})))
}
