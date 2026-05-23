use std::sync::Arc;

use actix_web::{Responder, get, web};

use batlehub_core::{entities::PackageId, services::ProxyService};

use crate::{RegistryMap, error::AppError, extractors::AuthIdentity};
use super::common::proxy_stream;

fn require_maven(registry: &str, map: &RegistryMap) -> Result<(), AppError> {
    match map.type_of(registry) {
        Some("maven") => Ok(()),
        Some(_) => Err(AppError::not_found(format!(
            "registry '{registry}' is not a Maven registry"
        ))),
        None => Err(AppError::not_found(format!("unknown registry '{registry}'"))),
    }
}

fn content_type_for(filename: &str) -> &'static str {
    if filename.ends_with(".jar") {
        "application/java-archive"
    } else if filename.ends_with(".pom") || filename.ends_with(".xml") {
        "application/xml"
    } else if filename.ends_with(".sha1")
        || filename.ends_with(".md5")
        || filename.ends_with(".sha256")
        || filename.ends_with(".sha512")
    {
        "text/plain"
    } else {
        "application/octet-stream"
    }
}

/// Proxy a Maven repository request.
///
/// The `path` captures everything after `/maven2/`, which encodes the full
/// Maven 2 repository layout:
/// - Metadata:  `{group/path}/{artifactId}/maven-metadata.xml`
/// - Artifacts: `{group/path}/{artifactId}/{version}/{filename}`
///
/// Both are forwarded to the configured upstream using the `MavenRegistryClient`.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/maven2/{path}",
    tag = "proxy/maven",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("path"     = String, Path, description = "Maven repository path"),
    ),
    responses(
        (status = 200, description = "Maven artifact or metadata"),
        (status = 400, description = "Invalid Maven path"),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Artifact not found"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/maven2/{path:.*}")]
pub async fn maven_get(
    path: web::Path<(String, String)>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    map: web::Data<RegistryMap>,
) -> Result<impl Responder, AppError> {
    let (registry, maven_path) = path.into_inner();
    require_maven(&registry, &map)?;

    if maven_path.is_empty() {
        return Err(AppError::not_found("empty Maven path"));
    }

    let segments: Vec<&str> = maven_path.split('/').filter(|s| !s.is_empty()).collect();
    if segments.is_empty() {
        return Err(AppError::not_found("invalid Maven path"));
    }

    let filename = *segments.last().unwrap();

    let pkg = if filename == "maven-metadata.xml" {
        // Path: {group/path}/{artifactId}/maven-metadata.xml
        if segments.len() < 2 {
            return Err(AppError::not_found(
                "invalid Maven metadata path: missing artifactId",
            ));
        }
        let artifact_id = segments[segments.len() - 2];
        let group_segs = &segments[..segments.len() - 2];
        if group_segs.is_empty() {
            return Err(AppError::not_found(
                "invalid Maven metadata path: missing groupId",
            ));
        }
        let group_id = group_segs.join(".");
        PackageId::new(&registry, format!("{group_id}:{artifact_id}"), "maven-metadata.xml")
    } else {
        // Path: {group/path}/{artifactId}/{version}/{filename}
        if segments.len() < 4 {
            return Err(AppError::bad_request(format!(
                "invalid Maven artifact path '{maven_path}': expected group/artifact/version/filename"
            )));
        }
        let version = segments[segments.len() - 2];
        let artifact_id = segments[segments.len() - 3];
        let group_segs = &segments[..segments.len() - 3];
        let group_id = group_segs.join(".");
        PackageId::new(&registry, format!("{group_id}:{artifact_id}"), version)
            .with_artifact(filename)
    };

    proxy_stream(svc, pkg, identity, "releases:read", Some(content_type_for(filename))).await
}
