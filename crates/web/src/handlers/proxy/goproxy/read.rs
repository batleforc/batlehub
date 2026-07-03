use std::io::Read as _;
use std::sync::Arc;

use actix_web::{get, web, HttpResponse, Responder};

use batlehub_config::schema::RegistryMode;
use batlehub_core::{
    entities::PackageId,
    error::CoreError,
    services::{LocalRegistryService, ProxyService},
};

use crate::handlers::proxy::common::{proxy_stream, require_registry_type};
use crate::{error::AppError, extractors::AuthIdentity, RegistryMap, RegistryModeMap};

/// Dispatch a local/hybrid goproxy file request.
///
/// Returns the response on success, or `Err(CoreError::NotFound)` when the
/// module/extension is not found (callers handle hybrid fallthrough).
async fn local_goproxy_file(
    local_svc: &LocalRegistryService,
    registry: &str,
    module: &str,
    version: &str,
    ext: &str,
    identity: &batlehub_core::entities::Identity,
) -> Result<HttpResponse, batlehub_core::error::CoreError> {
    let resp = match ext {
        "info" => local_svc
            .get_go_info(registry, module, version, identity)
            .await
            .map(|info| {
                HttpResponse::Ok()
                    .content_type("application/json")
                    .json(info)
            })?,
        "mod" => local_svc
            .get_go_mod(registry, module, version, identity)
            .await
            .map(|content| HttpResponse::Ok().content_type("text/plain").body(content))?,
        "zip" => {
            local_svc
                .check_prerelease_access(registry, version, identity)
                .await?;
            local_svc
                .get_artifact(registry, module, version, identity)
                .await
                .map(|bytes| {
                    HttpResponse::Ok()
                        .content_type("application/zip")
                        .body(bytes)
                })?
        }
        _ => {
            return Err(batlehub_core::error::CoreError::NotFound(format!(
                "unknown goproxy file extension '.{ext}'"
            )))
        }
    };
    Ok(resp)
}

/// Scan a Go module zip for a go.mod entry matching `mod_suffix` (exact) or
/// any path ending with `/go.mod` (fallback). Returns `None` if not found.
fn scan_zip_for_go_mod(zip_bytes: &[u8], mod_suffix: &str) -> Option<String> {
    let cursor = std::io::Cursor::new(zip_bytes);
    let mut archive = zip::ZipArchive::new(cursor).ok()?;
    for i in 0..archive.len() {
        if let Ok(mut file) = archive.by_index(i) {
            let name = file.name().to_owned();
            if name == mod_suffix || name.ends_with("/go.mod") {
                let mut contents = String::new();
                if file.read_to_string(&mut contents).is_ok() {
                    return Some(contents);
                }
            }
        }
    }
    None
}

/// Extract the go.mod content from a Go module zip archive.
/// Go module zips contain entries named `{module}@{version}/{path}`.
/// Returns a minimal go.mod if none is found.
pub(super) fn extract_go_mod(zip_bytes: &[u8], module: &str, version: &str) -> String {
    let mod_suffix = format!("{module}@{version}/go.mod");
    scan_zip_for_go_mod(zip_bytes, &mod_suffix)
        .unwrap_or_else(|| format!("module {module}\n\ngo 1.21\n"))
}

/// Fetch the latest version info for a Go module.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/{module}/@latest",
    tag = "proxy/goproxy",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("module"   = String, Path, description = "Go module path (may contain slashes)"),
    ),
    responses(
        (status = 200, description = "Latest version info JSON"),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Module not found"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/{module:[^@]+}@latest")]
pub async fn goproxy_latest(
    path: web::Path<(String, String)>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
) -> Result<impl Responder, AppError> {
    let (registry, raw_module) = path.into_inner();
    require_registry_type(&registry, "goproxy", &map)?;
    let module = raw_module.trim_end_matches('/');
    let mode = mode_map.get(&registry);

    if matches!(mode, RegistryMode::Local | RegistryMode::Hybrid) {
        match local_svc.get_go_latest(&registry, module, &identity).await {
            Ok(info) => {
                return Ok(HttpResponse::Ok()
                    .content_type("application/json")
                    .json(info))
            }
            Err(CoreError::NotFound(_)) if matches!(mode, RegistryMode::Hybrid) => {}
            Err(CoreError::NotFound(_)) => {
                return Err(AppError::not_found(format!("module '{module}' not found")));
            }
            Err(e) => return Err(AppError::from(e)),
        }
    }

    let pkg = PackageId::new(&registry, module, "latest");
    proxy_stream(
        svc,
        pkg,
        identity,
        batlehub_core::rules::resource_type::RELEASES_READ,
        Some("application/json"),
    )
    .await
}

/// List known versions for a Go module.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/{module}/@v/list",
    tag = "proxy/goproxy",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("module"   = String, Path, description = "Go module path (may contain slashes)"),
    ),
    responses(
        (status = 200, description = "Newline-separated version list"),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Module not found"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/{module:[^@]+}@v/list")]
