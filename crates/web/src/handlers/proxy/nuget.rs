use std::sync::Arc;

use actix_multipart::Multipart;
use actix_web::{delete, get, put, web, HttpRequest, HttpResponse, Responder};
use bytes::BytesMut;
use futures::StreamExt;
use quick_xml::{events::Event as XmlEvent, Reader as XmlReader};
use sha2::{Digest, Sha256};
use zip::ZipArchive;

use batlehub_config::schema::RegistryMode;
use batlehub_core::{
    entities::PackageId,
    error::CoreError,
    services::{artifact_storage_key, LocalRegistryService, ProxyService, PublishRequest},
};

use super::common::{
    append_signature_headers, extract_signature_headers, proxy_stream, require_local_mode,
};
use crate::{error::AppError, extractors::AuthIdentity, RegistryMap, RegistryModeMap};

// ── Guard ─────────────────────────────────────────────────────────────────────

fn require_nuget(registry: &str, map: &RegistryMap) -> Result<(), AppError> {
    match map.type_of(registry).as_deref() {
        Some("nuget") => Ok(()),
        Some(_) => Err(AppError::not_found(format!(
            "registry '{registry}' is not a NuGet registry"
        ))),
        None => Err(AppError::not_found(format!(
            "unknown registry '{registry}'"
        ))),
    }
}

// ── Content-type helpers ──────────────────────────────────────────────────────

fn content_type_for(filename: &str) -> &'static str {
    if filename.ends_with(".nupkg") {
        "application/octet-stream"
    } else if filename.ends_with(".nuspec") || filename.ends_with(".xml") {
        "application/xml"
    } else if filename.ends_with(".sha512") || filename.ends_with(".sha256") {
        "text/plain"
    } else {
        "application/octet-stream"
    }
}

// ── .nuspec parser ────────────────────────────────────────────────────────────

struct NuspecMetadata {
    id: String,
    version: String,
    description: Option<String>,
    authors: Option<String>,
    tags: Option<String>,
}

fn parse_nuspec(bytes: &[u8]) -> Result<NuspecMetadata, AppError> {
    let mut reader = XmlReader::from_reader(bytes);
    reader.config_mut().trim_text(true);

    let mut id = None::<String>;
    let mut version = None::<String>;
    let mut description = None::<String>;
    let mut authors = None::<String>;
    let mut tags = None::<String>;
    let mut depth: u32 = 0;
    let mut current_tag = String::new();
    let mut in_metadata = false;
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(XmlEvent::Start(e)) => {
                depth += 1;
                let local = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
                if local == "metadata" {
                    in_metadata = true;
                }
                if in_metadata && depth == 3 {
                    current_tag = local;
                }
            }
            Ok(XmlEvent::Text(e)) if in_metadata && depth == 3 => {
                let raw = e
                    .decode()
                    .map_err(|e| AppError::unprocessable(format!("nuspec parse: {e}")))?;
                let text = quick_xml::escape::unescape(&raw)
                    .map_err(|e| AppError::unprocessable(format!("nuspec parse: {e}")))?
                    .into_owned();
                match current_tag.as_str() {
                    "id" => id = Some(text),
                    "version" => version = Some(text),
                    "description" => description = Some(text),
                    "authors" => authors = Some(text),
                    "tags" => tags = Some(text),
                    _ => {}
                }
            }
            Ok(XmlEvent::End(_)) => {
                if depth == 3 {
                    current_tag.clear();
                }
                depth = depth.saturating_sub(1);
                if depth < 2 {
                    in_metadata = false;
                }
            }
            Ok(XmlEvent::Eof) => break,
            Err(e) => return Err(AppError::unprocessable(format!("nuspec parse error: {e}"))),
            _ => {}
        }
        buf.clear();
    }

    let id = id
        .filter(|s| !s.is_empty())
        .ok_or_else(|| AppError::unprocessable("nuspec missing <id>"))?;
    let version = version.unwrap_or_default();

    Ok(NuspecMetadata {
        id,
        version,
        description,
        authors,
        tags,
    })
}

/// Extract the `.nuspec` from a `.nupkg` ZIP archive.
fn extract_nuspec_from_nupkg(bytes: &[u8]) -> Result<Vec<u8>, AppError> {
    let cursor = std::io::Cursor::new(bytes);
    let mut archive = ZipArchive::new(cursor)
        .map_err(|e| AppError::unprocessable(format!("invalid .nupkg (not a ZIP): {e}")))?;

    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| AppError::unprocessable(format!("zip entry error: {e}")))?;
        if file.name().ends_with(".nuspec") {
            let mut buf = Vec::new();
            use std::io::Read;
            file.read_to_end(&mut buf)
                .map_err(|e| AppError::unprocessable(format!("reading nuspec: {e}")))?;
            return Ok(buf);
        }
    }

    Err(AppError::unprocessable(
        "no .nuspec found in .nupkg archive",
    ))
}

