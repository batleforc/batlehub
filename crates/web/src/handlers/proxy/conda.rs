use std::sync::Arc;

use actix_web::{get, post, web, HttpRequest, HttpResponse, Responder};
use sha2::{Digest, Sha256};

use batlehub_config::schema::RegistryMode;
use batlehub_core::{
    entities::PackageId,
    error::CoreError,
    services::{LocalRegistryService, ProxyService, PublishRequest},
};

use super::common::{
    collect_payload, extract_signature_headers, proxy_stream, require_local_mode,
    require_registry_type,
};
use crate::{
    error::AppError, extractors::AuthIdentity, services::NotificationService, RegistryMap,
    RegistryModeMap,
};
use batlehub_core::entities::NotificationEventType;

// ── Proxy routes ──────────────────────────────────────────────────────────────

/// Serve (and optionally merge) a conda channel's `repodata.json` for a
/// specific platform (e.g. `linux-64`, `noarch`).
///
/// - **Proxy mode**: stream `repodata.json` from upstream through the cache.
/// - **Local mode**: return only locally-published packages.
/// - **Hybrid mode**: merge upstream repodata with locally-published packages.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/{platform}/repodata.json",
    tag = "proxy/conda",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("platform" = String, Path, description = "Platform string, e.g. linux-64 or noarch"),
    ),
    responses(
        (status = 200, description = "repodata.json"),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Channel not found"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/{platform}/repodata.json")]
pub async fn conda_repodata(
    path: web::Path<(String, String)>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
) -> Result<impl Responder, AppError> {
    let (registry, platform) = path.into_inner();
    require_registry_type(&registry, "conda", &map)?;

    let mode = mode_map.get(&registry);

    if mode == RegistryMode::Local {
        let repodata = local_svc
            .get_conda_repodata(&registry, &platform)
            .await
            .map_err(AppError::from)?;
        return Ok(HttpResponse::Ok()
            .content_type("application/json")
            .body(serde_json::to_string(&repodata).unwrap_or_default()));
    }

    if mode == RegistryMode::Hybrid {
        // Fetch upstream repodata via ProxyService (cached), then merge local packages.
        let pkg = PackageId::new(&registry, "repodata", &platform).with_artifact("repodata.json");

        match svc
            .handle(batlehub_core::services::ProxyRequest {
                package_id: pkg,
                identity: identity.0.clone(),
                resource_type: "releases:read".to_owned(),
                ip_address: None,
                user_agent: None,
            })
            .await
            .map_err(AppError::from)?
        {
            batlehub_core::services::ProxyResponse::Denied { reason } => {
                return Err(AppError::forbidden(reason));
            }
            batlehub_core::services::ProxyResponse::Stream(mut stream) => {
                use futures::StreamExt;
                let mut buf = bytes::BytesMut::new();
                while let Some(chunk) = stream.next().await {
                    match chunk {
                        Ok(c) => buf.extend_from_slice(&c),
                        Err(e) => {
                            tracing::warn!(error = %e, "upstream repodata stream error");
                            break;
                        }
                    }
                }
                let upstream_bytes = buf.freeze();

                // Merge with local packages
                let local_repodata = local_svc
                    .get_conda_repodata(&registry, &platform)
                    .await
                    .map_err(AppError::from)?;

                let merged = merge_repodata(&upstream_bytes, &local_repodata);
                return Ok(HttpResponse::Ok()
                    .content_type("application/json")
                    .body(merged));
            }
        }
    }

    // Proxy mode: stream through cache.
    let pkg = PackageId::new(&registry, "repodata", &platform).with_artifact("repodata.json");
    proxy_stream(
        svc,
        pkg,
        identity,
        "releases:read",
        Some("application/json"),
    )
    .await
}

/// Merge a locally-built repodata JSON overlay into upstream `repodata.json` bytes.
fn merge_repodata(upstream_bytes: &[u8], local: &serde_json::Value) -> Vec<u8> {
    let mut upstream: serde_json::Value = match serde_json::from_slice(upstream_bytes) {
        Ok(v) => v,
        Err(_) => return serde_json::to_vec(local).unwrap_or_default(),
    };

    for key in ["packages", "packages.conda"] {
        if let Some(local_pkgs) = local.get(key).and_then(|v| v.as_object()) {
            let upstream_pkgs = upstream.get_mut(key).and_then(|v| v.as_object_mut());
            if let Some(up) = upstream_pkgs {
                for (filename, entry) in local_pkgs {
                    up.insert(filename.clone(), entry.clone());
                }
            } else {
                upstream[key] = local[key].clone();
            }
        }
    }

    serde_json::to_vec(&upstream).unwrap_or_default()
}

