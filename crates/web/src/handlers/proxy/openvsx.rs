use std::sync::Arc;

use actix_web::{Responder, get, web};

use batlehub_core::{
    entities::PackageId,
    services::ProxyService,
};

use crate::{RegistryMap, error::AppError, extractors::AuthIdentity};
use super::common::proxy_stream;

pub fn require_openvsx(registry: &str, map: &RegistryMap) -> Result<(), AppError> {
    match map.type_of(registry) {
        Some("openvsx") | Some("vscode-marketplace") => Ok(()),
        Some(_) => Err(AppError::not_found(format!(
            "registry '{registry}' is not an openvsx or vscode-marketplace registry"
        ))),
        None => Err(AppError::not_found(format!("unknown registry '{registry}'"))),
    }
}

/// Download a VS Code extension VSIX package.
///
/// Extension IDs follow the `{publisher}.{name}` convention (e.g. `ms-python.python`).
#[utoipa::path(
    get,
    path = "/proxy/{registry}/{extension_id}/{version}/vsix",
    tag = "proxy/openvsx",
    params(
        ("registry"     = String, Path, description = "Registry name"),
        ("extension_id" = String, Path, description = "Extension ID in publisher.name format"),
        ("version"      = String, Path, description = "Version"),
    ),
    responses(
        (status = 200, description = "VS Code extension VSIX package"),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Unknown registry or extension"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/{extension_id}/{version}/vsix")]
pub async fn download_vsix(
    path: web::Path<(String, String, String)>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    map: web::Data<RegistryMap>,
) -> Result<impl Responder, AppError> {
    let (registry, extension_id, version) = path.into_inner();
    require_openvsx(&registry, &map)?;
    let pkg = PackageId::new(&registry, &extension_id, &version).with_artifact("vsix");
    proxy_stream(svc, pkg, identity, "source:read", None).await
}
