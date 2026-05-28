use std::sync::Arc;

use actix_web::{HttpRequest, HttpResponse, Responder, delete, get, post, web};
use sha2::{Digest, Sha256};

use batlehub_config::schema::RegistryMode;
use batlehub_core::{
    entities::PackageId,
    error::CoreError,
    services::{LocalRegistryService, ProxyService, PublishRequest},
};

use crate::{RegistryMap, RegistryModeMap, error::AppError, extractors::AuthIdentity};
use super::common::{collect_payload, proxy_stream, require_local_mode};

fn require_composer(registry: &str, map: &RegistryMap) -> Result<(), AppError> {
    match map.type_of(registry) {
        Some("composer") => Ok(()),
        Some(_) => Err(AppError::not_found(format!(
            "registry '{registry}' is not a Composer registry"
        ))),
        None => Err(AppError::not_found(format!("unknown registry '{registry}'"))),
    }
}

/// Extract base URL from the incoming request, owned so the `ConnectionInfo`
/// borrow can be released before any `.await` points.
fn build_base_url(req: &HttpRequest) -> String {
    let conn = req.connection_info();
    format!("{}://{}", conn.scheme(), conn.host())
}

// ── packages.json ─────────────────────────────────────────────────────────────

/// Composer registry root index.
///
/// Returns a `packages.json` that points Composer clients to our `p2/` endpoints.
/// In local/hybrid mode the response also includes `available-packages` listing
/// all locally published package names.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/packages.json",
    tag = "proxy/composer",
    params(("registry" = String, Path, description = "Registry name")),
    responses(
        (status = 200, description = "Composer packages.json index"),
        (status = 404, description = "Unknown registry"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/packages.json")]
pub async fn composer_packages_json(
    req: HttpRequest,
    path: web::Path<String>,
    _identity: AuthIdentity,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
) -> Result<impl Responder, AppError> {
    let registry = path.into_inner();
    require_composer(&registry, &map)?;

    let base_url = build_base_url(&req);
    let metadata_url = format!("{base_url}/proxy/{registry}/p2/%package%.json");

    let mode = mode_map.get(&registry);
    let available_packages: Vec<String> =
        if matches!(mode, RegistryMode::Local | RegistryMode::Hybrid) {
            local_svc
                .get_composer_packages_list(&registry)
                .await
                .unwrap_or_default()
        } else {
            vec![]
        };

    let body = serde_json::json!({
        "packages": [],
        "metadata-url": metadata_url,
        "available-packages": available_packages,
    });

    Ok(HttpResponse::Ok()
        .content_type("application/json")
        .json(body))
}

// ── p2 metadata ───────────────────────────────────────────────────────────────

/// Packagist v2 package metadata (all versions).
///
/// Handles both `{vendor}/{package}.json` and `{vendor}/{package}~dev.json`.
/// In proxy mode: fetched and cached from the upstream Packagist.
/// In local mode: built from locally published packages only.
/// In hybrid mode: local packages first, falling back to upstream.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/p2/{path}",
    tag = "proxy/composer",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("path"     = String, Path, description = "Vendor/package path, e.g. symfony/console.json"),
    ),
    responses(
        (status = 200, description = "Packagist v2 metadata JSON"),
        (status = 404, description = "Package not found"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/p2/{path:.*}")]
pub async fn composer_p2_metadata(
    req: HttpRequest,
    path: web::Path<(String, String)>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
) -> Result<impl Responder, AppError> {
    let (registry, p2_path) = path.into_inner();
    require_composer(&registry, &map)?;

    // Parse the path: "vendor/package.json" or "vendor/package~dev.json".
    // The `~dev` suffix is significant — Packagist serves different JSON for dev variants,
    // so the cache key must distinguish them.
    let package_name = parse_p2_package_name(&p2_path).ok_or_else(|| {
        AppError::bad_request(format!("invalid Composer p2 path: '{p2_path}'"))
    })?;
    let is_dev = p2_path.contains('~');
    let p2_artifact = if is_dev { "p2~dev" } else { "p2" };

    let base_url = build_base_url(&req);
    let mode = mode_map.get(&registry);

    if matches!(mode, RegistryMode::Local | RegistryMode::Hybrid) {
        match local_svc
            .get_composer_p2_response(&registry, &package_name, &base_url, &identity.0)
            .await
        {
            Ok(json) => {
                return Ok(HttpResponse::Ok()
                    .content_type("application/json")
                    .json(json));
            }
            Err(CoreError::NotFound(_)) if matches!(mode, RegistryMode::Hybrid) => {
                // fall through to proxy
            }
            Err(CoreError::NotFound(_)) => {
                return Err(AppError::not_found(format!(
                    "composer package '{package_name}' not found in local registry '{registry}'"
                )));
            }
            Err(e) => return Err(AppError::from(e)),
        }
    }

    // Proxy mode (or hybrid fallback): fetch from upstream via ProxyService.
    // Use version="_index" so the artifact key is stable; p2_artifact encodes the ~dev variant.
    let pkg = PackageId::new(&registry, &package_name, "_index").with_artifact(p2_artifact);
    proxy_stream(svc, pkg, identity, "releases:read", Some("application/json")).await
}

// ── dist artifact download ────────────────────────────────────────────────────

/// Download a Composer package ZIP artifact.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/dist/{vendor}/{package}/{version}",
    tag = "proxy/composer",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("vendor"   = String, Path, description = "Vendor name"),
        ("package"  = String, Path, description = "Package name"),
        ("version"  = String, Path, description = "Version string"),
    ),
    responses(
        (status = 200, description = "Package ZIP artifact"),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Package or version not found"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/dist/{vendor}/{package}/{version}")]
pub async fn composer_dist(
    path: web::Path<(String, String, String, String)>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
) -> Result<impl Responder, AppError> {
    let (registry, vendor, package, version) = path.into_inner();
    require_composer(&registry, &map)?;

    let name = format!("{vendor}/{package}");
    let mode = mode_map.get(&registry);

    if matches!(mode, RegistryMode::Local) {
        local_svc
            .check_prerelease_access(&registry, &version, &identity.0)
            .await
            .map_err(AppError::from)?;
        match local_svc.get_artifact(&registry, &name, &version).await {
            Ok(bytes) => {
                return Ok(HttpResponse::Ok()
                    .content_type("application/zip")
                    .body(bytes));
            }
            Err(CoreError::NotFound(_)) => {
                return Err(AppError::not_found(format!(
                    "composer package '{name}@{version}' not found in local registry '{registry}'"
                )));
            }
            Err(e) => return Err(AppError::from(e)),
        }
    }

    if matches!(mode, RegistryMode::Hybrid) {
        // Check the pre-release gate first.  Any error here (including gated access)
        // is a hard denial — we must NOT fall through to the upstream proxy, because
        // the same version may exist there and would bypass the gate.
        local_svc
            .check_prerelease_access(&registry, &version, &identity.0)
            .await
            .map_err(AppError::from)?;

        // Gate passed — try local artifact; fall through to proxy only when we truly
        // don't have it locally (NotFound = "not published here, go upstream").
        match local_svc.get_artifact(&registry, &name, &version).await {
            Ok(bytes) => {
                return Ok(HttpResponse::Ok()
                    .content_type("application/zip")
                    .body(bytes));
            }
            Err(CoreError::NotFound(_)) => {} // fall through to proxy
            Err(e) => return Err(AppError::from(e)),
        }
    }

    let pkg = PackageId::new(&registry, &name, &version).with_artifact("dist");
    proxy_stream(svc, pkg, identity, "releases:read", Some("application/zip")).await
}

// ── Publish (local/hybrid only) ───────────────────────────────────────────────

/// Upload a Composer package ZIP (local/hybrid registries only).
///
/// The request body must be a ZIP file containing a `composer.json` at the root
/// or in a single top-level directory (GitHub-style zipball layout).
/// An optional `?version=x.y.z` query parameter overrides the version in `composer.json`.
#[utoipa::path(
    post,
    path = "/proxy/{registry}/api/upload",
    tag = "proxy/composer",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("version"  = String, Query, description = "Version override (optional)"),
    ),
    responses(
        (status = 200, description = "Package published successfully"),
        (status = 403, description = "Access denied or quota exceeded"),
        (status = 409, description = "Version already exists"),
        (status = 422, description = "Invalid ZIP or versioning policy violation"),
    ),
    security(("bearer_token" = [])),
)]
#[post("/proxy/{registry}/api/upload")]
pub async fn composer_upload(
    path: web::Path<String>,
    query: web::Query<UploadQuery>,
    payload: web::Payload,
    identity: AuthIdentity,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
) -> Result<impl Responder, AppError> {
    let registry = path.into_inner();
    require_composer(&registry, &map)?;
    require_local_mode(&registry, &mode_map)?;

    // Validate the version override early before buffering the payload.
    if let Some(ref ver) = query.version {
        validate_version_param(ver)?;
    }

    let data = collect_payload(payload).await?;

    let meta = batlehub_adapters::registry::composer::parse_composer_zip(
        &data,
        query.version.as_deref(),
    )
    .map_err(|e| AppError::unprocessable(e.to_string()))?;

    let checksum = hex::encode(Sha256::digest(&data));

    let index_metadata = meta.composer_json.clone();

    let name = meta.name.clone();
    let version = meta.version.clone();

    let quota_check = local_svc
        .publish(PublishRequest {
            registry: registry.clone(),
            name: name.clone(),
            version: version.clone(),
            artifact: data,
            checksum,
            index_metadata,
            publisher: identity.0,
            signature_bytes: None,
            signature_type: None,
        })
        .await
        .map_err(AppError::from)?;

    let mut resp = HttpResponse::Ok();
    for (header, value) in quota_check.headers() {
        resp.insert_header((header, value));
    }
    Ok(resp.json(serde_json::json!({
        "status": "success",
        "name": name,
        "version": version,
    })))
}

