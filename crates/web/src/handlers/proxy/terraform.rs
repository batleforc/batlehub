use std::sync::Arc;

use actix_web::{HttpRequest, HttpResponse, Responder, delete, get, post, put, web};
use sha2::{Digest, Sha256};

use batlehub_config::schema::RegistryMode;
use batlehub_core::{
    entities::PackageId,
    ports::StorageMeta,
    services::{LocalRegistryService, PublishRequest, ProxyService, tf_provider_binary_storage_key},
};

use crate::{RegistryMap, RegistryModeMap, UpstreamMap, error::AppError, extractors::AuthIdentity};
use super::common::{
    append_signature_headers, collect_payload, extract_signature_headers, proxy_stream,
    require_local_mode,
};

fn require_terraform(registry: &str, map: &RegistryMap) -> Result<(), AppError> {
    match map.type_of(registry) {
        Some("terraform") => Ok(()),
        Some(_) => Err(AppError::not_found(format!(
            "registry '{registry}' is not a Terraform registry"
        ))),
        None => Err(AppError::not_found(format!("unknown registry '{registry}'"))),
    }
}

fn base_url_from_req(req: &HttpRequest) -> String {
    let info = req.connection_info();
    format!("{}://{}", info.scheme(), info.host())
}

// ── Provider endpoints ────────────────────────────────────────────────────────

/// List available versions for a Terraform provider.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/v1/providers/{namespace}/{ptype}/versions",
    tag = "proxy/terraform",
    params(
        ("registry"  = String, Path, description = "Registry name"),
        ("namespace" = String, Path, description = "Provider namespace"),
        ("ptype"     = String, Path, description = "Provider type"),
    ),
    responses(
        (status = 200, description = "Provider versions JSON"),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Provider not found"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/v1/providers/{namespace}/{ptype}/versions")]
pub async fn tf_provider_versions(
    path: web::Path<(String, String, String)>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
) -> Result<impl Responder, AppError> {
    let (registry, namespace, ptype) = path.into_inner();
    require_terraform(&registry, &map)?;

    let name = format!("providers/{namespace}/{ptype}");
    let mode = mode_map.get(&registry);

    if matches!(mode, RegistryMode::Local | RegistryMode::Hybrid) {
        match local_svc.get_tf_provider_versions_response(&registry, &name, &identity).await {
            Ok(json) => return Ok(HttpResponse::Ok().json(json)),
            Err(batlehub_core::error::CoreError::NotFound(_)) if mode == RegistryMode::Hybrid => {}
            Err(batlehub_core::error::CoreError::NotFound(msg)) => return Err(AppError::not_found(msg)),
            Err(e) => return Err(AppError::from(e)),
        }
    }

    let pkg = PackageId::new(&registry, name, "versions");
    proxy_stream(svc, pkg, identity, "releases:read", Some("application/json")).await
}

/// Get download information for a specific Terraform provider version and platform.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/v1/providers/{namespace}/{ptype}/{version}/download/{os}/{arch}",
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
        (status = 200, description = "Provider download info JSON (includes binary URL and checksums)"),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Provider not found"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/v1/providers/{namespace}/{ptype}/{version}/download/{os}/{arch}")]
