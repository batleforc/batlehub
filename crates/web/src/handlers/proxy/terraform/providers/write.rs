use super::{
    collect_payload, delete, extract_signature_headers, post, put, require_local_mode,
    require_registry_type, tf_provider_binary_storage_key, web, AppError, Arc, AuthIdentity,
    Digest, HttpRequest, HttpResponse, LocalRegistryService, NotificationEventType,
    NotificationService, PublishRequest, RegistryMap, RegistryModeMap, Responder, Sha256,
    StorageMeta,
};

/// Upload a Terraform provider version manifest (JSON describing version + platforms).
///
/// Only available when the registry is in `local` or `hybrid` mode.
/// Optionally accepts `X-Artifact-Signature` (base64) and `X-Signature-Type` headers.
/// Platform binaries are uploaded separately via `PUT /artifact/{os}/{arch}`.
#[utoipa::path(
    post,
    path = "/proxy/{registry}/v1/providers/{namespace}/{ptype}/versions",
    tag = "proxy/terraform",
    params(
        ("registry"  = String, Path, description = "Registry name"),
        ("namespace" = String, Path, description = "Provider namespace"),
        ("ptype"     = String, Path, description = "Provider type"),
    ),
    responses(
        (status = 201, description = "Provider version manifest stored"),
        (status = 401, description = "Authentication required"),
        (status = 403, description = "Quota exceeded or ownership denied"),
        (status = 404, description = "Registry not found or not in local/hybrid mode"),
        (status = 409, description = "Version already published"),
        (status = 422, description = "Versioning policy violation"),
    ),
    security(("bearer_token" = [])),
)]
#[allow(clippy::too_many_arguments)]
#[post("/proxy/{registry}/v1/providers/{namespace}/{ptype}/versions")]
pub async fn tf_provider_upload(
    req: HttpRequest,
    path: web::Path<(String, String, String)>,
    identity: AuthIdentity,
    payload: web::Payload,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
    notification_svc: web::Data<Option<Arc<NotificationService>>>,
) -> Result<impl Responder, AppError> {
    let (registry, namespace, ptype) = path.into_inner();
    require_registry_type(&registry, "terraform", &map)?;
    require_local_mode(&registry, &mode_map)?;

    let bytes = collect_payload(payload).await?;

    let manifest: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|e| AppError::bad_request(format!("invalid JSON body: {e}")))?;

    let version = manifest
        .get("version")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::bad_request("manifest missing 'version' field"))?
        .to_owned();

    let checksum = hex::encode(Sha256::digest(&bytes));
    let mut index_metadata = manifest.clone();
    if let Some(obj) = index_metadata.as_object_mut() {
        obj.insert("sha256".to_owned(), serde_json::json!(checksum));
        obj.entry("yanked").or_insert(serde_json::json!(false));
    }

    let name = format!("providers/{namespace}/{ptype}");
    let (signature_bytes, signature_type) = extract_signature_headers(&req);
    let actor = identity.0.user_id.clone().unwrap_or_default();

    let quota_check = local_svc
        .publish(PublishRequest {
            registry: registry.clone(),
            name: name.clone(),
            version: version.clone(),
            artifact: bytes,
            checksum,
            index_metadata,
            publisher: identity.0,
            signature_bytes,
            signature_type,
        })
        .await
        .map_err(AppError::from)?;

    super::super::super::common::dispatch_notification(
        &notification_svc,
        NotificationEventType::PackagePublished,
        &registry,
        &name,
        Some(version),
        &actor,
    );

    let mut resp = HttpResponse::Created();
    for (header, value) in quota_check.headers() {
        resp.insert_header((header, value));
    }
    Ok(resp.json(serde_json::json!({"message": "provider version published"})))
}

