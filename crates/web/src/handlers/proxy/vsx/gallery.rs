//! `POST …/vscode/gallery/extensionquery` — the endpoint an editor lives on.
//!
//! Search, install and update all arrive here. The response is composed from
//! [`super::source`] entries rather than forwarded, so every URL in it points
//! back at this proxy and every blocked version is already gone.

use std::sync::Arc;

use actix_web::{post, web, HttpRequest, HttpResponse, Responder};

use batlehub_core::services::{LocalRegistryService, ProxyService};

use super::protocol::{ExtensionQueryRequest, GalleryQuery};
use super::render::{query_response_json, GalleryUrls};
use super::{require_vsx, source, vsx_kind};
use crate::handlers::proxy::common::registry_public_base;
use crate::handlers::schemas::UpstreamDocument;
use crate::{error::AppError, extractors::AuthIdentity, RegistryMap, RegistryModeMap};

/// Query the extension gallery.
///
/// The body is the VS Code Marketplace `extensionquery` shape; see
/// [`ExtensionQueryRequest`]. Every field is optional, so a hand-written client
/// gets results rather than a `400`.
#[utoipa::path(
    post,
    path = "/proxy/{registry}/vscode/gallery/extensionquery",
    tag = "proxy/openvsx",
    params(("registry" = String, Path, description = "Registry name")),
    request_body = ExtensionQueryRequest,
    responses(
        (status = 200, description = "Gallery query results", body = UpstreamDocument),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Unknown registry or wrong type"),
    ),
    security(("bearer_token" = [])),
)]
#[allow(clippy::too_many_arguments)]
#[post("/proxy/{registry}/vscode/gallery/extensionquery")]
pub async fn vsx_extension_query(
    req: HttpRequest,
    path: web::Path<String>,
    body: web::Json<ExtensionQueryRequest>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
) -> Result<impl Responder, AppError> {
    let registry = path.into_inner();
    require_vsx(&registry, &map)?;

    let kind = vsx_kind(&registry, &map);
    let mode = mode_map.get(&registry);
    let query = GalleryQuery::from_request(&body);

    let (entries, total) =
        source::search_entries(&svc, &local_svc, mode, &registry, kind, &query, &identity).await?;

    let urls = GalleryUrls::new(&registry_public_base(&req, &registry));
    Ok(HttpResponse::Ok()
        .content_type("application/json")
        .json(query_response_json(&entries, total, &urls, &query)))
}
