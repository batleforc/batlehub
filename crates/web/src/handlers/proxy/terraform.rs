use std::sync::Arc;

use actix_web::{HttpResponse, Responder, get, web};

use batlehub_core::{entities::PackageId, services::ProxyService};

use crate::{RegistryMap, UpstreamMap, error::AppError, extractors::AuthIdentity};
use super::common::proxy_stream;

fn require_terraform(registry: &str, map: &RegistryMap) -> Result<(), AppError> {
    match map.type_of(registry) {
        Some("terraform") => Ok(()),
        Some(_) => Err(AppError::not_found(format!(
            "registry '{registry}' is not a Terraform registry"
        ))),
        None => Err(AppError::not_found(format!("unknown registry '{registry}'"))),
    }
}

/// List available versions for a Terraform provider.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/v1/providers/{namespace}/{ptype}/versions",
    tag = "proxy/terraform",
    params(
        ("registry"  = String, Path, description = "Registry name"),
        ("namespace" = String, Path, description = "Provider namespace"),
        ("ptype"     = String, Path, description = "Provider type"),
    ),
    responses(
        (status = 200, description = "Provider versions JSON"),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Provider not found"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/v1/providers/{namespace}/{ptype}/versions")]
pub async fn tf_provider_versions(
    path: web::Path<(String, String, String)>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    map: web::Data<RegistryMap>,
) -> Result<impl Responder, AppError> {
    let (registry, namespace, ptype) = path.into_inner();
    require_terraform(&registry, &map)?;

    let pkg = PackageId::new(
        &registry,
        format!("providers/{namespace}/{ptype}"),
        "versions",
    );
    proxy_stream(svc, pkg, identity, "releases:read", Some("application/json")).await
}

/// Get download information for a specific Terraform provider version and platform.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/v1/providers/{namespace}/{ptype}/{version}/download/{os}/{arch}",
    tag = "proxy/terraform",
    params(
        ("registry"  = String, Path, description = "Registry name"),
        ("namespace" = String, Path, description = "Provider namespace"),
        ("ptype"     = String, Path, description = "Provider type"),
        ("version"   = String, Path, description = "Provider version"),
        ("os"        = String, Path, description = "Target OS"),
        ("arch"      = String, Path, description = "Target architecture"),
    ),
    responses(
        (status = 200, description = "Provider download info JSON (includes binary URL and checksums)"),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Provider not found"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/v1/providers/{namespace}/{ptype}/{version}/download/{os}/{arch}")]
pub async fn tf_provider_download(
    path: web::Path<(String, String, String, String, String, String)>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    map: web::Data<RegistryMap>,
) -> Result<impl Responder, AppError> {
    let (registry, namespace, ptype, version, os, arch) = path.into_inner();
    require_terraform(&registry, &map)?;

    let pkg = PackageId::new(
        &registry,
        format!("providers/{namespace}/{ptype}"),
        &version,
    )
    .with_artifact(format!("{os}/{arch}"));

    proxy_stream(svc, pkg, identity, "releases:read", Some("application/json")).await
}

/// List available versions for a Terraform module.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/v1/modules/{namespace}/{name}/{provider}/versions",
    tag = "proxy/terraform",
    params(
        ("registry"  = String, Path, description = "Registry name"),
        ("namespace" = String, Path, description = "Module namespace"),
        ("name"      = String, Path, description = "Module name"),
        ("provider"  = String, Path, description = "Module provider"),
    ),
    responses(
        (status = 200, description = "Module versions JSON"),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Module not found"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/v1/modules/{namespace}/{name}/{provider}/versions")]
pub async fn tf_module_versions(
    path: web::Path<(String, String, String, String)>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    map: web::Data<RegistryMap>,
) -> Result<impl Responder, AppError> {
    let (registry, namespace, name, provider) = path.into_inner();
    require_terraform(&registry, &map)?;

    let pkg = PackageId::new(
        &registry,
        format!("modules/{namespace}/{name}/{provider}"),
        "versions",
    );
    proxy_stream(svc, pkg, identity, "releases:read", Some("application/json")).await
}

/// Get the download URL for a specific Terraform module version.
///
/// Returns `204 No Content` with `X-Terraform-Get` header pointing to the source archive,
/// following the Terraform module registry protocol. The redirect is not cached.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/v1/modules/{namespace}/{name}/{provider}/{version}/download",
    tag = "proxy/terraform",
    params(
        ("registry"  = String, Path, description = "Registry name"),
        ("namespace" = String, Path, description = "Module namespace"),
        ("name"      = String, Path, description = "Module name"),
        ("provider"  = String, Path, description = "Module provider"),
        ("version"   = String, Path, description = "Module version"),
    ),
    responses(
        (status = 204, description = "X-Terraform-Get header contains the archive download URL"),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Module not found"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/v1/modules/{namespace}/{name}/{provider}/{version}/download")]
pub async fn tf_module_download(
    path: web::Path<(String, String, String, String, String)>,
    map: web::Data<RegistryMap>,
    upstream_map: web::Data<UpstreamMap>,
    client: web::Data<reqwest::Client>,
) -> Result<impl Responder, AppError> {
    let (registry, namespace, name, provider, version) = path.into_inner();
    require_terraform(&registry, &map)?;

    let upstream = upstream_map
        .upstream_for(&registry)
        .ok_or_else(|| AppError::not_found(format!("no upstream configured for '{registry}'")))?;

    let url = format!(
        "{}/v1/modules/{namespace}/{name}/{provider}/{version}/download",
        upstream.trim_end_matches('/')
    );

    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| AppError::internal(format!("terraform upstream request failed: {e}")))?;

    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(AppError::not_found(format!(
            "module {namespace}/{name}/{provider}@{version} not found"
        )));
    }

    // Forward X-Terraform-Get header as-is (the redirect URL to the source archive).
    if let Some(tf_get) = resp.headers().get("X-Terraform-Get") {
        let header_value = tf_get
            .to_str()
            .map_err(|_| AppError::internal("invalid X-Terraform-Get header from upstream"))?
            .to_owned();
        return Ok(HttpResponse::NoContent()
            .insert_header(("X-Terraform-Get", header_value))
            .finish());
    }

    // Some registries return 200 + body with the URL instead of 204 + header.
    let status = actix_web::http::StatusCode::from_u16(resp.status().as_u16())
        .unwrap_or(actix_web::http::StatusCode::INTERNAL_SERVER_ERROR);
    let body = resp
        .bytes()
        .await
        .map_err(|e| AppError::internal(format!("reading upstream response: {e}")))?;

    Ok(HttpResponse::build(status)
        .content_type("application/json")
        .body(body))
}
