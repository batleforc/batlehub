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

pub mod local;
pub mod proxy;
pub mod routing;

pub use local::{handle_maven_artifact, handle_maven_metadata, maven_local_response};
pub use proxy::{maven_get, maven_put};
pub use routing::{
    build_metadata_xml, content_type_for, parse_maven_path, parse_pom, MavenPathKind, PomMetadata,
};
