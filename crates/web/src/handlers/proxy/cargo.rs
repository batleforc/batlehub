use std::sync::Arc;

use actix_web::{delete, get, put, web, HttpRequest, HttpResponse, Responder};
use bytes::{Buf, Bytes};
use sha2::{Digest, Sha256};

use batlehub_config::schema::RegistryMode;
use batlehub_core::{
    entities::{CargoDep, CargoIndexEntry, PackageId},
    error::CoreError,
    services::{LocalRegistryService, ProxyService, PublishRequest},
};

use super::common::{
    append_signature_headers, collect_payload, extract_signature_headers, proxy_stream,
    require_local_mode,
};
use crate::{
    error::AppError, extractors::AuthIdentity, services::NotificationService, CargoIndexMap,
    RegistryMap, RegistryModeMap,
};
use batlehub_core::entities::NotificationEventType;

// ── Sparse index proxy ────────────────────────────────────────────────────────

/// HTTP client + upstream index URL for one cargo sparse index.
#[derive(Clone)]
pub struct CargoIndexProxy {
    pub http: reqwest::Client,
    /// Base URL of the upstream sparse index, e.g. `https://index.crates.io`.
    pub index_url: String,
}

fn require_cargo(registry: &str, map: &RegistryMap) -> Result<(), AppError> {
    match map.type_of(registry).as_deref() {
        Some("cargo") => Ok(()),
        Some(_) => Err(AppError::not_found(format!(
            "registry '{registry}' is not a cargo registry"
        ))),
        None => Err(AppError::not_found(format!(
            "unknown registry '{registry}'"
        ))),
    }
}

