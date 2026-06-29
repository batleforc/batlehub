use super::{
    base_url, get, post, proxy_stream, require_npm, require_npm_or_cargo,
    serve_local_or_proxy_artifact, web, AppError, Arc, AuthIdentity, CoreError, HttpRequest,
    HttpResponse, LocalOrProxyArtifactOpts, LocalRegistryService, PackageId, ProxyService,
    RegistryMap, RegistryMode, RegistryModeMap, Responder, UpstreamMap,
};

/// Fetch package metadata (all versions / packument).
#[utoipa::path(
    get,
    path = "/proxy/{registry}/{package}",
    tag = "proxy/npm",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("package"  = String, Path, description = "Package / crate name"),
    ),
    responses(
        (status = 200, description = "Package metadata JSON"),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Unknown registry"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/{package}")]
pub async fn get_packument(
    path: web::Path<(String, String)>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
    req: HttpRequest,
) -> Result<impl Responder, AppError> {
    let (registry, package) = path.into_inner();
    require_npm_or_cargo(&registry, &map)?;

    let mode = mode_map.get(&registry);

    if map.is_type(&registry, "npm") && matches!(mode, RegistryMode::Local | RegistryMode::Hybrid) {
        let url = base_url(&req);
        match local_svc
            .get_npm_packument(&registry, &package, &url, &identity)
            .await
        {
            Ok(packument) => {
                return Ok(HttpResponse::Ok().json(packument));
            }
            Err(CoreError::NotFound(_)) if matches!(mode, RegistryMode::Hybrid) => {
                // fall through to proxy
            }
            Err(CoreError::NotFound(_)) => {
                return Err(AppError::not_found(format!(
                    "package '{package}' not found"
                )));
            }
            Err(e) => return Err(AppError::from(e)),
        }
    }

    let pkg = PackageId::new(&registry, &package, "latest");
    proxy_stream(svc, pkg, identity, "releases:read", None).await
}

/// Fetch package version metadata.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/{package}/{version}",
    tag = "proxy/npm",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("package"  = String, Path, description = "Package / crate name"),
        ("version"  = String, Path, description = "Version"),
    ),
    responses(
        (status = 200, description = "Version metadata JSON"),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Unknown registry"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/{package}/{version}")]
pub async fn get_version(
    path: web::Path<(String, String, String)>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
    req: HttpRequest,
) -> Result<impl Responder, AppError> {
    let (registry, package, version) = path.into_inner();
    require_npm_or_cargo(&registry, &map)?;

    let mode = mode_map.get(&registry);

    if map.is_type(&registry, "npm") && matches!(mode, RegistryMode::Local | RegistryMode::Hybrid) {
        let url = base_url(&req);
        match local_svc
            .get_npm_version(&registry, &package, &version, &url, &identity)
            .await
        {
            Ok(meta) => return Ok(HttpResponse::Ok().json(meta)),
            Err(CoreError::NotFound(_)) if matches!(mode, RegistryMode::Hybrid) => {}
            Err(CoreError::NotFound(_)) => {
                return Err(AppError::not_found(format!(
                    "{package}@{version} not found"
                )));
            }
            Err(e) => return Err(AppError::from(e)),
        }
    }

    let pkg = PackageId::new(&registry, &package, &version);
    proxy_stream(svc, pkg, identity, "releases:read", None).await
}

/// Download npm package tarball for a specific version.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/{package}/{version}/tarball",
    tag = "proxy/npm",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("package"  = String, Path, description = "Package name"),
        ("version"  = String, Path, description = "Version"),
    ),
    responses(
        (status = 200, description = "npm .tgz tarball"),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Unknown registry"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/{package}/{version}/tarball")]
pub async fn download_tarball(
    path: web::Path<(String, String, String)>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
) -> Result<impl Responder, AppError> {
    let (registry, package, version) = path.into_inner();
    require_npm(&registry, &map)?;

    serve_local_or_proxy_artifact(
        svc,
        local_svc,
        &mode_map,
        &registry,
        &package,
        &version,
        identity,
        LocalOrProxyArtifactOpts {
            artifact_suffix: "tarball",
            local_content_type: "application/octet-stream",
            proxy_content_type: None,
            resource_type: "source:read",
            check_prerelease: true,
            append_signature: false,
        },
    )
    .await
}

/// Proxy npm audit requests to the upstream npm registry.
#[utoipa::path(
    post,
    path = "/proxy/{registry}/-/npm/v1/audit/quick",
    tag = "proxy/npm",
    params(
        ("registry" = String, Path, description = "npm registry name"),
    ),
    request_body = serde_json::Value,
    responses(
        (status = 200, description = "Audit advisory data from upstream"),
        (status = 404, description = "Unknown or non-npm registry"),
        (status = 502, description = "Upstream audit request failed"),
    ),
    security(("bearer_token" = [])),
)]
#[post("/proxy/{registry}/-/npm/v1/audit/quick")]
pub async fn audit_quick(
    path: web::Path<(String,)>,
    body: web::Json<serde_json::Value>,
    map: web::Data<RegistryMap>,
    upstream_map: web::Data<UpstreamMap>,
    client: web::Data<reqwest::Client>,
) -> Result<impl Responder, AppError> {
    let (registry,) = path.into_inner();
    forward_npm_audit(
        &registry,
        "quick",
        body.into_inner(),
        &map,
        &upstream_map,
        &client,
    )
    .await
}

/// Proxy full npm bulk audit requests (`npm audit` default mode).
#[utoipa::path(
    post,
    path = "/proxy/{registry}/-/npm/v1/audit/bulk",
    tag = "proxy/npm",
    params(
        ("registry" = String, Path, description = "npm registry name"),
    ),
    request_body = serde_json::Value,
    responses(
        (status = 200, description = "Bulk audit advisory data from upstream"),
        (status = 404, description = "Unknown or non-npm registry"),
        (status = 502, description = "Upstream audit request failed"),
    ),
    security(("bearer_token" = [])),
)]
#[post("/proxy/{registry}/-/npm/v1/audit/bulk")]
pub async fn audit_bulk(
    path: web::Path<(String,)>,
    body: web::Json<serde_json::Value>,
    map: web::Data<RegistryMap>,
    upstream_map: web::Data<UpstreamMap>,
    client: web::Data<reqwest::Client>,
) -> Result<impl Responder, AppError> {
    let (registry,) = path.into_inner();
    forward_npm_audit(
        &registry,
        "bulk",
        body.into_inner(),
        &map,
        &upstream_map,
        &client,
    )
    .await
}

async fn forward_npm_audit(
    registry: &str,
    endpoint: &str,
    body: serde_json::Value,
    map: &RegistryMap,
    upstream_map: &UpstreamMap,
    client: &reqwest::Client,
) -> Result<HttpResponse, AppError> {
    require_npm(registry, map)?;

    let upstream = upstream_map
        .upstream_for(registry)
        .ok_or_else(|| AppError::not_found(format!("no upstream configured for '{registry}'")))?;

    let url = format!("{upstream}/-/npm/v1/audit/{endpoint}");

    let resp = client
        .post(&url)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| AppError::bad_gateway(format!("upstream audit request failed: {e}")))?;

    let status = actix_web::http::StatusCode::from_u16(resp.status().as_u16())
        .unwrap_or(actix_web::http::StatusCode::INTERNAL_SERVER_ERROR);
    let response_body = resp
        .bytes()
        .await
        .map_err(|e| AppError::bad_gateway(format!("upstream audit response read failed: {e}")))?;

    Ok(HttpResponse::build(status)
        .content_type("application/json")
        .body(response_body))
}
