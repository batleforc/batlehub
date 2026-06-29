use actix_web::{get, post, web, HttpResponse, Responder};

use crate::{
    error::AppError,
    extractors::AuthIdentity,
    handlers::proxy::common::{collect_payload, require_registry_type},
    RegistryMap, VulnDbMap,
};

const DEFAULT_VULN_DB: &str = "https://vuln.go.dev";

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
        (status = 200, description = "Vulnerability database index JSON"),
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
    vuln_db: web::Data<VulnDbMap>,
) -> Result<impl Responder, AppError> {
    let registry = path.into_inner();
    require_registry_type(&registry, "goproxy", &map)?;

    let base = vuln_db
        .url_for(&registry)
        .unwrap_or_else(|| DEFAULT_VULN_DB.to_owned());
    if base.is_empty() {
        return Err(AppError::not_found(format!(
            "vuln DB proxy is disabled for registry '{registry}'"
        )));
    }

    let url = format!("{base}/v1/index.json");
    forward_get(&vuln_db.http, &url).await
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
        (status = 200, description = "Vulnerability OSV record JSON"),
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

    let base = vuln_db
        .url_for(&registry)
        .unwrap_or_else(|| DEFAULT_VULN_DB.to_owned());
    if base.is_empty() {
        return Err(AppError::not_found(format!(
            "vuln DB proxy is disabled for registry '{registry}'"
        )));
    }

    let url = format!("{base}/v1/ID/{id}.json");
    forward_get(&vuln_db.http, &url).await
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
        (status = 200, description = "Matching vulnerability records"),
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
    vuln_db: web::Data<VulnDbMap>,
) -> Result<impl Responder, AppError> {
    let registry = path.into_inner();
    require_registry_type(&registry, "goproxy", &map)?;

    let body = collect_payload(payload).await?;

    let base = vuln_db
        .url_for(&registry)
        .unwrap_or_else(|| DEFAULT_VULN_DB.to_owned());
    if base.is_empty() {
        return Err(AppError::not_found(format!(
            "vuln DB proxy is disabled for registry '{registry}'"
        )));
    }

    let url = format!("{base}/v1/query");
    let resp = vuln_db
        .http
        .post(&url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(body.to_vec())
        .send()
        .await
        .map_err(|e| AppError::bad_gateway(format!("vuln DB upstream error: {e}")))?;

    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(AppError::not_found("vuln DB query endpoint not found"));
    }

    let status = resp.status();
    if !status.is_success() {
        return Err(AppError::bad_gateway(format!(
            "vuln DB returned {status}"
        )));
    }

    let bytes = resp
        .bytes()
        .await
        .map_err(|e| AppError::bad_gateway(format!("reading vuln DB response: {e}")))?;

    Ok(HttpResponse::Ok()
        .content_type("application/json")
        .body(bytes))
}

// ── Shared helpers ────────────────────────────────────────────────────────────

async fn forward_get(client: &reqwest::Client, url: &str) -> Result<HttpResponse, AppError> {
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| AppError::bad_gateway(format!("vuln DB upstream error: {e}")))?;

    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(AppError::not_found("not found in vuln DB"));
    }

    let status = resp.status();
    if !status.is_success() {
        return Err(AppError::bad_gateway(format!(
            "vuln DB returned {status}"
        )));
    }

    let bytes = resp
        .bytes()
        .await
        .map_err(|e| AppError::bad_gateway(format!("reading vuln DB response: {e}")))?;

    Ok(HttpResponse::Ok()
        .content_type("application/json")
        .body(bytes))
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_validation_accepts_valid_ids() {
        let valid = ["GO-2023-1234", "GO-2024-5678", "CVE-2023-12345", "GHSA-x.1-y2"];
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