#[derive(serde::Deserialize)]
struct UploadQuery {
    version: Option<String>,
}

// ── Yank (local/hybrid only) ──────────────────────────────────────────────────

/// Yank a Composer package version (local/hybrid registries only).
#[utoipa::path(
    delete,
    path = "/proxy/{registry}/api/packages/{vendor}/{package}/versions/{version}",
    tag = "proxy/composer",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("vendor"   = String, Path, description = "Vendor name"),
        ("package"  = String, Path, description = "Package name"),
        ("version"  = String, Path, description = "Version to yank"),
    ),
    responses(
        (status = 200, description = "Version yanked"),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Package or version not found"),
    ),
    security(("bearer_token" = [])),
)]
#[delete("/proxy/{registry}/api/packages/{vendor}/{package}/versions/{version}")]
pub async fn composer_yank(
    path: web::Path<(String, String, String, String)>,
    identity: AuthIdentity,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
) -> Result<impl Responder, AppError> {
    let (registry, vendor, package, version) = path.into_inner();
    require_composer(&registry, &map)?;
    require_local_mode(&registry, &mode_map)?;

    let name = format!("{vendor}/{package}");
    local_svc
        .yank(&registry, &name, &version, &identity.0)
        .await
        .map_err(AppError::from)?;

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "message": format!("Successfully yanked {name} ({version})")
    })))
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Validate that a `?version=` query-parameter value is safe to use as a
/// package version string. Rejects values that are excessively long or
/// contain characters outside the set allowed by Composer.
fn validate_version_param(v: &str) -> Result<(), AppError> {
    if v.len() > 128 {
        return Err(AppError::unprocessable("version parameter too long".to_owned()));
    }
    let ok = v.chars().all(|c| {
        c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+' | '~' | '_' | 'v' | 'V')
    });
    if !ok {
        return Err(AppError::unprocessable(format!(
            "invalid characters in version parameter: '{v}'"
        )));
    }
    Ok(())
}

