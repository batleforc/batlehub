use std::sync::Arc;

use actix_web::{get, web, HttpRequest, HttpResponse, Responder};

use batlehub_config::schema::RegistryMode;
use batlehub_core::{
    entities::PackageId,
    services::{LocalRegistryService, ProxyService},
};

use super::super::common::{proxy_stream, require_registry_type};
use crate::{error::AppError, extractors::AuthIdentity, RegistryMap, RegistryModeMap};

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
    require_registry_type(&registry, "nuget", &map)?;

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

        let lower = versions.first().map(|v| v.version.as_str()).unwrap_or("");
        let upper = versions.last().map(|v| v.version.as_str()).unwrap_or("");

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
        batlehub_core::rules::resource_type::RELEASES_READ,
        Some("application/json"),
    )
    .await
}
