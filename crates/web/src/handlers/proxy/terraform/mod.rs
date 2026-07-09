use std::sync::Arc;

use actix_web::{delete, get, post, put, web, HttpRequest, HttpResponse, Responder};
use sha2::{Digest, Sha256};

use batlehub_config::schema::RegistryMode;
use batlehub_core::{
    entities::PackageId,
    ports::StorageMeta,
    services::{
        terraform_provider_binary_storage_key, LocalRegistryService, ProxyService,
        PublishPolicyRequest, PublishRequest, TerraformPlatform,
    },
};

use super::common::{
    append_signature_headers, collect_payload, collect_storage_stream, dispatch_notification,
    extract_signature_headers, proxy_stream, require_local_mode, require_registry_type,
};
use crate::{
    error::AppError, extractors::AuthIdentity, services::NotificationService, RegistryMap,
    RegistryModeMap, UpstreamMap,
};
use batlehub_core::entities::NotificationEventType;

pub mod modules;
pub mod providers;
mod shared;

pub use modules::{
    terraform_module_artifact, terraform_module_download, terraform_module_unyank,
    terraform_module_upload, terraform_module_versions, terraform_module_yank,
};
pub use providers::{
    terraform_provider_artifact, terraform_provider_binary_upload, terraform_provider_download,
    terraform_provider_unyank, terraform_provider_upload, terraform_provider_versions,
    terraform_provider_yank,
};
pub(super) use shared::{
    base_url_from_req, terraform_set_yanked, terraform_versions_response, TerraformYankRequest,
};