/// Upload a platform binary for a locally-published Terraform provider.
#[utoipa::path(
    put,
    path = "/proxy/{registry}/v1/providers/{namespace}/{ptype}/{version}/artifact/{os}/{arch}",
    tag = "proxy/terraform",
    params(
        ("registry"  = String, Path, description = "Registry name"),
        ("namespace" = String, Path, description = "Provider namespace"),
        ("ptype"     = String, Path, description = "Provider type"),
        ("version"   = String, Path, description = "Provider version"),
        ("os"        = String, Path, description = "Target OS"),
        ("arch"      = String, Path, description = "Target architecture"),
    ),
    responses(
        (status = 200, description = "Binary stored"),
        (status = 401, description = "Authentication required"),
        (status = 404, description = "Registry not found or not in local/hybrid mode"),
    ),
    security(("bearer_token" = [])),
)]
#[put("/proxy/{registry}/v1/providers/{namespace}/{ptype}/{version}/artifact/{os}/{arch}")]
pub async fn tf_provider_binary_upload(
    path: web::Path<(String, String, String, String, String, String)>,
    identity: AuthIdentity,
    payload: web::Payload,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
) -> Result<impl Responder, AppError> {
    let (registry, namespace, ptype, version, os, arch) = path.into_inner();
    require_registry_type(&registry, "terraform", &map)?;
    require_local_mode(&registry, &mode_map)?;
    let _ = &identity; // auth presence validated by middleware

    // Edge chokepoint: this handler builds a storage key directly from the path
    // components, so reject any traversal attempt with a clean 400 first.
    for (kind, value) in [
        ("namespace", &namespace),
        ("provider type", &ptype),
        ("version", &version),
        ("os", &os),
        ("arch", &arch),
    ] {
        batlehub_core::services::validate_path_safe(kind, value).map_err(AppError::from)?;
    }

    let bytes = collect_payload(payload).await?;
    let key = tf_provider_binary_storage_key(&registry, &namespace, &ptype, &version, &os, &arch);
    local_svc
        .storage
        .store(
            &key,
            bytes,
            StorageMeta {
                content_type: Some("application/zip".to_owned()),
                size: None,
                checksum: None,
            },
        )
        .await
        .map_err(AppError::from)?;

    Ok(HttpResponse::Ok().finish())
}

/// Yank a Terraform provider version (local/hybrid registries only).
///
/// Yanked versions remain in storage but are hidden from version listings.
#[utoipa::path(
    delete,
    path = "/proxy/{registry}/v1/providers/{namespace}/{ptype}/versions/{version}",
    tag = "proxy/terraform",
    params(
        ("registry"  = String, Path, description = "Registry name"),
        ("namespace" = String, Path, description = "Provider namespace"),
        ("ptype"     = String, Path, description = "Provider type"),
        ("version"   = String, Path, description = "Provider version to yank"),
    ),
    responses(
        (status = 200, description = "Version yanked"),
        (status = 401, description = "Authentication required"),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Version not found"),
    ),
    security(("bearer_token" = [])),
)]
#[delete("/proxy/{registry}/v1/providers/{namespace}/{ptype}/versions/{version}")]
pub async fn tf_provider_yank(
    path: web::Path<(String, String, String, String)>,
    identity: AuthIdentity,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
    notification_svc: web::Data<Option<Arc<NotificationService>>>,
) -> Result<impl Responder, AppError> {
    let (registry, namespace, ptype, version) = path.into_inner();
    require_registry_type(&registry, "terraform", &map)?;
    require_local_mode(&registry, &mode_map)?;

    let name = format!("providers/{namespace}/{ptype}");
    let actor = identity.0.user_id.clone().unwrap_or_default();
    local_svc
        .yank(&registry, &name, &version, &identity.0)
        .await
        .map_err(AppError::from)?;

    super::super::super::common::dispatch_notification(
        &notification_svc,
        NotificationEventType::PackageYanked,
        &registry,
        &name,
        Some(version.clone()),
        &actor,
    );

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "message": format!("yanked provider {namespace}/{ptype}@{version}")
    })))
}

/// Unyank a Terraform provider version (local/hybrid registries only).
#[utoipa::path(
    post,
    path = "/proxy/{registry}/v1/providers/{namespace}/{ptype}/versions/{version}/unyank",
    tag = "proxy/terraform",
    params(
        ("registry"  = String, Path, description = "Registry name"),
        ("namespace" = String, Path, description = "Provider namespace"),
        ("ptype"     = String, Path, description = "Provider type"),
        ("version"   = String, Path, description = "Provider version to unyank"),
    ),
    responses(
        (status = 200, description = "Version unyanked"),
        (status = 401, description = "Authentication required"),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Version not found"),
    ),
    security(("bearer_token" = [])),
)]
#[post("/proxy/{registry}/v1/providers/{namespace}/{ptype}/versions/{version}/unyank")]
pub async fn tf_provider_unyank(
    path: web::Path<(String, String, String, String)>,
    identity: AuthIdentity,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
    notification_svc: web::Data<Option<Arc<NotificationService>>>,
) -> Result<impl Responder, AppError> {
    let (registry, namespace, ptype, version) = path.into_inner();
    require_registry_type(&registry, "terraform", &map)?;
    require_local_mode(&registry, &mode_map)?;

    let name = format!("providers/{namespace}/{ptype}");
    let actor = identity.0.user_id.clone().unwrap_or_default();
    local_svc
        .unyank(&registry, &name, &version, &identity.0)
        .await
        .map_err(AppError::from)?;

    super::super::super::common::dispatch_notification(
        &notification_svc,
        NotificationEventType::PackageUnyanked,
        &registry,
        &name,
        Some(version.clone()),
        &actor,
    );

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "message": format!("unyanked provider {namespace}/{ptype}@{version}")
    })))
}
