use std::sync::Arc;

use actix_web::{delete, get, post, put, web, HttpRequest, HttpResponse, Responder};
use sha2::{Digest, Sha256};

use batlehub_config::schema::RegistryMode;
use batlehub_core::{
    entities::PackageId,
    services::{LocalRegistryService, ProxyService, PublishRequest},
};

use super::common::{
    append_signature_headers, collect_payload, extract_signature_headers, proxy_gem_specs,
    proxy_stream, require_local_mode, require_registry_type,
};
use crate::{
    error::AppError, extractors::AuthIdentity, services::NotificationService, RegistryMap,
    RegistryModeMap,
};
use batlehub_core::entities::NotificationEventType;

pub mod download;
pub mod publish;
pub mod specs;

pub use download::*;
pub use publish::*;
pub use specs::*;
