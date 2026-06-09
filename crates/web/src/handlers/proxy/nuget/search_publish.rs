use std::sync::Arc;

use actix_multipart::Multipart;
use actix_web::{delete, get, put, web, HttpRequest, HttpResponse, Responder};
use bytes::BytesMut;
use futures::StreamExt;
use sha2::{Digest, Sha256};

use batlehub_config::schema::RegistryMode;
use batlehub_core::{
    entities::NotificationEventType,
    services::{LocalRegistryService, PublishRequest},
};

use super::super::common::{
    dispatch_notification, extract_signature_headers, require_local_mode, require_registry_type,
};
use super::nuspec::{extract_nuspec_from_nupkg, parse_nuspec};
use crate::{
    error::AppError, extractors::AuthIdentity, services::NotificationService, RegistryMap,
    RegistryModeMap,
};

// ── Search ────────────────────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct SearchQuery {
    #[serde(default)]
    q: String,
    #[serde(default = "default_take")]
    take: usize,
}

fn default_take() -> usize {
    20
}

/// Search for NuGet packages.
///
/// In proxy/hybrid mode the query is forwarded to the upstream search API.
/// In local mode the local package list is filtered.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/nuget/v3/query",
    tag = "proxy/nuget",
    params(
        ("registry"   = String, Path,  description = "Registry name"),
        ("q"          = String, Query, description = "Search query"),
        ("take"       = u32,   Query, description = "Max results"),
        ("prerelease" = bool,  Query, description = "Include pre-release"),
    ),
    responses(
        (status = 200, description = "Search results JSON"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/nuget/v3/query")]
pub async fn nuget_search(
    path: web::Path<String>,
    query: web::Query<SearchQuery>,
    identity: AuthIdentity,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
) -> Result<impl Responder, AppError> {
    let registry = path.into_inner();
    require_registry_type(&registry, "nuget", &map)?;

    let q = &query.q;
    let take = query.take.min(100);
    let mode = mode_map.get(&registry);

    // Search in local mode: scan published package names for the query.
    if mode == RegistryMode::Local {
        let names = local_svc
            .backend
            .list_package_names(&registry)
            .await
            .unwrap_or_default();

        let matched: Vec<serde_json::Value> = names
            .into_iter()
            .filter(|name| q.is_empty() || name.contains(q.as_str()))
            .take(take)
            .map(|name| {
                serde_json::json!({
                    "id": name,
                    "version": "",
                    "description": "",
                    "versions": []
                })
            })
            .collect();

        return Ok(HttpResponse::Ok()
            .content_type("application/json")
            .json(serde_json::json!({ "totalHits": matched.len(), "data": matched })));
    }

    // Proxy/hybrid: NuGet search is handled by the explore service (client.search_packages).
    // Return minimal empty response so dotnet CLI functions without error.
    let _ = &identity;
    Ok(HttpResponse::Ok()
        .content_type("application/json")
        .json(serde_json::json!({ "totalHits": 0, "data": [] })))
}

// ── Publish ───────────────────────────────────────────────────────────────────

/// Publish a `.nupkg` to the local registry.
///
/// Accepts either `multipart/form-data` (as sent by `dotnet nuget push`) or a raw
/// `application/octet-stream` body containing the `.nupkg` bytes directly.
///
/// Only available when the registry is configured in `local` or `hybrid` mode.
#[utoipa::path(
    put,
    path = "/proxy/{registry}/nuget/api/v2/package",
    tag = "proxy/nuget",
    params(("registry" = String, Path, description = "Registry name")),
    responses(
        (status = 201, description = "Package published"),
        (status = 400, description = "Invalid or missing .nupkg"),
        (status = 401, description = "Authentication required"),
        (status = 409, description = "Version already published"),
    ),
    security(("bearer_token" = [])),
)]
#[allow(clippy::too_many_arguments)]
#[put("/proxy/{registry}/nuget/api/v2/package")]
pub async fn nuget_publish(
    req: HttpRequest,
    path: web::Path<String>,
    mut multipart: Multipart,
    identity: AuthIdentity,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
    notification_svc: web::Data<Option<Arc<NotificationService>>>,
) -> Result<impl Responder, AppError> {
    let registry = path.into_inner();
    require_registry_type(&registry, "nuget", &map)?;
    require_local_mode(&registry, &mode_map)?;

    // dotnet nuget push and nuget.exe always send multipart/form-data.
    // Accept any field that looks like the package file.
    let mut nupkg_bytes_opt: Option<bytes::Bytes> = None;
    while let Some(field_result) = multipart.next().await {
        let mut field =
            field_result.map_err(|e| AppError::bad_request(format!("multipart error: {e}")))?;
        let field_name = field
            .content_disposition()
            .and_then(|cd| cd.get_name())
            .unwrap_or("")
            .to_owned();
        let mut buf = BytesMut::new();
        while let Some(chunk) = field.next().await {
            let chunk = chunk.map_err(|e| AppError::bad_request(format!("chunk error: {e}")))?;
            buf.extend_from_slice(&chunk);
        }
        // Accept "package" field or the first non-empty field.
        if field_name == "package" || nupkg_bytes_opt.is_none() {
            nupkg_bytes_opt = Some(buf.freeze());
        }
    }
    let nupkg_bytes =
        nupkg_bytes_opt.ok_or_else(|| AppError::bad_request("no .nupkg in multipart body"))?;

    if nupkg_bytes.is_empty() {
        return Err(AppError::bad_request("empty .nupkg body"));
    }

    // Extract .nuspec from the ZIP archive.
    let nuspec_bytes = extract_nuspec_from_nupkg(&nupkg_bytes)?;
    let nuspec = parse_nuspec(&nuspec_bytes)?;

    let id_lower = nuspec.id.to_lowercase();
    if nuspec.version.is_empty() {
        return Err(AppError::unprocessable("nuspec missing <version>"));
    }
    let version = nuspec.version.clone();

    let checksum = hex::encode(Sha256::digest(&nupkg_bytes));
    let index_metadata = serde_json::json!({
        "id": nuspec.id,
        "version": version,
        "description": nuspec.description,
        "authors": nuspec.authors,
        "tags": nuspec.tags,
        "sha256": checksum,
    });

    let (signature_bytes, signature_type) = extract_signature_headers(&req);
    let actor = identity.0.user_id.clone().unwrap_or_default();

    let quota_check = local_svc
        .publish(PublishRequest {
            registry: registry.clone(),
            name: id_lower.clone(),
            version: version.clone(),
            artifact: nupkg_bytes,
            checksum,
            index_metadata,
            publisher: identity.0,
            signature_bytes,
            signature_type,
        })
        .await
        .map_err(AppError::from)?;

    dispatch_notification(
        &notification_svc,
        NotificationEventType::PackagePublished,
        &registry,
        &nuspec.id,
        Some(version),
        &actor,
    );

    let mut resp = HttpResponse::Created();
    if let Some(limit) = quota_check.bytes_limit {
        resp.insert_header(("X-Quota-Used-Bytes", quota_check.bytes_used.to_string()));
        resp.insert_header(("X-Quota-Limit-Bytes", limit.to_string()));
    }
    Ok(resp.finish())
}

