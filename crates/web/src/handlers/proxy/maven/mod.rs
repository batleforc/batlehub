use std::sync::Arc;

use actix_web::{get, put, web, HttpResponse, Responder};
use quick_xml::{
    events::{BytesDecl, BytesEnd, BytesStart, BytesText, Event},
    Writer,
};
use sha2::{Digest, Sha256};

use batlehub_config::schema::RegistryMode;
use batlehub_core::{
    entities::PackageId,
    ports::StorageMeta,
    services::{
        artifact_storage_key, maven_artifact_storage_key, LocalRegistryService, ProxyService,
        PublishRequest,
    },
};

use super::common::{
    collect_payload, collect_storage_stream, proxy_stream, require_local_mode,
    require_registry_type,
};
use crate::{
    error::AppError, extractors::AuthIdentity, services::NotificationService, RegistryMap,
    RegistryModeMap,
};
use batlehub_core::entities::NotificationEventType;

pub mod local;
pub mod proxy;
pub mod routing;

pub use local::*;
pub use proxy::*;
pub use routing::*;
