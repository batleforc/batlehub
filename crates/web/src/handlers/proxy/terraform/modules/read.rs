use batlehub_core::error::CoreError;

use super::{
    append_signature_headers, base_url_from_req, get, require_local_mode, require_registry_type,
    terraform_versions_response, web, AppError, Arc, AuthIdentity, HttpRequest, HttpResponse,
    LocalRegistryService, ProxyService, RegistryMap, RegistryMode, RegistryModeMap, Responder,
    UpstreamMap,
};

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
pub async fn terraform_module_versions(
    path: web::Path<(String, String, String, String)>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
) -> Result<impl Responder, AppError> {
    let (registry, namespace, name, provider) = path.into_inner();
    require_registry_type(&registry, "terraform", &map)?;

    let pkg_name = format!("modules/{namespace}/{name}/{provider}");
    let mode = mode_map.get(&registry);

    let local_result = if matches!(mode, RegistryMode::Local | RegistryMode::Hybrid) {
        Some(
            local_svc
                .get_terraform_module_versions_response(&registry, &pkg_name, &identity)
                .await,
        )
    } else {
        None
    };

    terraform_versions_response(&registry, pkg_name, identity, svc, mode, local_result).await
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
pub async fn terraform_module_download(
    path: web::Path<(String, String, String, String, String)>,
    req: HttpRequest,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
    upstream_map: web::Data<UpstreamMap>,
    client: web::Data<reqwest::Client>,
) -> Result<impl Responder, AppError> {
    let (registry, namespace, name, provider, version) = path.into_inner();
    require_registry_type(&registry, "terraform", &map)?;

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
        .map_err(|e| CoreError::Registry(format!("terraform upstream request failed: {e}")))?;

    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(AppError::not_found(format!(
            "module {namespace}/{name}/{provider}@{version} not found"
        )));
    }

    if let Some(tf_get) = resp.headers().get("X-Terraform-Get") {
        let header_value = tf_get
            .to_str()
            .map_err(|_| {
                CoreError::Registry("invalid X-Terraform-Get header from upstream".to_string())
            })?
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
        .map_err(|e| CoreError::Registry(format!("reading upstream response: {e}")))?;

    Ok(HttpResponse::build(status)
        .content_type("application/json")
        .body(body))
}

/// Download the tarball for a locally-published Terraform module.
///
/// This is the target of the `X-Terraform-Get` redirect issued by `terraform_module_download`
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
pub async fn terraform_module_artifact(
    path: web::Path<(String, String, String, String, String)>,
    identity: AuthIdentity,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
) -> Result<impl Responder, AppError> {
    let (registry, namespace, name, provider, version) = path.into_inner();
    require_registry_type(&registry, "terraform", &map)?;
    require_local_mode(&registry, &mode_map)?;

    local_svc
        .check_prerelease_access(&registry, &version, &identity)
        .await
        .map_err(AppError::from)?;

    let pkg_name = format!("modules/{namespace}/{name}/{provider}");
    let bytes = local_svc
        .get_artifact(&registry, &pkg_name, &version, &identity)
        .await
        .map_err(AppError::from)?;

    let mut resp = HttpResponse::Ok();
    append_signature_headers(&mut resp, &local_svc, &registry, &pkg_name, &version).await;
    Ok(resp.content_type("application/gzip").body(bytes))
}