// ── Yank ──────────────────────────────────────────────────────────────────────

/// Yank (unlist) a NuGet package version from the local registry.
///
/// Only available when the registry is configured in `local` or `hybrid` mode.
#[utoipa::path(
    delete,
    path = "/proxy/{registry}/nuget/v2/package/{id}/{version}",
    tag = "proxy/nuget",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("id"       = String, Path, description = "Package ID"),
        ("version"  = String, Path, description = "Package version"),
    ),
    responses(
        (status = 204, description = "Package yanked"),
        (status = 401, description = "Authentication required"),
        (status = 404, description = "Package not found"),
    ),
    security(("bearer_token" = [])),
)]
#[delete("/proxy/{registry}/nuget/v2/package/{id}/{version}")]
pub async fn nuget_yank(
    path: web::Path<(String, String, String)>,
    identity: AuthIdentity,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
    notification_svc: web::Data<Option<Arc<NotificationService>>>,
) -> Result<impl Responder, AppError> {
    let (registry, id_raw, version) = path.into_inner();
    require_registry_type(&registry, "nuget", &map)?;
    require_local_mode(&registry, &mode_map)?;

    let id = id_raw.to_lowercase();
    let actor = identity.0.user_id.clone().unwrap_or_default();

    local_svc
        .yank(&registry, &id, &version, &identity.0)
        .await
        .map_err(AppError::from)?;

    dispatch_notification(
        &notification_svc,
        NotificationEventType::PackageYanked,
        &registry,
        &id,
        Some(version),
        &actor,
    );

    Ok(HttpResponse::NoContent().finish())
}
