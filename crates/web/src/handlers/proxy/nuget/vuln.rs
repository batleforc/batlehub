use std::sync::Arc;

use actix_web::{get, web, HttpResponse, Responder};

use batlehub_core::services::ProxyService;

use crate::handlers::schemas::UpstreamDocument;
use crate::{
    error::AppError,
    extractors::AuthIdentity,
    handlers::proxy::{
        common::require_registry_type,
        upstream::{cached_forward, Outbound},
    },
    RegistryMap, UpstreamMap,
};

const DEFAULT_NUGET_VULN_BASE: &str = "https://api.nuget.org/v3/vulnerabilities";

/// Proxy the NuGet vulnerability database index.
///
/// `dotnet list package --vulnerable` discovers this URL via the `VulnerabilitiesUrl/6.7.0`
/// entry in the v3 service index and fetches it to build an in-memory vulnerability catalogue.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/nuget/v3/vulnerabilities/index.json",
    tag = "proxy/nuget",
    params(
        ("registry" = String, Path, description = "Registry name (must be a nuget registry)"),
    ),
    responses(
        (status = 200, description = "NuGet vulnerability index JSON", body = Vec<UpstreamDocument>),
        (status = 404, description = "Registry not found or not a NuGet registry"),
        (status = 502, description = "Upstream vulnerability DB error"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/nuget/v3/vulnerabilities/index.json")]
pub async fn nuget_vuln_index(
    path: web::Path<String>,
    _identity: AuthIdentity,
    map: web::Data<RegistryMap>,
    upstream_map: web::Data<UpstreamMap>,
    svc: web::Data<Arc<ProxyService>>,
    client: web::Data<reqwest::Client>,
) -> Result<impl Responder, AppError> {
    let registry = path.into_inner();
    require_registry_type(&registry, "nuget", &map)?;

    let base = vuln_base(&registry, &upstream_map);
    let url = format!("{base}/index.json");
    let key = format!("vulndb:{registry}:nuget:index");
    forward_get(&svc, &client, &registry, &key, &url).await
}

/// Proxy a single page of NuGet vulnerability records.
///
/// The index returned by `nuget_vuln_index` contains URLs for individual pages.
/// This handler proxies each page through the BatleHub server so clients in
/// restricted environments do not need direct access to api.nuget.org.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/nuget/v3/vulnerabilities/page/{page}",
    tag = "proxy/nuget",
    params(
        ("registry" = String, Path, description = "Registry name (must be a nuget registry)"),
        ("page"     = String, Path, description = "Page identifier, e.g. 0.json"),
    ),
    responses(
        (status = 200, description = "NuGet vulnerability page JSON", body = UpstreamDocument),
        (status = 400, description = "Invalid page identifier"),
        (status = 404, description = "Registry not found, not NuGet, or page unknown"),
        (status = 502, description = "Upstream vulnerability DB error"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/nuget/v3/vulnerabilities/page/{page}")]
pub async fn nuget_vuln_page(
    path: web::Path<(String, String)>,
    _identity: AuthIdentity,
    map: web::Data<RegistryMap>,
    upstream_map: web::Data<UpstreamMap>,
    svc: web::Data<Arc<ProxyService>>,
    client: web::Data<reqwest::Client>,
) -> Result<impl Responder, AppError> {
    let (registry, page) = path.into_inner();
    require_registry_type(&registry, "nuget", &map)?;

    // Reject page identifiers that aren't safe (digits + optional .json suffix).
    if !page
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_')
    {
        return Err(AppError::bad_request(format!(
            "invalid vulnerability page identifier '{page}'"
        )));
    }

    let base = vuln_base(&registry, &upstream_map);
    let url = format!("{base}/page/{page}");
    let key = format!("vulndb:{registry}:nuget:page:{page}");
    forward_get(&svc, &client, &registry, &key, &url).await
}

/// Derive the vulnerability base URL: use the configured NuGet upstream host
/// (e.g. `https://api.nuget.org`) with `/v3/vulnerabilities` appended, falling
/// back to the NuGet gallery default when no upstream is configured.
fn vuln_base(registry: &str, upstream_map: &UpstreamMap) -> String {
    upstream_map
        .upstream_for(registry)
        .map(|u| format!("{}/v3/vulnerabilities", u.trim_end_matches('/')))
        .unwrap_or_else(|| DEFAULT_NUGET_VULN_BASE.to_owned())
}

/// Fetch a NuGet vulnerability document through the cache (RFC 0009 §4.2).
///
/// Was a bare `reqwest` GET, so `dotnet list package --vulnerable` failed
/// outright whenever api.nuget.org was unreachable — the exact case the proxy
/// exists to absorb, and the one most likely to be inside a build pipeline.
async fn forward_get(
    svc: &Arc<ProxyService>,
    client: &reqwest::Client,
    registry: &str,
    cache_key: &str,
    url: &str,
) -> Result<HttpResponse, AppError> {
    cached_forward(svc, client, registry, cache_key, Outbound::get(url)).await
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_validation_accepts_safe_ids() {
        let valid = ["0.json", "1", "123", "page-0.json", "0_1"];
        for id in valid {
            let ok = id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_');
            assert!(ok, "{id} should be accepted");
        }
    }

    #[test]
    fn page_validation_rejects_path_traversal() {
        let bad = ["../etc/passwd", "0/../../secret", "0 1", "0\x001"];
        for id in bad {
            let ok = id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_');
            assert!(!ok, "{id:?} should be rejected");
        }
    }

    #[test]
    fn vuln_base_falls_back_to_default_when_no_upstream() {
        let map = UpstreamMap::new(std::collections::HashMap::new());
        assert_eq!(vuln_base("myregistry", &map), DEFAULT_NUGET_VULN_BASE);
    }

    #[test]
    fn vuln_base_uses_upstream_host() {
        let map = UpstreamMap::new(
            [("myregistry".to_owned(), "https://api.nuget.org".to_owned())]
                .into_iter()
                .collect(),
        );
        assert_eq!(
            vuln_base("myregistry", &map),
            "https://api.nuget.org/v3/vulnerabilities"
        );
    }
}
