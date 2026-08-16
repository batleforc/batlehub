use actix_web::{get, web, HttpRequest, HttpResponse, Responder};

use super::super::common::{registry_public_base, require_registry_type};
use crate::handlers::schemas::UpstreamDocument;
use crate::{error::AppError, extractors::AuthIdentity, RegistryMap};

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

    let index = serde_json::json!({
        "version": "3.0.0",
        "resources": [
            {
                "@id": format!("{base}/nuget/v3/registration5/"),
                "@type": "RegistrationsBaseUrl/3.6.0",
                "comment": "Base URL for NuGet package registration (metadata)"
            },
            {
                "@id": format!("{base}/nuget/v3/flat/"),
                "@type": "PackageBaseAddress/3.0.0",
                "comment": "Base URL for NuGet package content (flat container)"
            },
            {
                "@id": format!("{base}/nuget/api/v2/package"),
                "@type": "PackagePublish/2.0.0",
                "comment": "Publish .nupkg files"
            },
            {
                "@id": format!("{base}/nuget/v3/query"),
                "@type": "SearchQueryService",
                "comment": "NuGet package search"
            },
            {
                // The one `dotnet package search` actually selects. Measured
                // against dotnet 10.0.400: advertising only the bare type and
                // `/3.5.0` makes it report "The source does not have a Search
                // service!" — the endpoint was implemented in phase 6 and still
                // unreachable (RFC 0009 §12.4).
                "@id": format!("{base}/nuget/v3/query"),
                "@type": "SearchQueryService/3.0.0-beta",
                "comment": "The resource type dotnet's search resolver selects"
            },
            {
                "@id": format!("{base}/nuget/v3/query"),
                "@type": "SearchQueryService/3.5.0",
                "comment": "NuGet package search"
            },
            {
                "@id": format!("{base}/nuget/v3/autocomplete"),
                "@type": "SearchAutocompleteService",
                "comment": "Package id completion for `dotnet package search`"
            },
            {
                "@id": format!("{base}/nuget/v3/autocomplete"),
                "@type": "SearchAutocompleteService/3.0.0-beta",
                "comment": "Package id completion for `dotnet package search`"
            },
            {
                "@id": format!("{base}/nuget/v3/autocomplete"),
                "@type": "SearchAutocompleteService/3.5.0",
                "comment": "Package id completion for `dotnet package search`"
            },
            {
                "@id": format!("{base}/nuget/api/v2/symbolpackage"),
                "@type": "SymbolPackagePublish/4.9.0",
                "comment": "`.snupkg` symbol package publish"
            },
            {
                "@id": format!("{base}/nuget/v3/vulnerabilities/"),
                "@type": "VulnerabilitiesUrl/6.7.0",
                "comment": "NuGet vulnerability database"
            }
        ],
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
