//! Marketplace-compatible multipart publish
//! (`POST /api/updates/upload`), so JetBrains' plugin-repository-rest-client /
//! Gradle tooling can publish to local/hybrid registries with a Bearer token.

use std::sync::Arc;

use actix_multipart::Multipart;
use actix_web::{post, web, HttpRequest, Responder};
use bytes::BytesMut;
use futures::StreamExt;
use sha2::{Digest, Sha256};

use batlehub_core::services::{
    validate_package_name, validate_path_safe, LocalRegistryService, PublishRequest,
};

use super::plugin_archive::extract_plugin_descriptor;
use super::require_jbm;
use crate::handlers::proxy::common::{
    extract_signature_headers, publish_and_respond, require_local_mode,
};
use crate::{
    error::AppError, extractors::AuthIdentity, services::NotificationService, RegistryMap,
    RegistryModeMap,
};

/// Publish a plugin archive (local/hybrid registries only).
///
/// Accepts the marketplace upload form: `file` (the `.jar`/`.zip`), an
/// optional `xmlId`/`pluginId` (must match the descriptor when present),
/// `channel`, and `isHidden` (stored as `unlisted`: hidden from listings,
/// downloadable by exact coordinate).
#[utoipa::path(
    post,
    path = "/proxy/{registry}/api/updates/upload",
    tag = "proxy/jetbrains-marketplace",
    params(("registry" = String, Path, description = "Registry name")),
    responses(
        (status = 201, description = "Plugin version published"),
        (status = 400, description = "Malformed upload or xmlId mismatch"),
        (status = 403, description = "Access denied or quota exceeded"),
        (status = 409, description = "Version already published"),
        (status = 422, description = "Archive has no usable plugin descriptor"),
    ),
    security(("bearer_token" = [])),
)]
#[allow(clippy::too_many_arguments)]
#[post("/proxy/{registry}/api/updates/upload")]
pub async fn jbm_upload(
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
    require_jbm(&registry, &map)?;
    require_local_mode(&registry, &mode_map)?;

    let mut form_xml_id: Option<String> = None;
    let mut channel: Option<String> = None;
    let mut is_hidden = false;
    let mut file: Option<bytes::Bytes> = None;
    let mut file_name: Option<String> = None;

    while let Some(field_result) = multipart.next().await {
        let mut field =
            field_result.map_err(|e| AppError::bad_request(format!("multipart error: {e}")))?;
        let field_name = field.name().unwrap_or("").to_owned();
        let disposition_file_name = field
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
            "file" => {
                file_name = disposition_file_name;
                file = Some(bytes);
            }
            // The REST client sends `xmlId`; older tooling sends `pluginId`.
            "xmlId" | "pluginId" => {
                form_xml_id = Some(String::from_utf8_lossy(&bytes).trim().to_owned());
            }
            "channel" => {
                channel = Some(String::from_utf8_lossy(&bytes).trim().to_owned());
            }
            "isHidden" => {
                let v = String::from_utf8_lossy(&bytes).trim().to_ascii_lowercase();
                is_hidden = v == "true" || v == "1";
            }
            _ => {}
        }
    }

    let file = file.ok_or_else(|| AppError::bad_request("missing 'file' field".to_owned()))?;

    let descriptor = extract_plugin_descriptor(&file)?;
    let xml_id = descriptor
        .id
        .clone()
        .ok_or_else(|| AppError::unprocessable("plugin descriptor has no <id>"))?;
    let version = descriptor
        .version
        .clone()
        .ok_or_else(|| AppError::unprocessable("plugin descriptor has no <version>"))?;

    if let Some(form_id) = form_xml_id.as_deref().filter(|s| !s.is_empty()) {
        if form_id != xml_id {
            return Err(AppError::bad_request(format!(
                "xmlId '{form_id}' does not match the descriptor id '{xml_id}'"
            )));
        }
    }

    // Edge validation before the coordinate reaches a storage key.
    validate_package_name(&xml_id).map_err(AppError::from)?;
    validate_path_safe("version", &version).map_err(AppError::from)?;
    let channel = channel.unwrap_or_default();
    if !channel.is_empty() {
        validate_path_safe("channel", &channel).map_err(AppError::from)?;
    }

    let file_name = file_name.unwrap_or_else(|| format!("{xml_id}-{version}.zip"));
    let checksum = hex::encode(Sha256::digest(&file));

    let index_metadata = serde_json::json!({
        "xmlId": xml_id,
        "version": version,
        "name": descriptor.name,
        "description": descriptor.description,
        "vendor": descriptor.vendor,
        "sinceBuild": descriptor.since_build,
        "untilBuild": descriptor.until_build,
        "channel": channel,
        "fileName": file_name,
        "size": file.len(),
        "isHidden": is_hidden,
        "depends": descriptor.depends,
        "changeNotes": descriptor.change_notes,
    });

    let (signature_bytes, signature_type) = extract_signature_headers(&req);

    let response_body = serde_json::json!({
        "id": xml_id,
        "pluginId": xml_id,
        "version": version,
        "channel": channel,
    });

    publish_and_respond(
        &local_svc,
        &notification_svc,
        PublishRequest {
            registry,
            name: xml_id,
            version,
            artifact: file,
            checksum,
            index_metadata,
            // Marketplace `isHidden` maps onto the generic unlisted flag:
            // filtered from every listing, still downloadable by coordinate.
            unlisted: is_hidden,
            publisher: identity.0,
            signature_bytes,
            signature_type,
        },
        actix_web::http::StatusCode::CREATED,
        response_body,
    )
    .await
}
