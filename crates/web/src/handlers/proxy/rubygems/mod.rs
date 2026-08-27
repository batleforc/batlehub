use std::sync::Arc;

use actix_web::{delete, get, post, put, web, HttpRequest, HttpResponse, Responder};
use sha2::{Digest, Sha256};

use batlehub_core::{
    entities::PackageId,
    services::{LocalRegistryService, ProxyService, PublishRequest},
};

use super::common::{
    collect_payload, document_response, extract_signature_headers, fetch_proxy_document,
    proxy_gem_specs, proxy_stream, require_local_mode, require_registry_type,
    serve_local_or_proxy_artifact, serve_local_or_proxy_document, ArtifactSignature,
    LocalOrProxyArtifactOpts,
};
use crate::{
    error::AppError, extractors::AuthIdentity, services::NotificationService, RegistryMap,
    RegistryModeMap,
};
use batlehub_core::entities::NotificationEventType;

pub mod compact;
pub mod download;
pub mod publish;
mod range;
pub mod specs;

pub use compact::{gem_compact_info, gem_compact_names, gem_compact_versions};
pub use download::{gem_download, gem_gemspec, gem_info, gem_versions};
pub use publish::{gem_publish, gem_unyank, gem_yank, GemYankQuery};
pub use specs::{gem_specs_full, gem_specs_latest, gem_specs_prerelease};
