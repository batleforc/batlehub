use std::sync::Arc;

use actix_web::{delete, get, post, put, web, HttpRequest, HttpResponse, Responder};
use sha2::{Digest, Sha256};

use batlehub_config::schema::RegistryMode;
use batlehub_core::{
    entities::PackageId,
    ports::StorageMeta,
    services::{
        tf_provider_binary_storage_key, LocalRegistryService, ProxyService, PublishRequest,
        TerraformPlatform,
    },
};

use super::common::{
    append_signature_headers, collect_payload, collect_storage_stream, extract_signature_headers,
    proxy_stream, require_local_mode, require_registry_type,
};
use crate::{
    error::AppError, extractors::AuthIdentity, services::NotificationService, RegistryMap,
    RegistryModeMap, UpstreamMap,
};
use batlehub_core::entities::NotificationEventType;

pub mod modules;
pub mod providers;

pub use modules::{
    tf_module_artifact, tf_module_download, tf_module_unyank, tf_module_upload, tf_module_versions,
    tf_module_yank,
};
pub use providers::{
    tf_provider_artifact, tf_provider_binary_upload, tf_provider_download, tf_provider_unyank,
    tf_provider_upload, tf_provider_versions, tf_provider_yank,
};

pub(super) fn base_url_from_req(req: &HttpRequest) -> String {
    let info = req.connection_info();
    format!("{}://{}", info.scheme(), info.host())
}
