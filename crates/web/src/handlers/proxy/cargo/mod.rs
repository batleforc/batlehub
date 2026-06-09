use std::sync::Arc;

use actix_web::{delete, get, put, web, HttpRequest, HttpResponse, Responder};
use bytes::{Buf, Bytes};
use sha2::{Digest, Sha256};

use batlehub_config::schema::RegistryMode;
use batlehub_core::{
    entities::{CargoDep, CargoIndexEntry, PackageId},
    error::CoreError,
    services::{LocalRegistryService, ProxyService, PublishRequest},
};

use super::common::{
    append_signature_headers, collect_payload, extract_signature_headers, proxy_stream,
    require_local_mode,
};
use crate::{
    error::AppError, extractors::AuthIdentity, services::NotificationService, CargoIndexMap,
    RegistryMap, RegistryModeMap,
};
use batlehub_core::entities::NotificationEventType;

// ── Sparse index proxy ────────────────────────────────────────────────────────

/// HTTP client + upstream index URL for one cargo sparse index.
#[derive(Clone)]
pub struct CargoIndexProxy {
    pub http: reqwest::Client,
    /// Base URL of the upstream sparse index, e.g. `https://index.crates.io`.
    pub index_url: String,
}

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

pub use index::*;
pub use ownership::*;
pub use publish::*;
