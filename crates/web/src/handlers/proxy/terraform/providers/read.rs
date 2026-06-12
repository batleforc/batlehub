use super::{
    append_signature_headers, base_url_from_req, collect_storage_stream, get, proxy_stream,
    require_registry_type, tf_provider_binary_storage_key, web, AppError, Arc, AuthIdentity,
    HttpRequest, HttpResponse, LocalRegistryService, PackageId, ProxyService, RegistryMap,
    RegistryMode, RegistryModeMap, Responder, TerraformPlatform,
};

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
    require_registry_type(&registry, "terraform", &map)?;

    let name = format!("providers/{namespace}/{ptype}");
    let mode = mode_map.get(&registry);

    if matches!(mode, RegistryMode::Local | RegistryMode::Hybrid) {
        match local_svc
            .get_tf_provider_versions_response(&registry, &name, &identity)
            .await
        {
            Ok(json) => return Ok(HttpResponse::Ok().json(json)),
            Err(batlehub_core::error::CoreError::NotFound(_)) if mode == RegistryMode::Hybrid => {}
            Err(batlehub_core::error::CoreError::NotFound(msg)) => {
                return Err(AppError::not_found(msg))
            }
            Err(e) => return Err(AppError::from(e)),
        }
    }

    let pkg = PackageId::new(&registry, name, "versions");
    proxy_stream(
        svc,
        pkg,
        identity,
        "releases:read",
        Some("application/json"),
    )
    .await
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
    require_registry_type(&registry, "terraform", &map)?;

    let name = format!("providers/{namespace}/{ptype}");
    let mode = mode_map.get(&registry);

    if matches!(mode, RegistryMode::Local | RegistryMode::Hybrid) {
        let base_url = base_url_from_req(&req);
        if let Some(resp) = try_local_provider_download(
            &local_svc, &registry, &name, &version, &os, &arch, &base_url, &identity, mode,
        )
        .await?
        {
            return Ok(resp);
        }
    }

    let pkg = PackageId::new(&registry, name, &version).with_artifact(format!("{os}/{arch}"));
    proxy_stream(
        svc,
        pkg,
        identity,
        "releases:read",
        Some("application/json"),
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn try_local_provider_download(
    local_svc: &LocalRegistryService,
    registry: &str,
    name: &str,
    version: &str,
    os: &str,
    arch: &str,
    base_url: &str,
    identity: &AuthIdentity,
    mode: RegistryMode,
) -> Result<Option<HttpResponse>, AppError> {
    match local_svc
        .get_tf_provider_download_response(
            registry,
            name,
            version,
            TerraformPlatform { os, arch },
            base_url,
            identity,
        )
        .await
    {
        Ok(json) => {
            let mut resp = HttpResponse::Ok();
            append_signature_headers(&mut resp, local_svc, registry, name, version).await;
            Ok(Some(resp.json(json)))
        }
        Err(batlehub_core::error::CoreError::NotFound(_)) if mode == RegistryMode::Hybrid => {
            Ok(None)
        }
        Err(batlehub_core::error::CoreError::NotFound(msg)) => Err(AppError::not_found(msg)),
        Err(e) => Err(AppError::from(e)),
    }
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
    require_registry_type(&registry, "terraform", &map)?;
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

    local_svc
        .check_prerelease_access(&registry, &version, &identity)
        .await
        .map_err(AppError::from)?;

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

    let buf = collect_storage_stream(artifact.stream).await?;
    Ok(HttpResponse::Ok().content_type("application/zip").body(buf))
}
