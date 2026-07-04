use std::sync::Arc;

use actix_web::{delete, get, post, put, web, HttpRequest, HttpResponse, Responder};
use sha2::{Digest, Sha256};

use batlehub_config::schema::RegistryMode;
use batlehub_core::{
    entities::PackageId,
    services::{LocalRegistryService, ProxyService, PublishRequest},
};

use super::common::{
    collect_payload, extract_signature_headers, proxy_gem_specs, proxy_stream, require_local_mode,
    require_registry_type, serve_local_or_proxy_artifact, LocalOrProxyArtifactOpts,
};
use crate::{
    error::AppError, extractors::AuthIdentity, services::NotificationService, RegistryMap,
    RegistryModeMap,
};
use batlehub_core::entities::NotificationEventType;

pub mod download;
pub mod publish;
pub mod specs;

pub use download::{gem_download, gem_gemspec, gem_info, gem_versions};
pub use publish::{gem_publish, gem_unyank, gem_yank, GemYankQuery};
pub use specs::{gem_specs_full, gem_specs_latest, gem_specs_prerelease};
