use std::sync::Arc;

use actix_multipart::Multipart;
use actix_web::{post, web, HttpRequest, HttpResponse, Responder};
use bytes::BytesMut;
use futures::StreamExt;
use sha2::{Digest, Sha256};

use batlehub_core::{
    entities::NotificationEventType,
    services::{LocalRegistryService, PublishRequest},
};

use crate::handlers::proxy::common::{
    dispatch_notification, extract_signature_headers, require_local_mode, require_registry_type,
};
use crate::{
    error::AppError, extractors::AuthIdentity, services::NotificationService, RegistryMap,
    RegistryModeMap,
};

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

    dispatch_notification(
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
