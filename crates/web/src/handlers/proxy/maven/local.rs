use super::{
    artifact_storage_key, build_metadata_xml, collect_storage_stream, content_type_for,
    maven_artifact_storage_key, AppError, AuthIdentity, HttpResponse, LocalRegistryService,
    MavenPathKind, RegistryMode,
};

/// Try to serve a Maven request from local/hybrid storage.
/// Returns `Ok(Some(response))` on a local hit, `Ok(None)` to fall through to proxy.
pub async fn maven_local_response(
    local_svc: &LocalRegistryService,
    registry: &str,
    kind: &MavenPathKind,
    identity: &AuthIdentity,
    mode: RegistryMode,
) -> Result<Option<HttpResponse>, AppError> {
    match kind {
        MavenPathKind::Metadata { name } => {
            handle_maven_metadata(local_svc, registry, name, identity, mode).await
        }
        MavenPathKind::Artifact {
            name,
            version,
            filename,
        } => {
            handle_maven_artifact(local_svc, registry, name, version, filename, identity, mode)
                .await
        }
    }
}

pub async fn handle_maven_metadata(
    local_svc: &LocalRegistryService,
    registry: &str,
    name: &str,
    identity: &AuthIdentity,
    mode: RegistryMode,
) -> Result<Option<HttpResponse>, AppError> {
    match local_svc.get_maven_versions(registry, name, identity).await {
        Ok(versions) => {
            let group_id = versions
                .first()
                .and_then(|v| v.index_metadata.get("group_id"))
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_owned();
            let artifact_id = versions
                .first()
                .and_then(|v| v.index_metadata.get("artifact_id"))
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_owned();
            let xml = build_metadata_xml(&group_id, &artifact_id, &versions)?;
            Ok(Some(
                HttpResponse::Ok().content_type("application/xml").body(xml),
            ))
        }
        Err(batlehub_core::error::CoreError::NotFound(_)) if mode == RegistryMode::Hybrid => {
            Ok(None)
        }
        Err(batlehub_core::error::CoreError::NotFound(msg)) => Err(AppError::not_found(msg)),
        Err(e) => Err(AppError::from(e)),
    }
}

pub async fn handle_maven_artifact(
    local_svc: &LocalRegistryService,
    registry: &str,
    name: &str,
    version: &str,
    filename: &str,
    identity: &AuthIdentity,
    mode: RegistryMode,
) -> Result<Option<HttpResponse>, AppError> {
    // Gate must be enforced before falling through to upstream.
    local_svc
        .check_prerelease_access(registry, version, identity)
        .await
        .map_err(AppError::from)?;
    let storage_key = if filename.ends_with(".pom") {
        artifact_storage_key(registry, name, version)
    } else {
        maven_artifact_storage_key(registry, name, version, filename)
    };
    match local_svc.storage.retrieve(&storage_key).await {
        Ok(Some(artifact)) => {
            let buf = collect_storage_stream(artifact.stream).await?;
            Ok(Some(
                HttpResponse::Ok()
                    .content_type(content_type_for(filename))
                    .body(buf),
            ))
        }
        Ok(None) if mode == RegistryMode::Hybrid => Ok(None),
        Ok(None) => Err(AppError::not_found(format!(
            "{name}@{version}/{filename} not found in local registry"
        ))),
        Err(e) if mode == RegistryMode::Hybrid => {
            tracing::warn!("local storage error, falling back to proxy: {e}");
            Ok(None)
        }
        Err(e) => Err(AppError::from(e)),
    }
}
