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
use crate::handlers::schemas::{ArtifactBytes, MessageResponse, UpstreamDocument};
use crate::{
    error::AppError, extractors::AuthIdentity, services::NotificationService, RegistryMap,
    RegistryModeMap,
};

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
        (status = 200, description = "repodata.json", body = UpstreamDocument),
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

    // Proxy mode, and the upstream half of Hybrid. `multi_package_document`
    // rather than `proxy_stream`: `repodata.json` describes a whole channel, and
    // a blocked package left in it gets selected by the solver and then refused
    // at download — mid `conda install`, after the environment plan is fixed.
    //
    // Its blocked set comes from a 30-second snapshot rather than a per-request
    // query; see `ProxyService::blocked_in_registry_snapshot` for why, and the
    // admin guide for the operator-facing statement of the delay.
    let upstream = fetch_conda_index(
        svc,
        &registry,
        &platform,
        identity,
        batlehub_core::ports::DocumentKind::Versions,
    )
    .await?;

    if mode == RegistryMode::Hybrid {
        let local_repodata = local_svc
            .get_conda_repodata(&registry, &platform)
            .await
            .map_err(AppError::from)?;
        let merged = merge_repodata(&upstream, &local_repodata);
        return Ok(HttpResponse::Ok()
            .content_type("application/json")
            .body(merged));
    }

    Ok(HttpResponse::Ok()
        .content_type("application/json")
        .body(upstream))
}

/// Fetch and filter one of a conda channel's index documents, as bytes.
///
/// Bytes rather than a `VersionDocument` because the Hybrid path has to merge
/// locally published packages into it before answering.
async fn fetch_conda_index(
    svc: web::Data<Arc<ProxyService>>,
    registry: &str,
    platform: &str,
    identity: AuthIdentity,
    kind: batlehub_core::ports::DocumentKind,
) -> Result<Vec<u8>, AppError> {
    // The *platform* is the coordinate: a conda listing is scoped to a subdir,
    // not to a package.
    let req = batlehub_core::services::ProxyRequest {
        package_id: PackageId::new(registry, platform, "__repodata__"),
        identity: identity.0,
        resource_type: batlehub_core::rules::resource_type::RELEASES_READ.to_owned(),
        ip_address: None,
        user_agent: None,
    };
    let doc = svc
        .multi_package_document(&req, kind, "")
        .await
        .map_err(AppError::from)?;
    Ok(match doc.body {
        batlehub_core::ports::DocumentBody::Json(v) => serde_json::to_vec(&v).unwrap_or_default(),
        batlehub_core::ports::DocumentBody::Text(t) => t.into_bytes(),
    })
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
        (status = 200, description = "current_repodata.json", body = UpstreamDocument),
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

    let body = fetch_conda_index(
        svc,
        &registry,
        &platform,
        identity,
        batlehub_core::ports::DocumentKind::CURRENT_REPODATA,
    )
    .await?;
    Ok(HttpResponse::Ok()
        .content_type("application/json")
        .body(body))
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
        (status = 200, description = "Package bytes", body = ArtifactBytes, content_type = "application/octet-stream"),
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
    // `platform` is part of the route but not of the coordinate: a conda
    // package's identity is its name and version, and the same release is
    // served under several subdirs.
    let (registry, _platform, filename) = path.into_inner();
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

    // Proxy through cache, addressed by the *package* coordinate the filename
    // encodes rather than by the filename stem and the platform.
    //
    // The coordinate is what the rule chain judges. Named
    // `("numpy-1.1.0-py311_0", "linux-64")` — as this route used to be — a block
    // recorded against `numpy@1.1.0` matches nothing and the download is
    // allowed, which would make conda the one ecosystem where hiding a version
    // from the channel index is the *only* half of a block that works. The
    // filename stays as the artifact sub-coordinate, so two builds of one
    // version keep distinct cache entries.
    let (name, version) = parse_conda_filename(&filename)
        .ok_or_else(|| AppError::bad_request(format!("unparseable conda filename: {filename}")))?;
    let pkg = PackageId::new(&registry, name, version).with_artifact(&filename);
    proxy_stream(
        svc,
        pkg,
        identity,
        batlehub_core::rules::resource_type::RELEASES_READ,
        Some("application/octet-stream"),
    )
    .await
}

/// Split a conda filename into its `(name, version)`.
///
/// Conda filenames are `{name}-{version}-{build}.{tar.bz2,conda}` and **the name
/// may contain hyphens** (`ruamel-yaml-0.17.21-py311_0.conda`), so the split is
/// from the right: the last two fields are the build and the version, and
/// whatever precedes them is the name.
///
/// `None` for anything that does not have all three fields, which the caller
/// turns into a `400` rather than guessing at a coordinate the rule chain would
/// then judge.
fn parse_conda_filename(filename: &str) -> Option<(&str, &str)> {
    let stem = filename
        .strip_suffix(".conda")
        .or_else(|| filename.strip_suffix(".tar.bz2"))?;
    let (rest, _build) = stem.rsplit_once('-')?;
    let (name, version) = rest.rsplit_once('-')?;
    if name.is_empty() || version.is_empty() {
        return None;
    }
    Some((name, version))
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
        (status = 200, description = "Package published", body = MessageResponse),
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

    super::common::publish_and_respond(
        &local_svc,
        &notification_svc,
        PublishRequest {
            unlisted: false,
            registry,
            name: pkg_info.name.clone(),
            version: version_key,
            artifact: data,
            checksum,
            index_metadata,
            publisher: identity.0,
            signature_bytes,
            signature_type,
        },
        actix_web::http::StatusCode::OK,
        MessageResponse::new(format!("Conda package published: {filename}")),
    )
    .await
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
