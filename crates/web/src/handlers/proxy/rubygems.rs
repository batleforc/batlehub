use std::sync::Arc;

use actix_web::{HttpRequest, HttpResponse, Responder, delete, get, post, put, web};
use sha2::{Digest, Sha256};

use batlehub_config::schema::RegistryMode;
use batlehub_core::{
    entities::PackageId,
    services::{LocalRegistryService, ProxyService, PublishRequest},
};

use crate::{RegistryMap, RegistryModeMap, error::AppError, extractors::AuthIdentity};
use super::common::{append_signature_headers, collect_payload, extract_signature_headers, proxy_stream, require_local_mode};

fn require_rubygems(registry: &str, map: &RegistryMap) -> Result<(), AppError> {
    match map.type_of(registry) {
        Some("rubygems") => Ok(()),
        Some(_) => Err(AppError::not_found(format!(
            "registry '{registry}' is not a RubyGems registry"
        ))),
        None => Err(AppError::not_found(format!("unknown registry '{registry}'"))),
    }
}

// ── Proxy & shared download routes ───────────────────────────────────────────

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
    require_rubygems(&registry, &map)?;

    let stem = filename
        .strip_suffix(".gem")
        .ok_or_else(|| AppError::bad_request(format!("invalid gem filename: {filename}")))?;

    let (name, version) =
        batlehub_adapters::registry::rubygems::split_gem_stem(stem).ok_or_else(|| {
            AppError::bad_request(format!("cannot parse gem filename: {filename}"))
        })?;

    let mode = mode_map.get(&registry);
    if matches!(mode, RegistryMode::Local | RegistryMode::Hybrid) {
        match local_svc.get_artifact(&registry, name, version).await {
            Ok(bytes) => {
                let mut resp = HttpResponse::Ok();
                resp.content_type("application/octet-stream");
                append_signature_headers(&mut resp, &local_svc, &registry, name, version).await;
                return Ok(resp.body(bytes));
            }
            Err(batlehub_core::error::CoreError::NotFound(_))
                if matches!(mode, RegistryMode::Hybrid) => {}
            Err(batlehub_core::error::CoreError::NotFound(_)) => {
                return Err(AppError::not_found(format!("gem '{name}@{version}' not found")));
            }
            Err(e) => return Err(AppError::from(e)),
        }
    }

    let pkg = PackageId::new(&registry, name, version).with_artifact("gem");
    proxy_stream(svc, pkg, identity, "releases:read", Some("application/octet-stream")).await
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
    require_rubygems(&registry, &map)?;

    let mode = mode_map.get(&registry);
    if matches!(mode, RegistryMode::Local | RegistryMode::Hybrid) {
        match local_svc.get_rubygems_gem_info(&registry, &name).await {
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
    proxy_stream(svc, pkg, identity, "releases:read", Some("application/json")).await
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
    require_rubygems(&registry, &map)?;

    let mode = mode_map.get(&registry);
    if matches!(mode, RegistryMode::Local | RegistryMode::Hybrid) {
        match local_svc.get_rubygems_versions(&registry, &name).await {
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
    proxy_stream(svc, pkg, identity, "releases:read", Some("application/json")).await
}

/// Serve the full gem index (specs.4.8.gz).
#[utoipa::path(
    get,
    path = "/proxy/{registry}/specs.4.8.gz",
    tag = "proxy/rubygems",
    params(("registry" = String, Path, description = "Registry name")),
    responses(
        (status = 200, description = "Gzip-compressed binary specs index"),
        (status = 404, description = "Not available for local-only registries"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/specs.4.8.gz")]
pub async fn gem_specs_full(
    path: web::Path<String>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
) -> Result<impl Responder, AppError> {
    let registry = path.into_inner();
    require_rubygems(&registry, &map)?;

    if mode_map.get(&registry) == RegistryMode::Local {
        return Err(AppError::not_found(
            "binary specs index is not available for local-only registries; use /api/v1/versions/{name}.json".to_owned(),
        ));
    }

    let pkg = PackageId::new(&registry, "_index", "specs");
    proxy_stream(svc, pkg, identity, "releases:read", Some("application/octet-stream")).await
}

/// Serve the latest-versions gem index (latest_specs.4.8.gz).
#[utoipa::path(
    get,
    path = "/proxy/{registry}/latest_specs.4.8.gz",
    tag = "proxy/rubygems",
    params(("registry" = String, Path, description = "Registry name")),
    responses(
        (status = 200, description = "Gzip-compressed binary latest-specs index"),
        (status = 404, description = "Not available for local-only registries"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/latest_specs.4.8.gz")]
pub async fn gem_specs_latest(
    path: web::Path<String>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
) -> Result<impl Responder, AppError> {
    let registry = path.into_inner();
    require_rubygems(&registry, &map)?;

    if mode_map.get(&registry) == RegistryMode::Local {
        return Err(AppError::not_found(
            "binary specs index is not available for local-only registries; use /api/v1/versions/{name}.json".to_owned(),
        ));
    }

    let pkg = PackageId::new(&registry, "_index", "latest_specs");
    proxy_stream(svc, pkg, identity, "releases:read", Some("application/octet-stream")).await
}

/// Serve the prerelease gem index (prerelease_specs.4.8.gz).
#[utoipa::path(
    get,
    path = "/proxy/{registry}/prerelease_specs.4.8.gz",
    tag = "proxy/rubygems",
    params(("registry" = String, Path, description = "Registry name")),
    responses(
        (status = 200, description = "Gzip-compressed binary prerelease-specs index"),
        (status = 404, description = "Not available for local-only registries"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/prerelease_specs.4.8.gz")]
pub async fn gem_specs_prerelease(
    path: web::Path<String>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
) -> Result<impl Responder, AppError> {
    let registry = path.into_inner();
    require_rubygems(&registry, &map)?;

    if mode_map.get(&registry) == RegistryMode::Local {
        return Err(AppError::not_found(
            "binary specs index is not available for local-only registries; use /api/v1/versions/{name}.json".to_owned(),
        ));
    }

    let pkg = PackageId::new(&registry, "_index", "prerelease_specs");
    proxy_stream(svc, pkg, identity, "releases:read", Some("application/octet-stream")).await
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
    require_rubygems(&registry, &map)?;

    let stem = filename
        .strip_suffix(".gemspec.rz")
        .ok_or_else(|| AppError::bad_request(format!("invalid gemspec filename: {filename}")))?;

    let (name, version) =
        batlehub_adapters::registry::rubygems::split_gem_stem(stem).ok_or_else(|| {
            AppError::bad_request(format!("cannot parse gemspec filename: {filename}"))
        })?;

    let pkg = PackageId::new(&registry, name, version).with_artifact("gemspec");
    proxy_stream(svc, pkg, identity, "releases:read", Some("application/octet-stream")).await
}

// ── Local / hybrid write routes ───────────────────────────────────────────────

/// Publish a gem (local/hybrid registries only).
///
/// Accepts the raw `.gem` file bytes in the request body.
/// Compatible with `gem push`.
#[utoipa::path(
    post,
    path = "/proxy/{registry}/api/v1/gems",
    tag = "proxy/rubygems",
    params(("registry" = String, Path, description = "Registry name")),
    responses(
        (status = 200, description = "Gem published successfully"),
        (status = 403, description = "Access denied or quota exceeded"),
        (status = 409, description = "Version already exists"),
        (status = 422, description = "Invalid gem file or versioning policy violation"),
    ),
    security(("bearer_token" = [])),
)]
#[post("/proxy/{registry}/api/v1/gems")]
pub async fn gem_publish(
    req: HttpRequest,
    path: web::Path<String>,
    payload: web::Payload,
    identity: AuthIdentity,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
) -> Result<impl Responder, AppError> {
    let registry = path.into_inner();
    require_rubygems(&registry, &map)?;
    require_local_mode(&registry, &mode_map)?;

    let data = collect_payload(payload).await?;

    let gem_meta = batlehub_adapters::registry::rubygems::parse_gem_bytes(&data)
        .map_err(|e| AppError::unprocessable(e.to_string()))?;

    let checksum = hex::encode(Sha256::digest(&data));

    let index_metadata = serde_json::json!({
        "name": gem_meta.name,
        "version": gem_meta.version,
        "platform": gem_meta.platform,
        "summary": gem_meta.summary,
        "authors": gem_meta.authors,
        "sha": checksum,
    });

    let name = gem_meta.name.clone();
    let version = gem_meta.version.clone();

    let (signature_bytes, signature_type) = extract_signature_headers(&req);

    let quota_check = local_svc
        .publish(PublishRequest {
            registry: registry.clone(),
            name: name.clone(),
            version: version.clone(),
            artifact: data,
            checksum,
            index_metadata,
            publisher: identity.0,
            signature_bytes,
            signature_type,
        })
        .await
        .map_err(AppError::from)?;

    let mut resp = HttpResponse::Ok();
    for (header, value) in quota_check.headers() {
        resp.insert_header((header, value));
    }
    Ok(resp.json(serde_json::json!({
        "message": format!("Successfully registered gem: {} ({})", name, version)
    })))
}

#[derive(serde::Deserialize)]
struct GemYankQuery {
    gem_name: String,
    version: String,
}

/// Yank a gem version (local/hybrid registries only).
#[utoipa::path(
    delete,
    path = "/proxy/{registry}/api/v1/gems/yank",
    tag = "proxy/rubygems",
    params(
        ("registry"  = String, Path, description = "Registry name"),
        ("gem_name"  = String, Query, description = "Gem name"),
        ("version"   = String, Query, description = "Gem version to yank"),
    ),
    responses(
        (status = 200, description = "Gem yanked"),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Gem or version not found"),
    ),
    security(("bearer_token" = [])),
)]
#[delete("/proxy/{registry}/api/v1/gems/yank")]
pub async fn gem_yank(
    path: web::Path<String>,
    query: web::Query<GemYankQuery>,
    identity: AuthIdentity,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
) -> Result<impl Responder, AppError> {
    let registry = path.into_inner();
    require_rubygems(&registry, &map)?;
    require_local_mode(&registry, &mode_map)?;

    local_svc
        .yank(&registry, &query.gem_name, &query.version, &identity.0)
        .await
        .map_err(AppError::from)?;

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "message": format!("Successfully yanked gem: {} ({})", query.gem_name, query.version)
    })))
}

/// Unyank a gem version (local/hybrid registries only).
#[utoipa::path(
    put,
    path = "/proxy/{registry}/api/v1/gems/unyank",
    tag = "proxy/rubygems",
    params(
        ("registry"  = String, Path, description = "Registry name"),
        ("gem_name"  = String, Query, description = "Gem name"),
        ("version"   = String, Query, description = "Gem version to unyank"),
    ),
    responses(
        (status = 200, description = "Gem unyanked"),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Gem or version not found"),
    ),
    security(("bearer_token" = [])),
)]
#[put("/proxy/{registry}/api/v1/gems/unyank")]
pub async fn gem_unyank(
    path: web::Path<String>,
    query: web::Query<GemYankQuery>,
    identity: AuthIdentity,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
) -> Result<impl Responder, AppError> {
    let registry = path.into_inner();
    require_rubygems(&registry, &map)?;
    require_local_mode(&registry, &mode_map)?;

    local_svc
        .unyank(&registry, &query.gem_name, &query.version, &identity.0)
        .await
        .map_err(AppError::from)?;

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "message": format!("Successfully unyanked gem: {} ({})", query.gem_name, query.version)
    })))
}
