use std::sync::Arc;

use actix_web::{get, post, web, HttpResponse, Responder};

use batlehub_core::services::ProxyService;

use crate::handlers::schemas::{ProtocolDocument, UpstreamDocument};
use crate::{
    error::AppError,
    extractors::AuthIdentity,
    handlers::proxy::{
        common::{collect_payload, require_registry_type},
        upstream::{cached_forward, Outbound},
    },
    RegistryMap, SumDbMap, VulnDbMap,
};

/// Resolves the vuln DB base URL for `registry`, or a 404 if the map builder
/// omitted it (its absence-means-disabled contract — see
/// `server/src/hot_config.rs::build_vuln_db_map`).
fn vuln_db_base_or_disabled(vuln_db: &VulnDbMap, registry: &str) -> Result<String, AppError> {
    vuln_db.url_for(registry).ok_or_else(|| {
        AppError::not_found(format!(
            "vuln DB proxy is disabled for registry '{registry}'"
        ))
    })
}

/// Proxy the Go Vulnerability Database index.
///
/// Clients set `GOVULNDB=<proxy-base>/<registry>` and `govulncheck` calls
/// `GET /v1/index.json` first to discover the available vulnerability IDs.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/v1/index.json",
    tag = "proxy/goproxy",
    params(
        ("registry" = String, Path, description = "Registry name (must be a goproxy registry)"),
    ),
    responses(
        (status = 200, description = "Vulnerability database index JSON", body = UpstreamDocument),
        (status = 404, description = "Registry not found or vuln DB disabled"),
        (status = 502, description = "Upstream vuln DB error"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/v1/index.json")]
pub async fn goproxy_vuln_index(
    path: web::Path<String>,
    _identity: AuthIdentity,
    map: web::Data<RegistryMap>,
    svc: web::Data<Arc<ProxyService>>,
    vuln_db: web::Data<VulnDbMap>,
) -> Result<impl Responder, AppError> {
    let registry = path.into_inner();
    require_registry_type(&registry, "goproxy", &map)?;

    let base = vuln_db_base_or_disabled(&vuln_db, &registry)?;

    let url = format!("{base}/v1/index.json");
    let key = format!("vulndb:{registry}:index");
    forward_get(&svc, &vuln_db.http, &registry, &key, &url).await
}

/// Proxy a single Go vulnerability record by its ID (e.g. `GO-2023-1234`).
#[utoipa::path(
    get,
    path = "/proxy/{registry}/v1/ID/{id}.json",
    tag = "proxy/goproxy",
    params(
        ("registry" = String, Path, description = "Registry name (must be a goproxy registry)"),
        ("id"       = String, Path, description = "Vulnerability ID, e.g. GO-2023-1234"),
    ),
    responses(
        (status = 200, description = "Vulnerability OSV record JSON", body = UpstreamDocument),
        (status = 400, description = "Invalid vulnerability ID"),
        (status = 404, description = "Registry not found, vuln DB disabled, or ID unknown"),
        (status = 502, description = "Upstream vuln DB error"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/v1/ID/{id}.json")]
pub async fn goproxy_vuln_entry(
    path: web::Path<(String, String)>,
    _identity: AuthIdentity,
    map: web::Data<RegistryMap>,
    svc: web::Data<Arc<ProxyService>>,
    vuln_db: web::Data<VulnDbMap>,
) -> Result<impl Responder, AppError> {
    let (registry, id) = path.into_inner();
    require_registry_type(&registry, "goproxy", &map)?;

    // Reject IDs that aren't safe alphanumeric-plus-dash identifiers.
    if !id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
    {
        return Err(AppError::bad_request(format!(
            "invalid vulnerability ID '{id}'"
        )));
    }

    let base = vuln_db_base_or_disabled(&vuln_db, &registry)?;

    let url = format!("{base}/v1/ID/{id}.json");
    let key = format!("vulndb:{registry}:id:{id}");
    forward_get(&svc, &vuln_db.http, &registry, &key, &url).await
}

/// Proxy a Go vulnerability database query.
///
/// `govulncheck` POSTs a JSON body describing the modules and versions to scan.
/// The response is a JSON array of matching OSV records.
#[utoipa::path(
    post,
    path = "/proxy/{registry}/v1/query",
    tag = "proxy/goproxy",
    params(
        ("registry" = String, Path, description = "Registry name (must be a goproxy registry)"),
    ),
    request_body(content_type = "application/json", description = "govulncheck query payload"),
    responses(
        (status = 200, description = "Matching vulnerability records", body = UpstreamDocument),
        (status = 404, description = "Registry not found or vuln DB disabled"),
        (status = 502, description = "Upstream vuln DB error"),
    ),
    security(("bearer_token" = [])),
)]
#[post("/proxy/{registry}/v1/query")]
pub async fn goproxy_vuln_query(
    path: web::Path<String>,
    payload: web::Payload,
    _identity: AuthIdentity,
    map: web::Data<RegistryMap>,
    svc: web::Data<Arc<ProxyService>>,
    vuln_db: web::Data<VulnDbMap>,
) -> Result<impl Responder, AppError> {
    let registry = path.into_inner();
    require_registry_type(&registry, "goproxy", &map)?;

    let body = collect_payload(payload).await?;

    let base = vuln_db_base_or_disabled(&vuln_db, &registry)?;

    // The POSTed module set selects the answer, so it belongs in the key —
    // hashed, because a whole dependency graph is far too long for one.
    let digest = {
        use sha2::{Digest, Sha256};
        hex::encode(Sha256::digest(&body))
    };
    let key = format!("vulndb:{registry}:query:{digest}");
    let parsed: serde_json::Value = serde_json::from_slice(&body)
        .map_err(|e| AppError::bad_request(format!("query body is not JSON: {e}")))?;

    let url = format!("{base}/v1/query");
    cached_forward(
        &svc,
        &vuln_db.http,
        &registry,
        &key,
        Outbound::post_json(url, parsed),
    )
    .await
}

// ── Shared helpers ────────────────────────────────────────────────────────────

/// Fetch a vuln DB document through the cache (RFC 0009 §4.2).
///
/// This used to be a bare `reqwest` GET, so `govulncheck` failed outright the
/// moment the vulnerability database was unreachable — including when we had
/// answered the identical request a minute earlier. A vulnerability check that
/// fails closed on upstream loss is the one most likely to be running in a
/// pipeline that must not stop.
async fn forward_get(
    svc: &Arc<ProxyService>,
    client: &reqwest::Client,
    registry: &str,
    cache_key: &str,
    url: &str,
) -> Result<HttpResponse, AppError> {
    cached_forward(svc, client, registry, cache_key, Outbound::get(url)).await
}

// ── The checksum database (RFC 0009 §7.4) ─────────────────────────────────────

/// Proxy the Go checksum database.
///
/// The `sumdb/` half of the `GOPROXY` protocol, and until this route existed it
/// was simply absent: `go mod download` through BatleHub still opened a direct
/// connection to `sum.golang.org` for every module it had not seen. So the
/// proxy had *moved* the egress rather than removed it, and an air-gapped
/// estate failed closed on a lookup it could not make.
///
/// **Cached, and that is the point.** A sumdb lookup answerable only while
/// `sum.golang.org` is reachable buys nothing; cached, the second build needs no
/// route off the site. Caching is sound because the log is signed — the
/// signature travels with the bytes, so a cached checksum record is exactly as
/// trustworthy as a live one, and this proxy neither parses nor rewrites it.
///
/// No filtering, for the same reason: a transparency log that has been edited is
/// a transparency log the client rejects.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/sumdb/{path}",
    tag = "proxy/goproxy",
    params(
        ("registry" = String, Path, description = "Registry name (must be a goproxy registry)"),
        ("path"     = String, Path, description = "Checksum database path, e.g. sum.golang.org/supported"),
    ),
    responses(
        (status = 200, description = "Checksum database response, verbatim", body = ProtocolDocument),
        (status = 404, description = "Registry not found or sumdb proxying disabled"),
        (status = 502, description = "Checksum database unreachable and nothing cached"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/sumdb/{path:.*}")]
pub async fn goproxy_sumdb(
    path: web::Path<(String, String)>,
    _identity: AuthIdentity,
    map: web::Data<RegistryMap>,
    svc: web::Data<Arc<ProxyService>>,
    sumdb: web::Data<SumDbMap>,
    client: web::Data<reqwest::Client>,
) -> Result<impl Responder, AppError> {
    let (registry, sub) = path.into_inner();
    require_registry_type(&registry, "goproxy", &map)?;

    let base = sumdb.url_for(&registry).ok_or_else(|| {
        AppError::not_found(format!(
            "checksum database proxying is disabled for registry '{registry}'"
        ))
    })?;

    // The path reaches an upstream URL, so `..` must not survive it. The go
    // command never sends one; something that does is not the go command.
    if sub.split('/').any(|seg| seg == ".." || seg == ".") {
        return Err(AppError::bad_request(
            "invalid checksum database path".to_owned(),
        ));
    }

    // `sub` already carries the sumdb host as its first segment
    // (`sum.golang.org/lookup/...`), which is how the protocol addresses a
    // database — so the configured base is the *proxy* root, not the log's.
    let url = format!("{}/{sub}", base.trim_end_matches('/'));
    let key = format!("sumdb:{registry}:{sub}");
    cached_forward(&svc, &client, &registry, &key, Outbound::get(url)).await
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    #[test]
    fn id_validation_accepts_valid_ids() {
        let valid = [
            "GO-2023-1234",
            "GO-2024-5678",
            "CVE-2023-12345",
            "GHSA-x.1-y2",
        ];
        for id in valid {
            let ok = id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.');
            assert!(ok, "{id} should be accepted");
        }
    }

    #[test]
    fn id_validation_rejects_path_traversal() {
        let bad = ["../etc/passwd", "GO/../../secret", "GO 2023", "GO\x002023"];
        for id in bad {
            let ok = id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.');
            assert!(!ok, "{id:?} should be rejected");
        }
    }
}