pub async fn tf_provider_download(
    path: web::Path<(String, String, String, String, String, String)>,
    req: HttpRequest,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
) -> Result<impl Responder, AppError> {
    let (registry, namespace, ptype, version, os, arch) = path.into_inner();
    require_terraform(&registry, &map)?;

    let name = format!("providers/{namespace}/{ptype}");
    let mode = mode_map.get(&registry);

    if matches!(mode, RegistryMode::Local | RegistryMode::Hybrid) {
        let base_url = base_url_from_req(&req);
        match local_svc
            .get_tf_provider_download_response(&registry, &name, &version, &os, &arch, &base_url, &registry, &identity)
            .await
        {
            Ok(json) => {
                let mut resp = HttpResponse::Ok();
                append_signature_headers(&mut resp, &local_svc, &registry, &name, &version).await;
                return Ok(resp.json(json));
            }
            Err(batlehub_core::error::CoreError::NotFound(_)) if mode == RegistryMode::Hybrid => {}
            Err(batlehub_core::error::CoreError::NotFound(msg)) => return Err(AppError::not_found(msg)),
            Err(e) => return Err(AppError::from(e)),
        }
    }

    let pkg = PackageId::new(&registry, name, &version).with_artifact(format!("{os}/{arch}"));
    proxy_stream(svc, pkg, identity, "releases:read", Some("application/json")).await
}

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
#[post("/proxy/{registry}/v1/providers/{namespace}/{ptype}/versions")]
pub async fn tf_provider_upload(
    req: HttpRequest,
    path: web::Path<(String, String, String)>,
    identity: AuthIdentity,
    payload: web::Payload,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
) -> Result<impl Responder, AppError> {
    let (registry, namespace, ptype) = path.into_inner();
    require_terraform(&registry, &map)?;
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

    let quota_check = local_svc
        .publish(PublishRequest {
            registry: registry.clone(),
            name,
            version,
            artifact: bytes,
            checksum,
            index_metadata,
            publisher: identity.0,
            signature_bytes,
            signature_type,
        })
        .await
        .map_err(AppError::from)?;

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
    require_terraform(&registry, &map)?;
    require_local_mode(&registry, &mode_map)?;
    let _ = &identity; // auth presence validated by middleware

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

/// Download a Terraform provider platform binary from local storage.
#[utoipa::path(
    get,
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
        (status = 200, description = "Provider binary"),
        (status = 404, description = "Binary not found"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/v1/providers/{namespace}/{ptype}/{version}/artifact/{os}/{arch}")]
pub async fn tf_provider_artifact(
    path: web::Path<(String, String, String, String, String, String)>,
    identity: AuthIdentity,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
) -> Result<impl Responder, AppError> {
    let (registry, namespace, ptype, version, os, arch) = path.into_inner();
    require_terraform(&registry, &map)?;

    local_svc.check_prerelease_access(&registry, &version, &identity).await.map_err(AppError::from)?;

    let key = tf_provider_binary_storage_key(&registry, &namespace, &ptype, &version, &os, &arch);
    let artifact = local_svc
        .storage
        .retrieve(&key)
        .await
        .map_err(AppError::from)?
        .ok_or_else(|| {
            AppError::not_found(format!(
                "provider {namespace}/{ptype}@{version} platform {os}/{arch} not found"
            ))
        })?;

    use futures::StreamExt;
    let mut buf = Vec::new();
    let mut stream = artifact.stream;
    while let Some(chunk) = stream.next().await {
        buf.extend_from_slice(&chunk.map_err(|e| AppError::internal(e.to_string()))?);
    }
    Ok(HttpResponse::Ok().content_type("application/zip").body(buf))
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
) -> Result<impl Responder, AppError> {
    let (registry, namespace, ptype, version) = path.into_inner();
    require_terraform(&registry, &map)?;
    require_local_mode(&registry, &mode_map)?;

    let name = format!("providers/{namespace}/{ptype}");
    local_svc
        .yank(&registry, &name, &version, &identity.0)
        .await
        .map_err(AppError::from)?;

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
) -> Result<impl Responder, AppError> {
    let (registry, namespace, ptype, version) = path.into_inner();
    require_terraform(&registry, &map)?;
    require_local_mode(&registry, &mode_map)?;

    let name = format!("providers/{namespace}/{ptype}");
    local_svc
        .unyank(&registry, &name, &version, &identity.0)
        .await
        .map_err(AppError::from)?;

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "message": format!("unyanked provider {namespace}/{ptype}@{version}")
    })))
}

// ── Module endpoints ──────────────────────────────────────────────────────────

