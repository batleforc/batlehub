use std::sync::Arc;

use actix_web::{delete, get, put, web, HttpRequest, HttpResponse, Responder};
use bytes::{Buf, Bytes};
use sha2::{Digest, Sha256};

use batlehub_config::schema::RegistryMode;
use batlehub_core::{
    entities::{CargoDep, CargoIndexEntry},
    error::CoreError,
    services::{LocalRegistryService, ProxyService, PublishRequest},
};

use super::common::{
    collect_payload, extract_signature_headers, require_local_mode, serve_local_or_proxy_artifact,
    LocalOrProxyArtifactOpts,
};
use crate::{
    error::AppError, extractors::AuthIdentity, services::NotificationService, CargoIndexMap,
    RegistryMap, RegistryModeMap,
};
use batlehub_core::entities::NotificationEventType;

pub(super) fn require_cargo(registry: &str, map: &RegistryMap) -> Result<(), AppError> {
    match map.type_of(registry).as_deref() {
        Some("cargo") => Ok(()),
        Some(_) => Err(AppError::not_found(format!(
            "registry '{registry}' is not a cargo registry"
        ))),
        None => Err(AppError::not_found(format!(
            "unknown registry '{registry}'"
        ))),
    }
}

pub mod helpers;
pub mod index;
pub mod ownership;
pub mod publish;

pub use helpers::CargoIndexProxy;
pub use index::*;
pub use ownership::*;
pub use publish::*;
