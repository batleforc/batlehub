use actix_web::{get, web, HttpRequest, HttpResponse, Responder};

use super::super::common::require_registry_type;
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
    require_registry_type(&registry, "nuget", &map)?;

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
            },
            {
                "@id": format!("{base}/proxy/{registry}/nuget/v3/vulnerabilities/"),
                "@type": "VulnerabilitiesUrl/6.7.0",
                "comment": "NuGet vulnerability database"
            }
        ]
    });

    Ok(HttpResponse::Ok()
        .content_type("application/json")
        .json(index))
}