/// List available versions for a Terraform module.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/v1/modules/{namespace}/{name}/{provider}/versions",
    tag = "proxy/terraform",
    params(
        ("registry"  = String, Path, description = "Registry name"),
        ("namespace" = String, Path, description = "Module namespace"),
        ("name"      = String, Path, description = "Module name"),
        ("provider"  = String, Path, description = "Module provider"),
    ),
    responses(
        (status = 200, description = "Module versions JSON"),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Module not found"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/v1/modules/{namespace}/{name}/{provider}/versions")]
pub async fn tf_module_versions(
    path: web::Path<(String, String, String, String)>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
) -> Result<impl Responder, AppError> {
    let (registry, namespace, name, provider) = path.into_inner();
    require_terraform(&registry, &map)?;

    let pkg_name = format!("modules/{namespace}/{name}/{provider}");
    let mode = mode_map.get(&registry);

    if matches!(mode, RegistryMode::Local | RegistryMode::Hybrid) {
        match local_svc.get_tf_module_versions_response(&registry, &pkg_name, &identity).await {
            Ok(json) => return Ok(HttpResponse::Ok().json(json)),
            Err(batlehub_core::error::CoreError::NotFound(_)) if mode == RegistryMode::Hybrid => {}
            Err(batlehub_core::error::CoreError::NotFound(msg)) => return Err(AppError::not_found(msg)),
            Err(e) => return Err(AppError::from(e)),
        }
    }

    let pkg = PackageId::new(&registry, pkg_name, "versions");
    proxy_stream(svc, pkg, identity, "releases:read", Some("application/json")).await
}

/// Get the download URL for a specific Terraform module version.
///
/// In local/hybrid mode: returns `204 No Content` with `X-Terraform-Get` pointing at the
/// local artifact endpoint. In proxy mode: forwards to upstream.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/v1/modules/{namespace}/{name}/{provider}/{version}/download",
    tag = "proxy/terraform",
    params(
        ("registry"  = String, Path, description = "Registry name"),
        ("namespace" = String, Path, description = "Module namespace"),
        ("name"      = String, Path, description = "Module name"),
        ("provider"  = String, Path, description = "Module provider"),
        ("version"   = String, Path, description = "Module version"),
    ),
    responses(
        (status = 204, description = "X-Terraform-Get header contains the archive download URL"),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Module not found"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/v1/modules/{namespace}/{name}/{provider}/{version}/download")]
pub async fn tf_module_download(
    path: web::Path<(String, String, String, String, String)>,
    req: HttpRequest,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
    upstream_map: web::Data<UpstreamMap>,
    client: web::Data<reqwest::Client>,
) -> Result<impl Responder, AppError> {
    let (registry, namespace, name, provider, version) = path.into_inner();
    require_terraform(&registry, &map)?;

    let mode = mode_map.get(&registry);
    if matches!(mode, RegistryMode::Local | RegistryMode::Hybrid) {
        let base_url = base_url_from_req(&req);
        let artifact_url = format!(
            "{base_url}/proxy/{registry}/v1/modules/{namespace}/{name}/{provider}/{version}/artifact"
        );
        return Ok(HttpResponse::NoContent()
            .insert_header(("X-Terraform-Get", artifact_url))
            .finish());
    }

    let upstream = upstream_map
        .upstream_for(&registry)
        .ok_or_else(|| AppError::not_found(format!("no upstream configured for '{registry}'")))?;

    let url = format!(
        "{}/v1/modules/{namespace}/{name}/{provider}/{version}/download",
        upstream.trim_end_matches('/')
    );

    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| AppError::internal(format!("terraform upstream request failed: {e}")))?;

    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(AppError::not_found(format!(
            "module {namespace}/{name}/{provider}@{version} not found"
        )));
    }

    if let Some(tf_get) = resp.headers().get("X-Terraform-Get") {
        let header_value = tf_get
            .to_str()
            .map_err(|_| AppError::internal("invalid X-Terraform-Get header from upstream"))?
            .to_owned();
        return Ok(HttpResponse::NoContent()
            .insert_header(("X-Terraform-Get", header_value))
            .finish());
    }

    let status = actix_web::http::StatusCode::from_u16(resp.status().as_u16())
        .unwrap_or(actix_web::http::StatusCode::INTERNAL_SERVER_ERROR);
    let body = resp
        .bytes()
        .await
        .map_err(|e| AppError::internal(format!("reading upstream response: {e}")))?;

    Ok(HttpResponse::build(status)
        .content_type("application/json")
        .body(body))
}

