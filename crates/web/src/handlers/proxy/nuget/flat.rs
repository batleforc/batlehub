use std::sync::Arc;

use actix_web::{get, web, HttpResponse, Responder};

use batlehub_config::schema::RegistryMode;
use batlehub_core::{
    entities::PackageId,
    error::CoreError,
    services::{artifact_storage_key, LocalRegistryService, ProxyService},
};

use super::super::common::{
    append_signature_headers, collect_storage_stream, proxy_stream, require_registry_type,
};
use super::nuspec::{content_type_for, extract_nuspec_from_nupkg};
use crate::{error::AppError, extractors::AuthIdentity, RegistryMap, RegistryModeMap};

// ── Flat container — version list ─────────────────────────────────────────────

/// Return the list of available versions for a NuGet package (flat container).
///
/// In `local`/`hybrid` mode this is generated from locally published packages.
/// In `proxy` mode it is fetched from the upstream flat container.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/nuget/v3/flat/{id}/index.json",
    tag = "proxy/nuget",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("id" = String, Path, description = "Package ID (case-insensitive)"),
    ),
    responses(
        (status = 200, description = "Version list JSON"),
        (status = 404, description = "Package not found"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/nuget/v3/flat/{id}/index.json")]
pub async fn nuget_flat_versions(
    path: web::Path<(String, String)>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
) -> Result<impl Responder, AppError> {
    let (registry, id_raw) = path.into_inner();
    require_registry_type(&registry, "nuget", &map)?;

    let id = id_raw.to_lowercase();
    let mode = mode_map.get(&registry);

    if matches!(mode, RegistryMode::Local | RegistryMode::Hybrid) {
        match local_svc
            .get_nuget_versions(&registry, &id, &identity)
            .await
        {
            Ok(versions) => {
                let version_list: Vec<&str> = versions
                    .iter()
                    .filter(|v| !v.yanked)
                    .map(|v| v.version.as_str())
                    .collect();
                let body = serde_json::json!({ "versions": version_list });
                return Ok(HttpResponse::Ok()
                    .content_type("application/json")
                    .json(body));
            }
            Err(CoreError::NotFound(_)) if mode == RegistryMode::Hybrid => {
                // fall through to upstream proxy
            }
            Err(CoreError::NotFound(msg)) => return Err(AppError::not_found(msg)),
            Err(e) => return Err(AppError::from(e)),
        }
    }

    // Proxy mode or hybrid miss
    proxy_stream(
        svc,
        PackageId::new(&registry, &id, "__index__"),
        identity,
        "releases:read",
        Some("application/json"),
    )
    .await
}

// ── Flat container — artifact download ───────────────────────────────────────

/// Download a NuGet package artifact (`.nupkg`, `.nuspec`, checksum, etc.).
#[utoipa::path(
    get,
    path = "/proxy/{registry}/nuget/v3/flat/{id}/{version}/{filename}",
    tag = "proxy/nuget",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("id"       = String, Path, description = "Package ID"),
        ("version"  = String, Path, description = "Package version"),
        ("filename" = String, Path, description = "Artifact filename"),
    ),
    responses(
        (status = 200, description = "Artifact bytes"),
        (status = 404, description = "Artifact not found"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/nuget/v3/flat/{id}/{version}/{filename}")]
pub async fn nuget_flat_download(
    path: web::Path<(String, String, String, String)>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
) -> Result<impl Responder, AppError> {
    let (registry, id_raw, version, filename) = path.into_inner();
    require_registry_type(&registry, "nuget", &map)?;

    let id = id_raw.to_lowercase();
    let mode = mode_map.get(&registry);

    if matches!(mode, RegistryMode::Local | RegistryMode::Hybrid) {
        local_svc
            .check_prerelease_access(&registry, &version, &identity)
            .await
            .map_err(AppError::from)?;

        let storage_key = artifact_storage_key(&registry, &id, &version);
        match local_svc.storage.retrieve(&storage_key).await {
            Ok(Some(artifact)) => {
                let buf = collect_storage_stream(artifact.stream).await?;
                let body = if filename.ends_with(".nuspec") {
                    extract_nuspec_from_nupkg(&buf)?
                } else {
                    buf.to_vec()
                };
                let mut resp = HttpResponse::Ok();
                resp.content_type(content_type_for(&filename));
                append_signature_headers(&mut resp, &local_svc, &registry, &id, &version).await;
                return Ok(resp.body(body));
            }
            Ok(None) if mode == RegistryMode::Hybrid => {} // fall through
            Ok(None) => {
                return Err(AppError::not_found(format!(
                    "{id}@{version} not found in local registry"
                )));
            }
            Err(e) if mode == RegistryMode::Hybrid => {
                tracing::warn!("local storage error, falling back to proxy: {e}");
            }
            Err(e) => return Err(AppError::from(e)),
        }
    }

    proxy_stream(
        svc,
        PackageId::new(&registry, &id, &version).with_artifact(&filename),
        identity,
        "releases:read",
        Some(content_type_for(&filename)),
    )
    .await
}