/// Cargo sparse registry `config.json`.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/registry/config.json",
    tag = "proxy/cargo",
    params(("registry" = String, Path, description = "Registry name")),
    responses(
        (status = 200, description = "Sparse registry configuration"),
        (status = 404, description = "No cargo registry configured"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/registry/config.json")]
pub async fn cargo_registry_config(
    path: web::Path<String>,
    indexes: web::Data<CargoIndexMap>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
    req: HttpRequest,
) -> HttpResponse {
    let registry = path.into_inner();
    if !map.is_type(&registry, "cargo") {
        return HttpResponse::NotFound().body(format!("unknown cargo registry '{registry}'"));
    }

    let mode = mode_map.get(&registry);

    // Proxy and Hybrid modes require a configured upstream index.
    if matches!(mode, RegistryMode::Proxy | RegistryMode::Hybrid)
        && indexes.get(&registry).is_none()
    {
        return HttpResponse::NotFound().body("no cargo index configured");
    }

    let (scheme, host) = {
        let info = req.connection_info();
        (info.scheme().to_owned(), info.host().to_owned())
    };
    let dl = format!("{scheme}://{host}/proxy/{registry}/{{crate}}/{{version}}/download");
    let mut resp = serde_json::json!({ "dl": dl });

    // Expose the publish API URL for local and hybrid registries.
    if matches!(mode, RegistryMode::Local | RegistryMode::Hybrid) {
        resp["api"] = serde_json::Value::String(format!("{scheme}://{host}/proxy/{registry}"));
    }

    HttpResponse::Ok()
        .content_type("application/json")
        .json(resp)
}

/// Cargo sparse registry index entries.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/registry/{path}",
    tag = "proxy/cargo",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("path"     = String, Path, description = "Crate index path, e.g. se/rd/serde"),
    ),
    responses(
        (status = 200, description = "Sparse index entry (newline-delimited JSON)"),
        (status = 404, description = "Crate not found in index"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/registry/{path:.*}")]
pub async fn cargo_registry_index(
    path: web::Path<(String, String)>,
    indexes: web::Data<CargoIndexMap>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    identity: AuthIdentity,
) -> HttpResponse {
    let (registry, index_path) = path.into_inner();
    if !map.is_type(&registry, "cargo") {
        return HttpResponse::NotFound().body(format!("unknown cargo registry '{registry}'"));
    }

    let mode = mode_map.get(&registry);

    match mode {
        RegistryMode::Local => {
            serve_local_index(&local_svc, &registry, &index_path, &identity).await
        }
        RegistryMode::Hybrid => {
            let local = serve_local_index(&local_svc, &registry, &index_path, &identity).await;
            if local.status() != actix_web::http::StatusCode::NOT_FOUND {
                return local;
            }
            proxy_upstream_index(&indexes, &registry, &index_path).await
        }
        RegistryMode::Proxy => proxy_upstream_index(&indexes, &registry, &index_path).await,
    }
}

async fn serve_local_index(
    local_svc: &LocalRegistryService,
    registry: &str,
    index_path: &str,
    identity: &batlehub_core::entities::Identity,
) -> HttpResponse {
    // The Cargo sparse index path format is "{prefix1}/{prefix2}/{name}" for
    // names ≥ 3 chars, or "{len}/{name}" for 1–2 char names.
    // `splitn(3, '/')` captures everything after the prefix segments as the
    // final component, which preserves slashes in package names (e.g. a
    // name like "scope/pkg" decoded from "scope%2Fpkg" in the URL remains
    // intact as "scope/pkg" rather than being truncated to "pkg").
    let name = index_path.splitn(3, '/').last().unwrap_or(index_path);
    match local_svc.get_index(registry, name, identity).await {
        Ok(content) => HttpResponse::Ok()
            .content_type("text/plain; charset=utf-8")
            .body(content),
        Err(CoreError::NotFound(_)) => {
            HttpResponse::NotFound().body(format!("crate '{name}' not found in local registry"))
        }
        Err(CoreError::AccessDenied(msg)) => HttpResponse::Forbidden().body(msg),
        Err(e) => {
            tracing::error!(error = %e, "local index lookup failed");
            HttpResponse::InternalServerError().body(e.to_string())
        }
    }
}

async fn proxy_upstream_index(
    indexes: &CargoIndexMap,
    registry: &str,
    index_path: &str,
) -> HttpResponse {
    let Some(index) = indexes.get(registry) else {
        return HttpResponse::NotFound().body("no cargo registry configured");
    };
    let url = format!("{}/{}", index.index_url.trim_end_matches('/'), index_path);
    tracing::debug!(url = %url, "fetching cargo sparse index entry");
    let resp = match index.http.get(&url).send().await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(url = %url, error = %e, "cargo index fetch failed");
            return HttpResponse::BadGateway().body(e.to_string());
        }
    };
    let status = actix_web::http::StatusCode::from_u16(resp.status().as_u16())
        .unwrap_or(actix_web::http::StatusCode::INTERNAL_SERVER_ERROR);
    match resp.bytes().await {
        Ok(bytes) => HttpResponse::build(status)
            .content_type("text/plain; charset=utf-8")
            .body(bytes),
        Err(e) => HttpResponse::BadGateway().body(e.to_string()),
    }
}

/// Download a `.crate` file for a specific version.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/{name}/{version}/download",
    tag = "proxy/cargo",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("name"     = String, Path, description = "Crate name"),
        ("version"  = String, Path, description = "Version"),
    ),
    responses(
        (status = 200, description = ".crate file stream"),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Unknown registry"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/{name}/{version}/download")]
pub async fn download_crate(
    path: web::Path<(String, String, String)>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
) -> Result<impl Responder, AppError> {
    let (registry, name, version) = path.into_inner();
    require_cargo(&registry, &map)?;

    let mode = mode_map.get(&registry);

    if matches!(mode, RegistryMode::Local) {
        local_svc
            .check_prerelease_access(&registry, &version, &identity)
            .await
            .map_err(AppError::from)?;
        let bytes = local_svc
            .get_artifact(&registry, &name, &version, &identity)
            .await
            .map_err(AppError::from)?;
        let mut resp = HttpResponse::Ok();
        resp.content_type("application/octet-stream");
        append_signature_headers(&mut resp, &local_svc, &registry, &name, &version).await;
        return Ok(resp.body(bytes));
    }

    if matches!(mode, RegistryMode::Hybrid) {
        // Gate must be enforced before falling through to upstream: a non-member
        // must not receive a pre-release artifact from the upstream registry.
        local_svc
            .check_prerelease_access(&registry, &version, &identity)
            .await
            .map_err(AppError::from)?;
        match local_svc
            .get_artifact(&registry, &name, &version, &identity)
            .await
        {
            Ok(bytes) => {
                let mut resp = HttpResponse::Ok();
                resp.content_type("application/octet-stream");
                append_signature_headers(&mut resp, &local_svc, &registry, &name, &version).await;
                return Ok(resp.body(bytes));
            }
            Err(CoreError::NotFound(_)) => {} // not found locally; fall through to upstream
            Err(e) => return Err(AppError::from(e)),
        }
    }

    let pkg = PackageId::new(&registry, &name, &version).with_artifact("dl");
    proxy_stream(svc, pkg, identity, "source:read", None).await
}

