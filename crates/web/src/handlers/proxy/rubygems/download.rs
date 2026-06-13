use super::{
    append_signature_headers, get, proxy_stream, require_registry_type, web, AppError, Arc,
    AuthIdentity, HttpResponse, LocalRegistryService, PackageId, ProxyService, RegistryMap,
    RegistryMode, RegistryModeMap, Responder,
};

/// Download a gem file.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/gems/{filename}",
    tag = "proxy/rubygems",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("filename" = String, Path, description = "Gem filename, e.g. rails-7.1.0.gem"),
    ),
    responses(
        (status = 200, description = "Gem binary"),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Gem not found"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/gems/{filename}")]
pub async fn gem_download(
    path: web::Path<(String, String)>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
) -> Result<impl Responder, AppError> {
    let (registry, filename) = path.into_inner();
    require_registry_type(&registry, "rubygems", &map)?;

    let stem = filename
        .strip_suffix(".gem")
        .ok_or_else(|| AppError::bad_request(format!("invalid gem filename: {filename}")))?;

    let (name, version) = batlehub_adapters::registry::rubygems::split_gem_stem(stem)
        .ok_or_else(|| AppError::bad_request(format!("cannot parse gem filename: {filename}")))?;

    let mode = mode_map.get(&registry);
    if matches!(mode, RegistryMode::Local) {
        local_svc
            .check_prerelease_access(&registry, version, &identity)
            .await
            .map_err(AppError::from)?;
        let bytes = local_svc
            .get_artifact(&registry, name, version, &identity)
            .await
            .map_err(AppError::from)?;
        let mut resp = HttpResponse::Ok();
        resp.content_type("application/octet-stream");
        append_signature_headers(&mut resp, &local_svc, &registry, name, version).await;
        return Ok(resp.body(bytes));
    }

    if matches!(mode, RegistryMode::Hybrid) {
        // Gate must be enforced before falling through to upstream: a non-member
        // must not receive a pre-release artifact from the upstream registry.
        local_svc
            .check_prerelease_access(&registry, version, &identity)
            .await
            .map_err(AppError::from)?;
        match local_svc
            .get_artifact(&registry, name, version, &identity)
            .await
        {
            Ok(bytes) => {
                let mut resp = HttpResponse::Ok();
                resp.content_type("application/octet-stream");
                append_signature_headers(&mut resp, &local_svc, &registry, name, version).await;
                return Ok(resp.body(bytes));
            }
            Err(batlehub_core::error::CoreError::NotFound(_)) => {} // not found locally; fall through to upstream
            Err(e) => return Err(AppError::from(e)),
        }
    }

    let pkg = PackageId::new(&registry, name, version).with_artifact("gem");
    proxy_stream(
        svc,
        pkg,
        identity,
        "releases:read",
        Some("application/octet-stream"),
    )
    .await
}

/// Get gem information JSON (latest version).
#[utoipa::path(
    get,
    path = "/proxy/{registry}/api/v1/gems/{name}.json",
    tag = "proxy/rubygems",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("name"     = String, Path, description = "Gem name"),
    ),
    responses(
        (status = 200, description = "Gem info JSON"),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Gem not found"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/api/v1/gems/{name}.json")]
pub async fn gem_info(
    path: web::Path<(String, String)>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
) -> Result<impl Responder, AppError> {
    let (registry, name) = path.into_inner();
    require_registry_type(&registry, "rubygems", &map)?;

    let mode = mode_map.get(&registry);
    if matches!(mode, RegistryMode::Local | RegistryMode::Hybrid) {
        match local_svc
            .get_rubygems_gem_info(&registry, &name, &identity)
            .await
        {
            Ok(info) => return Ok(HttpResponse::Ok().json(info)),
            Err(batlehub_core::error::CoreError::NotFound(_))
                if matches!(mode, RegistryMode::Hybrid) => {}
            Err(batlehub_core::error::CoreError::NotFound(_)) => {
                return Err(AppError::not_found(format!("gem '{name}' not found")));
            }
            Err(e) => return Err(AppError::from(e)),
        }
    }

    let pkg = PackageId::new(&registry, &name, "info");
    proxy_stream(
        svc,
        pkg,
        identity,
        "releases:read",
        Some("application/json"),
    )
    .await
}

/// List all versions of a gem.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/api/v1/versions/{name}.json",
    tag = "proxy/rubygems",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("name"     = String, Path, description = "Gem name"),
    ),
    responses(
        (status = 200, description = "Gem versions JSON array"),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Gem not found"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/api/v1/versions/{name}.json")]
pub async fn gem_versions(
    path: web::Path<(String, String)>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
) -> Result<impl Responder, AppError> {
    let (registry, name) = path.into_inner();
    require_registry_type(&registry, "rubygems", &map)?;

    let mode = mode_map.get(&registry);
    if matches!(mode, RegistryMode::Local | RegistryMode::Hybrid) {
        match local_svc
            .get_rubygems_versions(&registry, &name, &identity)
            .await
        {
            Ok(versions) => return Ok(HttpResponse::Ok().json(versions)),
            Err(batlehub_core::error::CoreError::NotFound(_))
                if matches!(mode, RegistryMode::Hybrid) => {}
            Err(batlehub_core::error::CoreError::NotFound(_)) => {
                return Err(AppError::not_found(format!("gem '{name}' not found")));
            }
            Err(e) => return Err(AppError::from(e)),
        }
    }

    let pkg = PackageId::new(&registry, &name, "versions");
    proxy_stream(
        svc,
        pkg,
        identity,
        "releases:read",
        Some("application/json"),
    )
    .await
}

/// Serve a compressed gemspec file.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/quick/Marshal.4.8/{filename}",
    tag = "proxy/rubygems",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("filename" = String, Path, description = "Gemspec filename, e.g. rails-7.1.0.gemspec.rz"),
    ),
    responses(
        (status = 200, description = "Zlib-compressed gemspec"),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Gemspec not found"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/quick/Marshal.4.8/{filename}")]
pub async fn gem_gemspec(
    path: web::Path<(String, String)>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    map: web::Data<RegistryMap>,
) -> Result<impl Responder, AppError> {
    let (registry, filename) = path.into_inner();
    require_registry_type(&registry, "rubygems", &map)?;

    let stem = filename
        .strip_suffix(".gemspec.rz")
        .ok_or_else(|| AppError::bad_request(format!("invalid gemspec filename: {filename}")))?;

    let (name, version) =
        batlehub_adapters::registry::rubygems::split_gem_stem(stem).ok_or_else(|| {
            AppError::bad_request(format!("cannot parse gemspec filename: {filename}"))
        })?;

    let pkg = PackageId::new(&registry, name, version).with_artifact("gemspec");
    proxy_stream(
        svc,
        pkg,
        identity,
        "releases:read",
        Some("application/octet-stream"),
    )
    .await
}