/// Upload a Terraform module tarball to the local registry.
///
/// Accepts a `.tar.gz` archive as the request body.
/// Only available when the registry is in `local` or `hybrid` mode.
/// Optionally accepts `X-Artifact-Signature` (base64) and `X-Signature-Type` headers.
#[utoipa::path(
    post,
    path = "/proxy/{registry}/v1/modules/{namespace}/{name}/{provider}/{version}",
    tag = "proxy/terraform",
    params(
        ("registry"  = String, Path, description = "Registry name"),
        ("namespace" = String, Path, description = "Module namespace"),
        ("name"      = String, Path, description = "Module name"),
        ("provider"  = String, Path, description = "Module provider"),
        ("version"   = String, Path, description = "Module version"),
    ),
    responses(
        (status = 201, description = "Module uploaded"),
        (status = 401, description = "Authentication required"),
        (status = 403, description = "Quota exceeded or ownership denied"),
        (status = 404, description = "Registry not found or not in local/hybrid mode"),
        (status = 409, description = "Version already published"),
        (status = 422, description = "Versioning policy violation"),
    ),
    security(("bearer_token" = [])),
)]
#[post("/proxy/{registry}/v1/modules/{namespace}/{name}/{provider}/{version}")]
pub async fn tf_module_upload(
    req: HttpRequest,
    path: web::Path<(String, String, String, String, String)>,
    identity: AuthIdentity,
    payload: web::Payload,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
) -> Result<impl Responder, AppError> {
    let (registry, namespace, name, provider, version) = path.into_inner();
    require_terraform(&registry, &map)?;
    require_local_mode(&registry, &mode_map)?;

    let bytes = collect_payload(payload).await?;
    let checksum = hex::encode(Sha256::digest(&bytes));
    let index_metadata = serde_json::json!({
        "namespace": namespace,
        "name": name,
        "provider": provider,
        "version": version,
        "sha256": checksum,
        "yanked": false,
    });

    let pkg_name = format!("modules/{namespace}/{name}/{provider}");
    let (signature_bytes, signature_type) = extract_signature_headers(&req);

    let quota_check = local_svc
        .publish(PublishRequest {
            registry: registry.clone(),
            name: pkg_name,
            version,
            artifact: bytes,
            checksum,
            index_metadata,
            publisher: identity.0,
            signature_bytes,
            signature_type,
        })
        .await
        .map_err(AppError::from)?;

    let mut resp = HttpResponse::Created();
    for (header, value) in quota_check.headers() {
        resp.insert_header((header, value));
    }
    Ok(resp.json(serde_json::json!({"message": "module uploaded"})))
}