pub async fn goproxy_list(
    path: web::Path<(String, String)>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
) -> Result<impl Responder, AppError> {
    let (registry, raw_module) = path.into_inner();
    require_registry_type(&registry, "goproxy", &map)?;
    let module = raw_module.trim_end_matches('/');
    let mode = mode_map.get(&registry);

    if matches!(mode, RegistryMode::Local | RegistryMode::Hybrid) {
        match local_svc
            .get_go_version_list(&registry, module, &identity)
            .await
        {
            Ok(list) => {
                return Ok(HttpResponse::Ok().content_type("text/plain").body(list));
            }
            Err(CoreError::NotFound(_)) if matches!(mode, RegistryMode::Hybrid) => {}
            Err(CoreError::NotFound(_)) => {
                return Ok(HttpResponse::Ok().content_type("text/plain").body(""));
            }
            Err(e) => return Err(AppError::from(e)),
        }
    }

    let pkg = PackageId::new(&registry, module, "latest").with_artifact("list");
    proxy_stream(
        svc,
        pkg,
        identity,
        batlehub_core::rules::resource_type::RELEASES_READ,
        Some("text/plain"),
    )
    .await
}

/// Fetch a versioned Go module file: `.info`, `.mod`, or `.zip`.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/{module}/@v/{filename}",
    tag = "proxy/goproxy",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("module"   = String, Path, description = "Go module path (may contain slashes)"),
        ("filename" = String, Path, description = "Versioned file: {version}.info, {version}.mod, or {version}.zip"),
    ),
    responses(
        (status = 200, description = "Requested module file"),
        (status = 400, description = "Unknown file type"),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Module or version not found"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/{module:[^@]+}@v/{filename}")]
pub async fn goproxy_file(
    path: web::Path<(String, String, String)>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
) -> Result<impl Responder, AppError> {
    let (registry, raw_module, filename) = path.into_inner();
    require_registry_type(&registry, "goproxy", &map)?;
    let module = raw_module.trim_end_matches('/');
    let mode = mode_map.get(&registry);

    let (version, ext) = filename
        .rsplit_once('.')
        .ok_or_else(|| AppError::not_found(format!("unknown goproxy file '{filename}'")))?;

    if matches!(mode, RegistryMode::Local | RegistryMode::Hybrid) {
        match local_goproxy_file(&local_svc, &registry, module, version, ext, &identity).await {
            Ok(resp) => return Ok(resp),
            Err(CoreError::NotFound(_)) if matches!(mode, RegistryMode::Hybrid) => {}
            Err(CoreError::NotFound(msg)) => return Err(AppError::not_found(msg)),
            Err(e) => return Err(AppError::from(e)),
        }
    }

    let (pkg, content_type, resource_type) = match ext {
        "info" => (
            PackageId::new(&registry, module, version),
            "application/json",
            batlehub_core::rules::resource_type::RELEASES_READ,
        ),
        "mod" => (
            PackageId::new(&registry, module, version).with_artifact("mod"),
            "text/plain",
            batlehub_core::rules::resource_type::RELEASES_READ,
        ),
        "zip" => (
            PackageId::new(&registry, module, version).with_artifact("zip"),
            "application/zip",
            batlehub_core::rules::resource_type::SOURCE_READ,
        ),
        _ => {
            return Err(AppError::not_found(format!(
                "unknown goproxy file extension '.{ext}'"
            )));
        }
    };

    proxy_stream(svc, pkg, identity, resource_type, Some(content_type)).await
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    fn make_zip_with_go_mod(entry_name: &str, content: &str) -> Vec<u8> {
        let mut buf = Vec::new();
        let mut zip = ZipWriter::new(std::io::Cursor::new(&mut buf));
        zip.start_file(entry_name, SimpleFileOptions::default())
            .unwrap();
        zip.write_all(content.as_bytes()).unwrap();
        zip.finish().unwrap();
        buf
    }

    #[test]
    fn extract_go_mod_exact_name_match() {
        let content = "module github.com/foo/bar\n\ngo 1.21\n";
        let zip = make_zip_with_go_mod("github.com/foo/bar@v1.0.0/go.mod", content);
        let result = extract_go_mod(&zip, "github.com/foo/bar", "v1.0.0");
        assert_eq!(result, content);
    }

    #[test]
    fn extract_go_mod_fallback_suffix_match() {
        let content = "module example.com/mod\n\ngo 1.22\n";
        let zip = make_zip_with_go_mod("example.com/mod@v2.0.0/go.mod", content);
        // Pass a different module/version — falls back to suffix match
        let result = extract_go_mod(&zip, "other/path", "v0.0.0");
        assert_eq!(result, content);
    }

    #[test]
    fn extract_go_mod_not_found_returns_minimal_fallback() {
        let zip = make_zip_with_go_mod("README.md", "hello");
        let result = extract_go_mod(&zip, "github.com/foo/bar", "v1.0.0");
        assert!(result.contains("module github.com/foo/bar"));
        assert!(result.contains("go 1.21"));
    }

    #[test]
    fn extract_go_mod_invalid_zip_returns_fallback() {
        let result = extract_go_mod(b"not a zip", "example.com/mod", "v1.0.0");
        assert!(result.contains("module example.com/mod"));
    }
}