// ── Publish API ───────────────────────────────────────────────────────────────

/// Publish a new crate version (`cargo publish`).
#[utoipa::path(
    put,
    path = "/proxy/{registry}/api/v1/crates/new",
    tag = "proxy/cargo",
    params(("registry" = String, Path, description = "Registry name")),
    request_body(content_type = "application/octet-stream", description = "Cargo publish binary payload (length-prefixed metadata + .crate bytes)"),
    responses(
        (status = 200, description = "Crate published successfully"),
        (status = 400, description = "Invalid publish payload"),
        (status = 403, description = "Access denied"),
        (status = 409, description = "Version already published"),
    ),
    security(("bearer_token" = [])),
)]
#[allow(clippy::too_many_arguments)]
#[put("/proxy/{registry}/api/v1/crates/new")]
pub async fn cargo_publish(
    req: actix_web::HttpRequest,
    path: web::Path<String>,
    payload: web::Payload,
    identity: AuthIdentity,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    notification_svc: web::Data<Option<Arc<NotificationService>>>,
) -> Result<impl Responder, AppError> {
    let registry = path.into_inner();
    require_cargo(&registry, &map)?;
    require_local_mode(&registry, &mode_map)?;

    let body = collect_payload(payload).await?;

    let (meta_json, crate_bytes) = parse_publish_body(body).map_err(AppError::bad_request)?;

    let name = meta_json
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::bad_request("missing 'name' in publish metadata"))?
        .to_owned();
    let version = meta_json
        .get("vers")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::bad_request("missing 'vers' in publish metadata"))?
        .to_owned();

    let checksum = hex::encode(Sha256::digest(&crate_bytes));

    let mut entry =
        metadata_to_index_entry(&meta_json, &checksum).map_err(AppError::bad_request)?;

    // Cargo-specific: validate caller-declared checksum against computed value.
    if !entry.cksum.is_empty() && entry.cksum != checksum {
        return Err(AppError::bad_request(format!(
            "checksum mismatch: declared {} but computed {}",
            entry.cksum, checksum
        )));
    }
    entry.cksum = checksum.clone();

    let index_metadata =
        serde_json::to_value(&entry).map_err(|e| AppError::bad_request(e.to_string()))?;

    let (signature_bytes, signature_type) = extract_signature_headers(&req);
    let actor = identity.0.user_id.clone().unwrap_or_default();

    let quota = local_svc
        .publish(PublishRequest {
            registry: registry.clone(),
            name: name.clone(),
            version: version.clone(),
            artifact: crate_bytes,
            checksum,
            index_metadata,
            publisher: identity.0.clone(),
            signature_bytes,
            signature_type,
        })
        .await
        .map_err(AppError::from)?;

    super::common::dispatch_notification(
        &notification_svc,
        NotificationEventType::PackagePublished,
        &registry,
        &name,
        Some(version),
        &actor,
    );

    let mut resp = HttpResponse::Ok();
    for (k, v) in quota.headers() {
        resp.insert_header((k, v));
    }
    Ok(resp.json(serde_json::json!({
        "warnings": {
            "invalid_categories": [],
            "invalid_badges": [],
            "other": []
        }
    })))
}

