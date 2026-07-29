use std::sync::Arc;

use actix_web::{get, post, web, HttpRequest, HttpResponse, Responder};

use batlehub_core::{
    entities::PackageId,
    services::{LocalRegistryService, ProxyService},
};

use super::common::{
    proxy_stream, serve_local_or_proxy_artifact, serve_local_or_proxy_json,
    LocalOrProxyArtifactOpts,
};
use crate::{error::AppError, extractors::AuthIdentity, RegistryMap, RegistryModeMap, UpstreamMap};

pub mod read;
pub mod write;

pub use read::{audit_bulk, audit_quick, download_tarball, get_packument, get_version};
pub use write::npm_publish;

pub(crate) fn require_npm_or_cargo(
    registry: &str,
    map: &crate::RegistryMap,
) -> Result<(), crate::error::AppError> {
    match map.type_of(registry).as_deref() {
        Some("npm") | Some("cargo") | Some("openvsx") => Ok(()),
        Some(_) => Err(crate::error::AppError::not_found(format!(
            "registry '{registry}' is not an npm, cargo, or openvsx registry"
        ))),
        None => Err(crate::error::AppError::not_found(format!(
            "unknown registry '{registry}'"
        ))),
    }
}

pub(crate) fn require_npm(
    registry: &str,
    map: &crate::RegistryMap,
) -> Result<(), crate::error::AppError> {
    match map.type_of(registry).as_deref() {
        Some("npm") => Ok(()),
        Some(_) => Err(crate::error::AppError::not_found(format!(
            "registry '{registry}' is not an npm registry"
        ))),
        None => Err(crate::error::AppError::not_found(format!(
            "unknown registry '{registry}'"
        ))),
    }
}

/// Base URL of this proxy as seen by the caller. Forwarded host/scheme headers
/// are honoured only from a trusted peer — see [`crate::middleware::proxy_trust`].
pub(crate) fn base_url(req: &actix_web::HttpRequest) -> String {
    crate::middleware::trusted_base_url(req)
}
