use std::sync::Arc;

use actix_multipart::Multipart;
use actix_web::{delete, get, put, web, HttpRequest, HttpResponse, Responder};
use bytes::BytesMut;
use futures::StreamExt;
use sha2::{Digest, Sha256};

use batlehub_core::{
    entities::NotificationEventType,
    services::{LocalRegistryService, ProxyService, PublishRequest},
};

use super::super::common::{
    dispatch_notification, extract_signature_headers, publish_and_respond, require_local_mode,
    require_registry_type, MAX_UPLOAD_BYTES,
};
use super::nuspec::{extract_nuspec_from_nupkg, parse_nuspec};
use crate::handlers::schemas::{MessageResponse, ProtocolDocument, UpstreamDocument};
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
    /// Offset into the result set. `dotnet package search` sends
    /// `skip=0&take=20` and then `skip=20` for the next page — ignoring it
    /// makes every page return the same first results, which reads as "the
    /// registry only has 20 packages" (RFC 0009 §12.4).
    #[serde(default)]
    skip: usize,
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
        (status = 200, description = "Search results JSON", body = UpstreamDocument),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/nuget/v3/query")]
pub async fn nuget_search(
    path: web::Path<String>,
    query: web::Query<SearchQuery>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
) -> Result<impl Responder, AppError> {
    let registry = path.into_inner();
    require_registry_type(&registry, "nuget", &map)?;

    // RFC 0009 §7.7. Proxy and hybrid mode used to return
    // `{"totalHits": 0, "data": []}` unconditionally, under a comment saying it
    // was so the dotnet CLI "functions without error" — while `service_index.rs`
    // advertised `SearchQueryService` pointing here. So `dotnet package search`
    // reported zero results against a registry holding thousands of packages,
    // and nothing anywhere looked broken.
    //
    // `ProxyService::search` is the shared three-rung path: cached, then
    // upstream, then what this registry actually holds. The last rung is why
    // the service index can go on advertising this endpoint — there is now no
    // state in which it has nothing to say.
    let _ = &identity;
    let results = svc
        .search(
            &registry,
            &query.q,
            query.take,
            crate::handlers::proxy::search::search_mode(mode_map.get(&registry)),
            crate::handlers::proxy::search::local_hits(&local_svc, &registry, &query.q, query.take)
                .await,
        )
        .await
        .map_err(AppError::from)?;

    let data: Vec<serde_json::Value> = results
        .hits
        .iter()
        .skip(query.skip)
        .map(|h| {
            serde_json::json!({
                "id": h.name,
                "version": h.version,
                "description": h.description.clone().unwrap_or_default(),
                // NuGet clients read `versions[]` for the version picker. One
                // entry — the one this hit names — is honest; inventing a range
                // we have not checked would advertise versions that may be
                // blocked.
                "versions": [ { "version": h.version, "downloads": 0 } ],
            })
        })
        .collect();

    Ok(HttpResponse::Ok()
        .content_type("application/json")
        .insert_header(("X-BatleHub-Cache", results.freshness.header_value()))
        .json(serde_json::json!({ "totalHits": results.total, "data": data })))
}