/// Yank a published crate version.
#[utoipa::path(
    delete,
    path = "/proxy/{registry}/api/v1/crates/{name}/{version}/yank",
    tag = "proxy/cargo",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("name"     = String, Path, description = "Crate name"),
        ("version"  = String, Path, description = "Version"),
    ),
    responses(
        (status = 200, description = "Yanked"),
        (status = 403, description = "Access denied"),
    ),
    security(("bearer_token" = [])),
)]
#[delete("/proxy/{registry}/api/v1/crates/{name}/{version}/yank")]
pub async fn cargo_yank(
    path: web::Path<(String, String, String)>,
    identity: AuthIdentity,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    notification_svc: web::Data<Option<Arc<NotificationService>>>,
) -> Result<impl Responder, AppError> {
    let (registry, name, version) = path.into_inner();
    require_cargo(&registry, &map)?;
    require_local_mode(&registry, &mode_map)?;
    let actor = identity.0.user_id.clone().unwrap_or_default();
    local_svc
        .yank(&registry, &name, &version, &identity.0)
        .await
        .map_err(AppError::from)?;
    super::common::dispatch_notification(
        &notification_svc,
        NotificationEventType::PackageYanked,
        &registry,
        &name,
        Some(version),
        &actor,
    );
    Ok(HttpResponse::Ok().json(serde_json::json!({ "ok": true })))
}

/// Unyank a previously yanked crate version.
#[utoipa::path(
    put,
    path = "/proxy/{registry}/api/v1/crates/{name}/{version}/unyank",
    tag = "proxy/cargo",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("name"     = String, Path, description = "Crate name"),
        ("version"  = String, Path, description = "Version"),
    ),
    responses(
        (status = 200, description = "Unyanked"),
        (status = 403, description = "Access denied"),
    ),
    security(("bearer_token" = [])),
)]
#[put("/proxy/{registry}/api/v1/crates/{name}/{version}/unyank")]
pub async fn cargo_unyank(
    path: web::Path<(String, String, String)>,
    identity: AuthIdentity,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    notification_svc: web::Data<Option<Arc<NotificationService>>>,
) -> Result<impl Responder, AppError> {
    let (registry, name, version) = path.into_inner();
    require_cargo(&registry, &map)?;
    require_local_mode(&registry, &mode_map)?;
    let actor = identity.0.user_id.clone().unwrap_or_default();
    local_svc
        .unyank(&registry, &name, &version, &identity.0)
        .await
        .map_err(AppError::from)?;
    super::common::dispatch_notification(
        &notification_svc,
        NotificationEventType::PackageUnyanked,
        &registry,
        &name,
        Some(version),
        &actor,
    );
    Ok(HttpResponse::Ok().json(serde_json::json!({ "ok": true })))
}

