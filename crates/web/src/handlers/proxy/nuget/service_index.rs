use actix_web::{get, web, HttpRequest, HttpResponse, Responder};

use super::super::common::{registry_public_base, require_registry_type};
use crate::handlers::schemas::UpstreamDocument;
use crate::{error::AppError, extractors::AuthIdentity, RegistryMap};

/// Every resource the service index advertises: `(path, @type, comment)`.
///
/// A table rather than eleven object literals that differ in two strings.
/// Several `@type`s deliberately point at the same endpoint: `dotnet` selects a
/// resource by type, and advertising only the bare name and `/3.5.0` made an
/// implemented search endpoint unreachable — "The source does not have a Search
/// service!" against dotnet 10.0.400 (RFC 0009 §12.4). Reading the versions
/// down one column is the point.
const RESOURCES: &[(&str, &str, &str)] = &[
    (
        "/nuget/v3/registration5/",
        "RegistrationsBaseUrl/3.6.0",
        "Base URL for NuGet package registration (metadata)",
    ),
    (
        "/nuget/v3/flat/",
        "PackageBaseAddress/3.0.0",
        "Base URL for NuGet package content (flat container)",
    ),
    (
        "/nuget/api/v2/package",
        "PackagePublish/2.0.0",
        "Publish .nupkg files",
    ),
    (
        "/nuget/v3/query",
        "SearchQueryService",
        "NuGet package search",
    ),
    (
        "/nuget/v3/query",
        "SearchQueryService/3.0.0-beta",
        "The resource type dotnet's search resolver selects",
    ),
    (
        "/nuget/v3/query",
        "SearchQueryService/3.5.0",
        "NuGet package search",
    ),
    (
        "/nuget/v3/autocomplete",
        "SearchAutocompleteService",
        "Package id completion for `dotnet package search`",
    ),
    (
        "/nuget/v3/autocomplete",
        "SearchAutocompleteService/3.0.0-beta",
        "Package id completion for `dotnet package search`",
    ),
    (
        "/nuget/v3/autocomplete",
        "SearchAutocompleteService/3.5.0",
        "Package id completion for `dotnet package search`",
    ),
    (
        "/nuget/api/v2/symbolpackage",
        "SymbolPackagePublish/4.9.0",
        "`.snupkg` symbol package publish",
    ),
    (
        "/nuget/v3/vulnerabilities/",
        "VulnerabilitiesUrl/6.7.0",
        "NuGet vulnerability database",
    ),
];

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
        (status = 200, description = "NuGet v3 service index", body = UpstreamDocument),
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
    require_registry_type(&registry, "nuget", &map)?;

    // Built from the incoming request so the service index works behind reverse
    // proxies, on a registry host, and in local dev alike.
    let base = registry_public_base(&req, &registry);

    let _ = &identity; // auth enforced by middleware; referenced to satisfy extractor

    let resources: Vec<serde_json::Value> = RESOURCES
        .iter()
        .map(|(path, kind, comment)| {
            serde_json::json!({ "@id": format!("{base}{path}"), "@type": kind, "comment": comment })
        })
        .collect();

    let index = serde_json::json!({
        "version": "3.0.0",
        "resources": resources,
        // Without these a client falls back to nuget.org links, which for a
        // private registry publishes an internal package name to a public site
        // every time someone runs a command that prints one (RFC 0009 §7.6).
        "ReportAbuseUriTemplate": format!("{base}/packages/{{id}}/{{version}}"),
        "PackageDetailsUriTemplate": format!("{base}/packages/{{id}}/{{version}}"),
    });

    Ok(HttpResponse::Ok()
        .content_type("application/json")
        .json(index))
}