/// Extract the `vendor/package` name from a p2 path like:
/// - `"vendor/package.json"` → `"vendor/package"`
/// - `"vendor/package~dev.json"` → `"vendor/package"`
///
/// Rejects paths whose vendor or package segments contain characters outside
/// `[a-z0-9A-Z_.-]` to prevent path-traversal and injection.
fn parse_p2_package_name(path: &str) -> Option<String> {
    // Must contain exactly one slash (vendor/package[~dev].json)
    let (vendor, rest) = path.split_once('/')?;
    if vendor.is_empty() || !is_valid_composer_segment(vendor) {
        return None;
    }
    // Strip .json, then optionally the `~dev` suffix. Only `~dev` is a valid
    // Packagist v2 variant; any other tilde sequence is rejected to prevent
    // silent truncation (e.g. `pkg~evil.json` being served as `pkg`).
    let without_json = rest.strip_suffix(".json")?;
    let package = if let Some((base, suffix)) = without_json.split_once('~') {
        if suffix != "dev" {
            return None;
        }
        base
    } else {
        without_json
    };
    if package.is_empty() || !is_valid_composer_segment(package) {
        return None;
    }
    Some(format!("{vendor}/{package}"))
}

/// Returns `true` when every character in `s` is a safe Composer name segment
/// character: ASCII alphanumeric, hyphen, underscore, or dot.
fn is_valid_composer_segment(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_p2_normal_package() {
        assert_eq!(
            parse_p2_package_name("symfony/console.json"),
            Some("symfony/console".to_owned())
        );
    }

    #[test]
    fn parse_p2_dev_variant() {
        assert_eq!(
            parse_p2_package_name("symfony/console~dev.json"),
            Some("symfony/console".to_owned())
        );
    }

    #[test]
    fn parse_p2_unknown_tilde_suffix_rejected() {
        // Only `~dev` is valid; any other suffix must be rejected, not truncated.
        assert_eq!(parse_p2_package_name("vendor/pkg~2.json"), None);
        assert_eq!(parse_p2_package_name("vendor/pkg~evil.json"), None);
        assert_eq!(parse_p2_package_name("vendor/pkg~beta.json"), None);
    }

    #[test]
    fn parse_p2_no_slash_returns_none() {
        assert_eq!(parse_p2_package_name("console.json"), None);
    }

    #[test]
    fn parse_p2_empty_returns_none() {
        assert_eq!(parse_p2_package_name(""), None);
    }

    #[test]
    fn parse_p2_wrong_extension_returns_none() {
        assert_eq!(parse_p2_package_name("vendor/pkg.txt"), None);
    }

    #[test]
    fn parse_p2_empty_vendor_returns_none() {
        assert_eq!(parse_p2_package_name("/pkg.json"), None);
    }

    #[test]
    fn parse_p2_empty_package_returns_none() {
        assert_eq!(parse_p2_package_name("vendor/.json"), None);
    }

    #[test]
    fn parse_p2_path_traversal_rejected() {
        assert_eq!(parse_p2_package_name("../etc/pkg.json"), None);
        assert_eq!(parse_p2_package_name("vendor/../etc.json"), None);
    }

    #[test]
    fn validate_version_param_accepts_valid() {
        assert!(validate_version_param("1.0.0").is_ok());
        assert!(validate_version_param("v2.3.4-beta.1").is_ok());
        assert!(validate_version_param("dev-main").is_ok());
    }

    #[test]
    fn validate_version_param_rejects_long() {
        let long = "a".repeat(129);
        assert!(validate_version_param(&long).is_err());
    }

    #[test]
    fn validate_version_param_rejects_special_chars() {
        assert!(validate_version_param("../../etc/passwd").is_err());
        assert!(validate_version_param("1.0.0; rm -rf /").is_err());
    }
}