/// List owners of a crate (`cargo owner --list`).
#[utoipa::path(
    get,
    path = "/proxy/{registry}/api/v1/crates/{name}/owners",
    tag = "proxy/cargo",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("name"     = String, Path, description = "Crate name"),
    ),
    responses(
        (status = 200, description = "Owner list"),
        (status = 404, description = "Crate not found"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/api/v1/crates/{name}/owners")]
pub async fn cargo_owners(
    path: web::Path<(String, String)>,
    map: web::Data<RegistryMap>,
    local_svc: web::Data<Arc<LocalRegistryService>>,
) -> Result<impl Responder, AppError> {
    let (registry, name) = path.into_inner();
    require_cargo(&registry, &map)?;

    if let Some(ref ownership) = local_svc.ownership {
        let entries = ownership
            .list_owners(&registry, &name)
            .await
            .map_err(AppError::from)?;
        let users: Vec<_> = entries
            .into_iter()
            .enumerate()
            .map(|(i, e)| serde_json::json!({ "id": i + 1, "login": e.principal_id, "name": e.principal_id }))
            .collect();
        return Ok(HttpResponse::Ok().json(serde_json::json!({ "users": users })));
    }

    // Fallback: derive from first-published version.
    let versions = local_svc
        .backend
        .get_versions(&registry, &name)
        .await
        .map_err(AppError::from)?;
    if versions.is_empty() {
        return Err(AppError::not_found(format!("crate '{name}' not found")));
    }
    let publisher = versions[0]
        .published_by
        .clone()
        .unwrap_or_else(|| "unknown".to_owned());
    Ok(HttpResponse::Ok().json(serde_json::json!({
        "users": [{ "id": 1, "login": publisher, "name": publisher }]
    })))
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Parse the Cargo publish wire format:
/// `[4B LE u32 meta_len][JSON][4B LE u32 crate_len][.crate bytes]`
fn parse_publish_body(mut body: Bytes) -> Result<(serde_json::Value, Bytes), String> {
    if body.remaining() < 4 {
        return Err("publish body too short (missing metadata length)".into());
    }
    let meta_len = body.get_u32_le() as usize;
    if body.remaining() < meta_len {
        return Err(format!(
            "metadata length {meta_len} exceeds remaining body ({} bytes)",
            body.remaining()
        ));
    }
    let meta_bytes = body.copy_to_bytes(meta_len);
    let meta_json: serde_json::Value = serde_json::from_slice(&meta_bytes)
        .map_err(|e| format!("invalid publish metadata JSON: {e}"))?;

    if body.remaining() < 4 {
        return Err("publish body too short (missing crate length)".into());
    }
    let crate_len = body.get_u32_le() as usize;
    if body.remaining() < crate_len {
        return Err(format!(
            "crate length {crate_len} exceeds remaining body ({} bytes)",
            body.remaining()
        ));
    }
    let crate_bytes = body.copy_to_bytes(crate_len);

    Ok((meta_json, crate_bytes))
}

/// Convert a Cargo publish metadata JSON object into a sparse index `CargoIndexEntry`.
/// The publish format uses `version_req`; the index format uses `req`.
fn metadata_to_index_entry(
    meta: &serde_json::Value,
    checksum: &str,
) -> Result<CargoIndexEntry, String> {
    let name = meta["name"].as_str().ok_or("missing 'name'")?.to_owned();
    let vers = meta["vers"].as_str().ok_or("missing 'vers'")?.to_owned();

    let deps = meta
        .get("deps")
        .and_then(|d| d.as_array())
        .map(|arr| {
            arr.iter()
                .map(|dep| {
                    Ok(CargoDep {
                        name: dep["name"].as_str().ok_or("dep missing 'name'")?.to_owned(),
                        req: dep
                            .get("version_req")
                            .and_then(|v| v.as_str())
                            .or_else(|| dep.get("req").and_then(|v| v.as_str()))
                            .ok_or("dep missing 'version_req'")?
                            .to_owned(),
                        features: dep
                            .get("features")
                            .and_then(|f| f.as_array())
                            .map(|arr| {
                                arr.iter()
                                    .filter_map(|v| v.as_str().map(str::to_owned))
                                    .collect()
                            })
                            .unwrap_or_default(),
                        optional: dep
                            .get("optional")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false),
                        default_features: dep
                            .get("default_features")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(true),
                        target: dep
                            .get("target")
                            .and_then(|v| v.as_str())
                            .map(str::to_owned),
                        kind: dep
                            .get("kind")
                            .and_then(|v| v.as_str())
                            .unwrap_or("normal")
                            .to_owned(),
                        registry: dep
                            .get("registry")
                            .and_then(|v| v.as_str())
                            .map(str::to_owned),
                        explicit_name_in_toml: dep
                            .get("explicit_name_in_toml")
                            .and_then(|v| v.as_str())
                            .map(str::to_owned),
                    })
                })
                .collect::<Result<Vec<_>, &str>>()
        })
        .transpose()
        .map_err(|e: &str| e.to_owned())?
        .unwrap_or_default();

    let features = meta
        .get("features")
        .cloned()
        .unwrap_or(serde_json::Value::Object(Default::default()));

    let links = meta
        .get("links")
        .and_then(|v| v.as_str())
        .map(str::to_owned);

    let rust_version = meta
        .get("rust_version")
        .and_then(|v| v.as_str())
        .map(str::to_owned);

    Ok(CargoIndexEntry {
        name,
        vers,
        deps,
        cksum: checksum.to_owned(),
        features,
        features2: None,
        yanked: false,
        links,
        rust_version,
        v: None,
    })
}