/// Download the tarball for a locally-published Terraform module.
///
/// This is the target of the `X-Terraform-Get` redirect issued by `tf_module_download`
/// in local/hybrid mode. Returns `X-Artifact-Signature` and `X-Signature-Type` headers
/// if the version was uploaded with a signature.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/v1/modules/{namespace}/{name}/{provider}/{version}/artifact",
    tag = "proxy/terraform",
    params(
        ("registry"  = String, Path, description = "Registry name"),
        ("namespace" = String, Path, description = "Module namespace"),
        ("name"      = String, Path, description = "Module name"),
        ("provider"  = String, Path, description = "Module provider"),
        ("version"   = String, Path, description = "Module version"),
    ),
    responses(
        (status = 200, description = "Module tarball"),
        (status = 404, description = "Module not found"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/v1/modules/{namespace}/{name}/{provider}/{version}/artifact")]
pub async fn tf_module_artifact(
    path: web::Path<(String, String, String, String, String)>,
    identity: AuthIdentity,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
) -> Result<impl Responder, AppError> {
    let (registry, namespace, name, provider, version) = path.into_inner();
    require_terraform(&registry, &map)?;

    local_svc.check_prerelease_access(&registry, &version, &identity).await.map_err(AppError::from)?;

    let pkg_name = format!("modules/{namespace}/{name}/{provider}");
    let bytes = local_svc
        .get_artifact(&registry, &pkg_name, &version, &identity)
        .await
        .map_err(AppError::from)?;

    let mut resp = HttpResponse::Ok();
    append_signature_headers(&mut resp, &local_svc, &registry, &pkg_name, &version).await;
    Ok(resp.content_type("application/gzip").body(bytes))
}

/// Yank a Terraform module version (local/hybrid registries only).
///
/// Yanked versions remain in storage but are hidden from version listings.
#[utoipa::path(
    delete,
    path = "/proxy/{registry}/v1/modules/{namespace}/{name}/{provider}/versions/{version}",
    tag = "proxy/terraform",
    params(
        ("registry"  = String, Path, description = "Registry name"),
        ("namespace" = String, Path, description = "Module namespace"),
        ("name"      = String, Path, description = "Module name"),
        ("provider"  = String, Path, description = "Module provider"),
        ("version"   = String, Path, description = "Module version to yank"),
    ),
    responses(
        (status = 200, description = "Version yanked"),
        (status = 401, description = "Authentication required"),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Version not found"),
    ),
    security(("bearer_token" = [])),
)]
#[delete("/proxy/{registry}/v1/modules/{namespace}/{name}/{provider}/versions/{version}")]
pub async fn tf_module_yank(
    path: web::Path<(String, String, String, String, String)>,
    identity: AuthIdentity,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
) -> Result<impl Responder, AppError> {
    let (registry, namespace, name, provider, version) = path.into_inner();
    require_terraform(&registry, &map)?;
    require_local_mode(&registry, &mode_map)?;

    let pkg_name = format!("modules/{namespace}/{name}/{provider}");
    local_svc
        .yank(&registry, &pkg_name, &version, &identity.0)
        .await
        .map_err(AppError::from)?;

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "message": format!("yanked module {namespace}/{name}/{provider}@{version}")
    })))
}

/// Unyank a Terraform module version (local/hybrid registries only).
#[utoipa::path(
    post,
    path = "/proxy/{registry}/v1/modules/{namespace}/{name}/{provider}/versions/{version}/unyank",
    tag = "proxy/terraform",
    params(
        ("registry"  = String, Path, description = "Registry name"),
        ("namespace" = String, Path, description = "Module namespace"),
        ("name"      = String, Path, description = "Module name"),
        ("provider"  = String, Path, description = "Module provider"),
        ("version"   = String, Path, description = "Module version to unyank"),
    ),
    responses(
        (status = 200, description = "Version unyanked"),
        (status = 401, description = "Authentication required"),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Version not found"),
    ),
    security(("bearer_token" = [])),
)]
#[post("/proxy/{registry}/v1/modules/{namespace}/{name}/{provider}/versions/{version}/unyank")]
pub async fn tf_module_unyank(
    path: web::Path<(String, String, String, String, String)>,
    identity: AuthIdentity,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
) -> Result<impl Responder, AppError> {
    let (registry, namespace, name, provider, version) = path.into_inner();
    require_terraform(&registry, &map)?;
    require_local_mode(&registry, &mode_map)?;

    let pkg_name = format!("modules/{namespace}/{name}/{provider}");
    local_svc
        .unyank(&registry, &pkg_name, &version, &identity.0)
        .await
        .map_err(AppError::from)?;

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "message": format!("unyanked module {namespace}/{name}/{provider}@{version}")
    })))
}