// ── Service index ─────────────────────────────────────────────────────────────

/// Return a NuGet v3 service index pointing all resource URLs back to this proxy.
///
/// The dotnet client fetches this first to discover where to download packages,
/// where to publish, where to search, etc.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/nuget/v3/index.json",
    tag = "proxy/nuget",
    params(("registry" = String, Path, description = "Registry name")),
    responses(
        (status = 200, description = "NuGet v3 service index"),
        (status = 404, description = "Registry not found or not a NuGet registry"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/nuget/v3/index.json")]
pub async fn nuget_service_index(
    req: HttpRequest,
    path: web::Path<String>,
    identity: AuthIdentity,
    map: web::Data<RegistryMap>,
) -> Result<impl Responder, AppError> {
    let registry = path.into_inner();
    require_nuget(&registry, &map)?;

    // Build the base URL from the incoming request so the service index works
    // behind reverse proxies and in local dev alike.
    let conn = req.connection_info();
    let base = format!("{}://{}", conn.scheme(), conn.host());
    drop(conn);

    let _ = &identity; // auth enforced by middleware; referenced to satisfy extractor

    let index = serde_json::json!({
        "version": "3.0.0",
        "resources": [
            {
                "@id": format!("{base}/proxy/{registry}/nuget/v3/registration5/"),
                "@type": "RegistrationsBaseUrl/3.6.0",
                "comment": "Base URL for NuGet package registration (metadata)"
            },
            {
                "@id": format!("{base}/proxy/{registry}/nuget/v3/flat/"),
                "@type": "PackageBaseAddress/3.0.0",
                "comment": "Base URL for NuGet package content (flat container)"
            },
            {
                "@id": format!("{base}/proxy/{registry}/nuget/api/v2/package"),
                "@type": "PackagePublish/2.0.0",
                "comment": "Publish .nupkg files"
            },
            {
                "@id": format!("{base}/proxy/{registry}/nuget/v3/query"),
                "@type": "SearchQueryService",
                "comment": "NuGet package search"
            },
            {
                "@id": format!("{base}/proxy/{registry}/nuget/v3/query"),
                "@type": "SearchQueryService/3.5.0",
                "comment": "NuGet package search"
            }
        ]
    });

    Ok(HttpResponse::Ok()
        .content_type("application/json")
        .json(index))
}

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
    require_nuget(&registry, &map)?;

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
    require_nuget(&registry, &map)?;

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
                let mut buf = Vec::new();
                let mut stream = artifact.stream;
                while let Some(chunk) = stream.next().await {
                    buf.extend_from_slice(
                        &chunk.map_err(|e| AppError::internal(e.to_string()))?,
                    );
                }
                let mut resp = HttpResponse::Ok();
                resp.content_type(content_type_for(&filename));
                append_signature_headers(&mut resp, &local_svc, &registry, &id, &version).await;
                return Ok(resp.body(buf));
            }
            Ok(None) if mode == RegistryMode::Hybrid => {} // fall through
            Ok(None) => {
                return Err(AppError::not_found(format!(
                    "{id}@{version}/{filename} not found in local registry"
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

// ── Registration metadata ─────────────────────────────────────────────────────

/// Return NuGet v3 registration metadata for a package.
///
/// In `local` mode this is generated from the DB. In proxy/hybrid mode it is
/// fetched from the upstream registration API.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/nuget/v3/registration5/{id}/index.json",
    tag = "proxy/nuget",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("id"       = String, Path, description = "Package ID"),
    ),
    responses(
        (status = 200, description = "Registration index JSON"),
        (status = 404, description = "Package not found"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/nuget/v3/registration5/{id}/index.json")]
pub async fn nuget_registration(
    req: HttpRequest,
    path: web::Path<(String, String)>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
) -> Result<impl Responder, AppError> {
    let (registry, id_raw) = path.into_inner();
    require_nuget(&registry, &map)?;

    let id = id_raw.to_lowercase();
    let mode = mode_map.get(&registry);

    if mode == RegistryMode::Local {
        let versions = local_svc
            .get_nuget_versions(&registry, &id, &identity)
            .await
            .map_err(AppError::from)?;

        let conn = req.connection_info();
        let base = format!("{}://{}", conn.scheme(), conn.host());
        drop(conn);

        let items: Vec<serde_json::Value> = versions
            .iter()
            .filter(|v| !v.yanked)
            .map(|v| {
                let pkg_content = format!(
                    "{base}/proxy/{registry}/nuget/v3/flat/{id}/{}/{id}.{}.nupkg",
                    v.version, v.version
                );
                let published = v.published_at.to_rfc3339();
                let original_id = v
                    .index_metadata
                    .get("id")
                    .and_then(|s| s.as_str())
                    .unwrap_or(&id);
                let description = v
                    .index_metadata
                    .get("description")
                    .and_then(|s| s.as_str())
                    .unwrap_or("");
                let authors = v
                    .index_metadata
                    .get("authors")
                    .and_then(|s| s.as_str())
                    .unwrap_or("");

                serde_json::json!({
                    "@id": format!("{base}/proxy/{registry}/nuget/v3/registration5/{id}/{}.json", v.version),
                    "catalogEntry": {
                        "@id": format!("{base}/proxy/{registry}/nuget/v3/registration5/{id}/{}.json", v.version),
                        "@type": "PackageDetails",
                        "id": original_id,
                        "version": v.version,
                        "description": description,
                        "authors": authors,
                        "listed": true,
                        "published": published
                    },
                    "packageContent": pkg_content
                })
            })
            .collect();

        let lower = versions
            .first()
            .map(|v| v.version.as_str())
            .unwrap_or("");
        let upper = versions
            .last()
            .map(|v| v.version.as_str())
            .unwrap_or("");

        let response = serde_json::json!({
            "@id": format!("{base}/proxy/{registry}/nuget/v3/registration5/{id}/index.json"),
            "count": 1,
            "items": [{
                "@id": format!("{base}/proxy/{registry}/nuget/v3/registration5/{id}/page/{lower}/{upper}.json"),
                "lower": lower,
                "upper": upper,
                "count": items.len(),
                "items": items
            }]
        });

        return Ok(HttpResponse::Ok()
            .content_type("application/json")
            .json(response));
    }

    // Proxy or hybrid mode: forward to upstream registration.
    proxy_stream(
        svc,
        PackageId::new(&registry, &id, "__registration__"),
        identity,
        "releases:read",
        Some("application/json"),
    )
    .await
}

// ── Search ────────────────────────────────────────────────────────────────────

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
    req: HttpRequest,
    path: web::Path<String>,
    identity: AuthIdentity,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
) -> Result<impl Responder, AppError> {
    let registry = path.into_inner();
    require_nuget(&registry, &map)?;

    let query_string = req.query_string().to_owned();
    let q = extract_query_param(&query_string, "q").unwrap_or_default();
    let take: usize = extract_query_param(&query_string, "take")
        .and_then(|v| v.parse().ok())
        .unwrap_or(20)
        .min(100);

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
            .filter(|name| q.is_empty() || name.contains(&q))
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

        return Ok(HttpResponse::Ok().content_type("application/json").json(
            serde_json::json!({ "totalHits": matched.len(), "data": matched }),
        ));
    }

    // Proxy/hybrid: NuGet search is handled by the explore service (client.search_packages).
    // Return minimal empty response so dotnet CLI functions without error.
    let _ = &identity;
    Ok(HttpResponse::Ok()
        .content_type("application/json")
        .json(serde_json::json!({ "totalHits": 0, "data": [] })))
}

fn extract_query_param<'a>(query_string: &'a str, key: &str) -> Option<String> {
    query_string
        .split('&')
        .filter_map(|pair| {
            let mut parts = pair.splitn(2, '=');
            let k = parts.next()?;
            let v = parts.next().unwrap_or("");
            if k == key { Some(v.to_owned()) } else { None }
        })
        .next()
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
#[put("/proxy/{registry}/nuget/api/v2/package")]
pub async fn nuget_publish(
    req: HttpRequest,
    path: web::Path<String>,
    mut multipart: Multipart,
    identity: AuthIdentity,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
) -> Result<impl Responder, AppError> {
    let registry = path.into_inner();
    require_nuget(&registry, &map)?;
    require_local_mode(&registry, &mode_map)?;

    // dotnet nuget push and nuget.exe always send multipart/form-data.
    // Accept any field that looks like the package file.
    let mut nupkg_bytes_opt: Option<bytes::Bytes> = None;
    while let Some(field_result) = multipart.next().await {
        let mut field = field_result
            .map_err(|e| AppError::bad_request(format!("multipart error: {e}")))?;
        let field_name = field
            .content_disposition()
            .and_then(|cd| cd.get_name())
            .unwrap_or("")
            .to_owned();
        let mut buf = BytesMut::new();
        while let Some(chunk) = field.next().await {
            let chunk =
                chunk.map_err(|e| AppError::bad_request(format!("chunk error: {e}")))?;
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
    let version = if nuspec.version.is_empty() {
        return Err(AppError::unprocessable("nuspec missing <version>"));
    } else {
        nuspec.version.clone()
    };

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
) -> Result<impl Responder, AppError> {
    let (registry, id_raw, version) = path.into_inner();
    require_nuget(&registry, &map)?;
    require_local_mode(&registry, &mode_map)?;

    let id = id_raw.to_lowercase();

    local_svc
        .yank(&registry, &id, &version, &identity.0)
        .await
        .map_err(AppError::from)?;

    Ok(HttpResponse::NoContent().finish())
}