/// Serve the `current_repodata.json` (subset of `repodata.json` with latest
/// versions only).  Routed identically to `repodata.json` through the cache.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/{platform}/current_repodata.json",
    tag = "proxy/conda",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("platform" = String, Path, description = "Platform string"),
    ),
    responses(
        (status = 200, description = "current_repodata.json"),
        (status = 404, description = "Not found"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/{platform}/current_repodata.json")]
pub async fn conda_current_repodata(
    path: web::Path<(String, String)>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
) -> Result<impl Responder, AppError> {
    let (registry, platform) = path.into_inner();
    require_registry_type(&registry, "conda", &map)?;

    if mode_map.get(&registry) == RegistryMode::Local {
        return Err(AppError::not_found(
            "current_repodata.json is not available for local-only conda registries".to_owned(),
        ));
    }

    let pkg =
        PackageId::new(&registry, "repodata", &platform).with_artifact("current_repodata.json");
    proxy_stream(
        svc,
        pkg,
        identity,
        "releases:read",
        Some("application/json"),
    )
    .await
}

/// Download a conda package file (`.conda` or `.tar.bz2`) through the proxy cache.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/{platform}/{filename}",
    tag = "proxy/conda",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("platform" = String, Path, description = "Platform string"),
        ("filename" = String, Path, description = "Package filename"),
    ),
    responses(
        (status = 200, description = "Package bytes"),
        (status = 404, description = "Package not found"),
    ),
    security(("bearer_token" = [])),
)]
// Regex constrains filename to .tar.bz2 and .conda extensions, preventing
// this route from shadowing the npm/cargo GET /proxy/{registry}/{name}/{version} handler.
#[get("/proxy/{registry}/{platform}/{filename:.+\\.(?:tar\\.bz2|conda)}")]
pub async fn conda_file_download(
    path: web::Path<(String, String, String)>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
) -> Result<impl Responder, AppError> {
    let (registry, platform, filename) = path.into_inner();
    require_registry_type(&registry, "conda", &map)?;

    let mode = mode_map.get(&registry);

    if mode == RegistryMode::Local {
        // Look up by filename in index_metadata since package names may contain hyphens.
        let (name, version) = local_svc
            .find_conda_by_filename(&registry, &filename)
            .await
            .map_err(AppError::from)?
            .ok_or_else(|| AppError::not_found(format!("conda package not found: {filename}")))?;
        let bytes = local_svc
            .get_artifact(&registry, &name, &version, &identity)
            .await
            .map_err(AppError::from)?;
        return Ok(HttpResponse::Ok()
            .content_type("application/octet-stream")
            .body(bytes));
    }

    if mode == RegistryMode::Hybrid {
        if let Some((name, version)) = local_svc
            .find_conda_by_filename(&registry, &filename)
            .await
            .map_err(AppError::from)?
        {
            match local_svc
                .get_artifact(&registry, &name, &version, &identity)
                .await
            {
                Ok(bytes) => {
                    return Ok(HttpResponse::Ok()
                        .content_type("application/octet-stream")
                        .body(bytes));
                }
                Err(CoreError::NotFound(_)) => {}
                Err(e) => return Err(AppError::from(e)),
            }
        }
    }

    // Proxy through cache. Use the filename stem as the package name and the
    // platform as version to form a stable cache key.
    let stem = filename
        .strip_suffix(".conda")
        .or_else(|| filename.strip_suffix(".tar.bz2"))
        .unwrap_or(&filename);
    let pkg = PackageId::new(&registry, stem, &platform).with_artifact(&filename);
    proxy_stream(
        svc,
        pkg,
        identity,
        "releases:read",
        Some("application/octet-stream"),
    )
    .await
}

/// Extract package name from a conda filename.
/// e.g. `numpy-1.26.0-py311h0_0.tar.bz2` → `numpy`
#[cfg(test)]
fn conda_package_name_from_filename(filename: &str) -> String {
    let stem = filename
        .strip_suffix(".conda")
        .or_else(|| filename.strip_suffix(".tar.bz2"))
        .unwrap_or(filename);
    // conda filename: {name}-{version}-{build}
    let parts: Vec<&str> = stem.splitn(3, '-').collect();
    parts[0].to_owned()
}

/// Extract "{version}-{build}" from a conda filename for use as a local registry version key.
/// e.g. `numpy-1.26.0-py311h0_0.tar.bz2` → `"1.26.0-py311h0_0"`
#[cfg(test)]
fn conda_version_from_filename(filename: &str) -> Option<String> {
    let stem = filename
        .strip_suffix(".conda")
        .or_else(|| filename.strip_suffix(".tar.bz2"))?;
    let mut parts = stem.splitn(3, '-');
    parts.next(); // skip name
    let version = parts.next()?;
    let build = parts.next().unwrap_or("");
    if build.is_empty() {
        Some(version.to_owned())
    } else {
        Some(format!("{version}-{build}"))
    }
}