/// `SearchAutocompleteService` — package-id completion.
///
/// Advertised by the service index since RFC 0009 §7.6 and answered from the
/// same three-rung search path, so completion cannot offer a package the search
/// below it would not return, nor a version a block has hidden.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/nuget/v3/autocomplete",
    tag = "proxy/nuget",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("q"        = Option<String>, Query, description = "Partial package id"),
        ("take"     = Option<usize>, Query, description = "Maximum results"),
    ),
    responses(
        (status = 200, description = "Package id completions", body = ProtocolDocument),
        (status = 404, description = "Unknown or non-nuget registry"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/nuget/v3/autocomplete")]
pub async fn nuget_autocomplete(
    path: web::Path<String>,
    query: web::Query<SearchQuery>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
) -> Result<impl Responder, AppError> {
    let registry = path.into_inner();
    require_registry_type(&registry, "nuget", &map)?;
    let _ = &identity;

    let results = svc
        .search(
            &registry,
            &query.q,
            query.take,
            crate::handlers::proxy::search::search_mode(mode_map.get(&registry)),
            crate::handlers::proxy::search::local_hits(&local_svc, &registry, &query.q, query.take)
                .await,
        )
        .await
        .map_err(AppError::from)?;

    // The autocomplete document is ids only — no versions, no descriptions.
    let data: Vec<String> = results
        .hits
        .iter()
        .skip(query.skip)
        .map(|h| h.name.clone())
        .collect();

    Ok(HttpResponse::Ok()
        .content_type("application/json")
        .insert_header(("X-BatleHub-Cache", results.freshness.header_value()))
        .json(serde_json::json!({ "totalHits": data.len(), "data": data })))
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
        (status = 201, description = "Package published", body = MessageResponse),
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
    // Raw `Multipart` is not covered by `PayloadConfig`, so bound the cumulative
    // accumulation ourselves — otherwise an unauthenticated client could stream
    // an unbounded body into memory (OOM). Same ceiling as `collect_payload`.
    let mut total_bytes: u64 = 0;
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
            total_bytes += chunk.len() as u64;
            if total_bytes > MAX_UPLOAD_BYTES {
                return Err(AppError::from(
                    batlehub_core::error::CoreError::PayloadTooLarge(format!(
                        "upload exceeds the {MAX_UPLOAD_BYTES}-byte limit"
                    )),
                ));
            }
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

    publish_and_respond(
        &local_svc,
        &notification_svc,
        PublishRequest {
            unlisted: false,
            registry: registry.clone(),
            name: id_lower.clone(),
            version: version.clone(),
            artifact: nupkg_bytes,
            checksum,
            index_metadata,
            publisher: identity.0,
            signature_bytes,
            signature_type,
        },
        actix_web::http::StatusCode::CREATED,
        MessageResponse::new(format!("Successfully published {id_lower} {version}")),
    )
    .await
}

/// Publish a `.snupkg` symbol package.
///
/// RFC 0009 §7.6. `nuget push` sends symbols here when the service index
/// advertises `SymbolPackagePublish` — and it did not, so `dotnet nuget push`
/// of a `.snupkg` failed with no route rather than with an answer.
///
/// Stored under the **same coordinate** as its `.nupkg` with a `snupkg`
/// artifact sub-coordinate (RFC 0009 §11.3), not as a separate package. Symbol
/// *servers* are conventionally separate infrastructure, but that is a
/// deployment convention of the ecosystem rather than a property of the
/// artifact — and splitting the coordinate would put a package's symbols
/// outside every policy that addresses the package.
#[utoipa::path(
    put,
    path = "/proxy/{registry}/nuget/api/v2/symbolpackage",
    tag = "proxy/nuget",
    params(("registry" = String, Path, description = "Registry name")),
    responses(
        (status = 201, description = "Symbol package published", body = MessageResponse),
        (status = 400, description = "Invalid or missing .snupkg"),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Unknown registry, or not in local/hybrid mode"),
    ),
    security(("bearer_token" = [])),
)]
#[allow(clippy::too_many_arguments)]
#[put("/proxy/{registry}/nuget/api/v2/symbolpackage")]
pub async fn nuget_symbol_publish(
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

    let mut snupkg_opt: Option<bytes::Bytes> = None;
    let mut total_bytes: u64 = 0;
    while let Some(field_result) = multipart.next().await {
        let mut field =
            field_result.map_err(|e| AppError::bad_request(format!("multipart error: {e}")))?;
        let mut buf = BytesMut::new();
        while let Some(chunk) = field.next().await {
            let chunk = chunk.map_err(|e| AppError::bad_request(format!("chunk error: {e}")))?;
            total_bytes += chunk.len() as u64;
            if total_bytes > MAX_UPLOAD_BYTES {
                return Err(AppError::from(
                    batlehub_core::error::CoreError::PayloadTooLarge(format!(
                        "upload exceeds the {MAX_UPLOAD_BYTES}-byte limit"
                    )),
                ));
            }
            buf.extend_from_slice(&chunk);
        }
        if snupkg_opt.is_none() {
            snupkg_opt = Some(buf.freeze());
        }
    }
    let snupkg_bytes =
        snupkg_opt.ok_or_else(|| AppError::bad_request("no .snupkg in multipart body"))?;
    if snupkg_bytes.is_empty() {
        return Err(AppError::bad_request("empty .snupkg body"));
    }

    // A `.snupkg` is a ZIP with a `.nuspec` exactly like a `.nupkg`, so the
    // coordinate is read the same way — which is what lets it share one.
    let nuspec = parse_nuspec(&extract_nuspec_from_nupkg(&snupkg_bytes)?)?;
    if nuspec.version.is_empty() {
        return Err(AppError::unprocessable("nuspec missing <version>"));
    }
    let id_lower = nuspec.id.to_lowercase();
    let version = nuspec.version.clone();
    let checksum = hex::encode(Sha256::digest(&snupkg_bytes));

    let (signature_bytes, signature_type) = extract_signature_headers(&req);

    publish_and_respond(
        &local_svc,
        &notification_svc,
        PublishRequest {
            unlisted: false,
            registry: registry.clone(),
            name: format!("{id_lower}/snupkg"),
            version: version.clone(),
            artifact: snupkg_bytes,
            checksum,
            index_metadata: serde_json::json!({
                "id": nuspec.id,
                "version": version,
                "symbols": true,
                "sha256": hex::encode(Sha256::digest(b"")),
            }),
            publisher: identity.0,
            signature_bytes,
            signature_type,
        },
        actix_web::http::StatusCode::CREATED,
        MessageResponse::new(format!(
            "Successfully published symbols for {id_lower} {version}"
        )),
    )
    .await
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
