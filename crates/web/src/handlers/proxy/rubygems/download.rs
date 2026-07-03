use super::{
    get, proxy_stream, require_registry_type, serve_local_or_proxy_artifact, web, AppError, Arc,
    AuthIdentity, HttpResponse, LocalOrProxyArtifactOpts, LocalRegistryService, PackageId,
    ProxyService, RegistryMap, RegistryMode, RegistryModeMap, Responder,
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

    serve_local_or_proxy_artifact(
        svc,
        local_svc,
        &mode_map,
        &registry,
        name,
        version,
        identity,
        LocalOrProxyArtifactOpts {
            artifact_suffix: "gem",
            local_content_type: "application/octet-stream",
            proxy_content_type: Some("application/octet-stream"),
            resource_type: batlehub_core::rules::resource_type::RELEASES_READ,
            check_prerelease: true,
            append_signature: true,
        },
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
        batlehub_core::rules::resource_type::RELEASES_READ,
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
        batlehub_core::rules::resource_type::RELEASES_READ,
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
        batlehub_core::rules::resource_type::RELEASES_READ,
        Some("application/octet-stream"),
    )
    .await
}