// ── Publish route ─────────────────────────────────────────────────────────────

/// Publish a conda package (`.conda` or `.tar.bz2`) to a local/hybrid registry.
///
/// Accepts the raw package bytes as the request body.  The package name, version,
/// and build string are extracted from the `info/index.json` file inside the archive.
#[utoipa::path(
    post,
    path = "/proxy/{registry}/{platform}/",
    tag = "proxy/conda",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("platform" = String, Path, description = "Target platform, e.g. linux-64"),
    ),
    responses(
        (status = 200, description = "Package published"),
        (status = 403, description = "Access denied or quota exceeded"),
        (status = 409, description = "Version already published"),
        (status = 422, description = "Invalid conda package"),
    ),
    security(("bearer_token" = [])),
)]
#[allow(clippy::too_many_arguments)]
#[post("/proxy/{registry}/{platform}/")]
pub async fn conda_publish(
    req: HttpRequest,
    path: web::Path<(String, String)>,
    payload: web::Payload,
    identity: AuthIdentity,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
    notification_svc: web::Data<Option<Arc<NotificationService>>>,
) -> Result<impl Responder, AppError> {
    let (registry, platform) = path.into_inner();
    require_registry_type(&registry, "conda", &map)?;
    require_local_mode(&registry, &mode_map)?;

    let data = collect_payload(payload).await?;

    let pkg_info = batlehub_adapters::registry::conda::parse_conda_metadata(&data)
        .map_err(|e| AppError::unprocessable(e.to_string()))?;

    let checksum = hex::encode(Sha256::digest(&data));

    // Build the filename for this package
    let ext = if data.len() >= 4 && &data[..4] == b"PK\x03\x04" {
        "conda"
    } else {
        "tar.bz2"
    };
    let filename = format!(
        "{}-{}-{}.{ext}",
        pkg_info.name, pkg_info.version, pkg_info.build
    );

    // version key = "{version}-{build}" to keep versions unique per build
    let version_key = format!("{}-{}", pkg_info.version, pkg_info.build);

    let index_metadata = serde_json::json!({
        "name": pkg_info.name,
        "version": pkg_info.version,
        "build": pkg_info.build,
        "build_number": pkg_info.build_number,
        "depends": pkg_info.depends,
        "subdir": pkg_info.subdir.unwrap_or_else(|| platform.clone()),
        "license": pkg_info.license,
        "sha256": checksum,
        "filename": filename,
    });

    let (signature_bytes, signature_type) = extract_signature_headers(&req);
    let actor = identity.0.user_id.clone().unwrap_or_default();

    let quota_check = local_svc
        .publish(PublishRequest {
            registry: registry.clone(),
            name: pkg_info.name.clone(),
            version: version_key.clone(),
            artifact: data,
            checksum,
            index_metadata,
            publisher: identity.0,
            signature_bytes,
            signature_type,
        })
        .await
        .map_err(AppError::from)?;

    super::common::dispatch_notification(
        &notification_svc,
        NotificationEventType::PackagePublished,
        &registry,
        &pkg_info.name,
        Some(version_key),
        &actor,
    );

    let mut resp = HttpResponse::Ok();
    for (header, value) in quota_check.headers() {
        resp.insert_header((header, value));
    }
    Ok(resp.json(serde_json::json!({
        "message": format!("Conda package published: {filename}")
    })))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_name_from_filename() {
        assert_eq!(
            conda_package_name_from_filename("numpy-1.26.0-py311h0_0.tar.bz2"),
            "numpy"
        );
        assert_eq!(
            conda_package_name_from_filename("bzip2-1.0.8-h5eee18b_5.conda"),
            "bzip2"
        );
    }

    #[test]
    fn version_from_filename() {
        assert_eq!(
            conda_version_from_filename("numpy-1.26.0-py311h0_0.tar.bz2"),
            Some("1.26.0-py311h0_0".to_owned())
        );
    }

    #[test]
    fn merge_repodata_combines_packages() {
        let upstream = serde_json::json!({
            "packages": { "pkgA-1.0-0.tar.bz2": { "name": "pkgA" } },
            "packages.conda": {}
        });
        let local = serde_json::json!({
            "packages": { "pkgB-1.0-0.tar.bz2": { "name": "pkgB" } },
            "packages.conda": {}
        });
        let upstream_bytes = serde_json::to_vec(&upstream).unwrap();
        let merged_bytes = merge_repodata(&upstream_bytes, &local);
        let merged: serde_json::Value = serde_json::from_slice(&merged_bytes).unwrap();
        assert!(merged["packages"].get("pkgA-1.0-0.tar.bz2").is_some());
        assert!(merged["packages"].get("pkgB-1.0-0.tar.bz2").is_some());
    }
}
