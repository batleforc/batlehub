use super::helpers::{metadata_to_index_entry, parse_publish_body};
use super::{
    collect_payload, delete, extract_signature_headers, put, require_cargo, require_local_mode,
    web, AppError, Arc, AuthIdentity, Digest, HttpResponse, LocalRegistryService,
    NotificationEventType, NotificationService, PublishRequest, RegistryMap, RegistryModeMap,
    Responder, Sha256,
};

/// Publish a new crate version (`cargo publish`).
#[utoipa::path(
    put,
    path = "/proxy/{registry}/api/v1/crates/new",
    tag = "proxy/cargo",
    params(("registry" = String, Path, description = "Registry name")),
    request_body(content_type = "application/octet-stream", description = "Cargo publish binary payload (length-prefixed metadata + .crate bytes)"),
    responses(
        (status = 200, description = "Crate published successfully"),
        (status = 400, description = "Invalid publish payload"),
        (status = 403, description = "Access denied"),
        (status = 409, description = "Version already published"),
    ),
    security(("bearer_token" = [])),
)]
#[allow(clippy::too_many_arguments)]
#[put("/proxy/{registry}/api/v1/crates/new")]
pub async fn cargo_publish(
    req: actix_web::HttpRequest,
    path: web::Path<String>,
    payload: web::Payload,
    identity: AuthIdentity,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    notification_svc: web::Data<Option<Arc<NotificationService>>>,
) -> Result<impl Responder, AppError> {
    let registry = path.into_inner();
    require_cargo(&registry, &map)?;
    require_local_mode(&registry, &mode_map)?;

    let body = collect_payload(payload).await?;

    let (meta_json, crate_bytes) = parse_publish_body(body).map_err(AppError::bad_request)?;

    let name = meta_json
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::bad_request("missing 'name' in publish metadata"))?
        .to_owned();
    let version = meta_json
        .get("vers")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::bad_request("missing 'vers' in publish metadata"))?
        .to_owned();

    let checksum = hex::encode(Sha256::digest(&crate_bytes));

    let mut entry =
        metadata_to_index_entry(&meta_json, &checksum).map_err(AppError::bad_request)?;

    // Cargo-specific: validate caller-declared checksum against computed value.
    if !entry.cksum.is_empty() && entry.cksum != checksum {
        return Err(AppError::bad_request(format!(
            "checksum mismatch: declared {} but computed {}",
            entry.cksum, checksum
        )));
    }
    entry.cksum = checksum.clone();

    let index_metadata =
        serde_json::to_value(&entry).map_err(|e| AppError::bad_request(e.to_string()))?;

    let (signature_bytes, signature_type) = extract_signature_headers(&req);
    let actor = identity.0.user_id.clone().unwrap_or_default();

    let quota = local_svc
        .publish(PublishRequest {
            registry: registry.clone(),
            name: name.clone(),
            version: version.clone(),
            artifact: crate_bytes,
            checksum,
            index_metadata,
            publisher: identity.0.clone(),
            signature_bytes,
            signature_type,
        })
        .await
        .map_err(AppError::from)?;

    super::super::common::dispatch_notification(
        &notification_svc,
        NotificationEventType::PackagePublished,
        &registry,
        &name,
        Some(version),
        &actor,
    );

    let mut resp = HttpResponse::Ok();
    for (k, v) in quota.headers() {
        resp.insert_header((k, v));
    }
    Ok(resp.json(serde_json::json!({
        "warnings": {
            "invalid_categories": [],
            "invalid_badges": [],
            "other": []
        }
    })))
}

/// Yank a published crate version.
#[utoipa::path(
    delete,
    path = "/proxy/{registry}/api/v1/crates/{name}/{version}/yank",
    tag = "proxy/cargo",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("name"     = String, Path, description = "Crate name"),
        ("version"  = String, Path, description = "Version"),
    ),
    responses(
        (status = 200, description = "Yanked"),
        (status = 403, description = "Access denied"),
    ),
    security(("bearer_token" = [])),
)]
#[delete("/proxy/{registry}/api/v1/crates/{name}/{version}/yank")]
pub async fn cargo_yank(
    path: web::Path<(String, String, String)>,
    identity: AuthIdentity,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    notification_svc: web::Data<Option<Arc<NotificationService>>>,
) -> Result<impl Responder, AppError> {
    let (registry, name, version) = path.into_inner();
    require_cargo(&registry, &map)?;
    require_local_mode(&registry, &mode_map)?;
    let actor = identity.0.user_id.clone().unwrap_or_default();
    local_svc
        .yank(&registry, &name, &version, &identity.0)
        .await
        .map_err(AppError::from)?;
    super::super::common::dispatch_notification(
        &notification_svc,
        NotificationEventType::PackageYanked,
        &registry,
        &name,
        Some(version),
        &actor,
    );
    Ok(HttpResponse::Ok().json(serde_json::json!({ "ok": true })))
}

/// Unyank a previously yanked crate version.
#[utoipa::path(
    put,
    path = "/proxy/{registry}/api/v1/crates/{name}/{version}/unyank",
    tag = "proxy/cargo",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("name"     = String, Path, description = "Crate name"),
        ("version"  = String, Path, description = "Version"),
    ),
    responses(
        (status = 200, description = "Unyanked"),
        (status = 403, description = "Access denied"),
    ),
    security(("bearer_token" = [])),
)]
#[put("/proxy/{registry}/api/v1/crates/{name}/{version}/unyank")]
pub async fn cargo_unyank(
    path: web::Path<(String, String, String)>,
    identity: AuthIdentity,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    notification_svc: web::Data<Option<Arc<NotificationService>>>,
) -> Result<impl Responder, AppError> {
    let (registry, name, version) = path.into_inner();
    require_cargo(&registry, &map)?;
    require_local_mode(&registry, &mode_map)?;
    let actor = identity.0.user_id.clone().unwrap_or_default();
    local_svc
        .unyank(&registry, &name, &version, &identity.0)
        .await
        .map_err(AppError::from)?;
    super::super::common::dispatch_notification(
        &notification_svc,
        NotificationEventType::PackageUnyanked,
        &registry,
        &name,
        Some(version),
        &actor,
    );
    Ok(HttpResponse::Ok().json(serde_json::json!({ "ok": true })))
}
