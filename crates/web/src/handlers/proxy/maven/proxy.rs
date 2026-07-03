use super::{
    collect_payload, content_type_for, get, maven_artifact_storage_key, maven_local_response,
    parse_maven_path, parse_pom, proxy_stream, put, require_local_mode, require_registry_type, web,
    AppError, Arc, AuthIdentity, Digest, HttpResponse, LocalRegistryService, MavenPathKind,
    NotificationEventType, NotificationService, PackageId, ProxyService, PublishRequest,
    RegistryMap, RegistryMode, RegistryModeMap, Responder, Sha256, StorageMeta,
};

/// Proxy or serve a Maven repository request.
///
/// In `Local`/`Hybrid` mode:
/// - `maven-metadata.xml` is generated dynamically from published versions in the DB.
/// - Artifact files are served from local storage; Hybrid falls back to upstream if not found.
///
/// In `Proxy` mode (default): forwards to the configured upstream.
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
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
) -> Result<impl Responder, AppError> {
    let (registry, maven_path) = path.into_inner();
    require_registry_type(&registry, "maven", &map)?;

    let mode = mode_map.get(&registry);
    let kind = parse_maven_path(&registry, &maven_path)?;

    if matches!(mode, RegistryMode::Local | RegistryMode::Hybrid) {
        if let Some(resp) =
            maven_local_response(&local_svc, &registry, &kind, &identity, mode).await?
        {
            return Ok(resp);
        }
    }

    // Proxy fallback (Proxy mode or Hybrid miss)
    let pkg = match &kind {
        MavenPathKind::Metadata { name } => {
            PackageId::new(&registry, name.clone(), "maven-metadata.xml")
        }
        MavenPathKind::Artifact {
            name,
            version,
            filename,
        } => PackageId::new(&registry, name.clone(), version.as_str())
            .with_artifact(filename.as_str()),
    };
    let filename = match &kind {
        MavenPathKind::Metadata { .. } => "maven-metadata.xml",
        MavenPathKind::Artifact { filename, .. } => filename.as_str(),
    };
    proxy_stream(
        svc,
        pkg,
        identity,
        batlehub_core::rules::resource_type::RELEASES_READ,
        Some(content_type_for(filename)),
    )
    .await
}

/// Upload a Maven artifact to the local registry.
///
/// Accepts any Maven 2 repository path:
/// - `.pom` files trigger the three-phase publish, storing version metadata.
/// - All other files (`.jar`, checksums, etc.) are stored directly and accessible via GET.
/// - Client-uploaded `maven-metadata.xml` is accepted but ignored (generated dynamically).
///
/// Only available when the registry is configured in `local` or `hybrid` mode.
#[utoipa::path(
    put,
    path = "/proxy/{registry}/maven2/{path}",
    tag = "proxy/maven",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("path"     = String, Path, description = "Maven repository path"),
    ),
    responses(
        (status = 200, description = "Accepted (maven-metadata.xml silently ignored)"),
        (status = 201, description = "Artifact stored"),
        (status = 400, description = "Invalid Maven path or malformed POM"),
        (status = 401, description = "Authentication required"),
        (status = 404, description = "Registry not found or not in local/hybrid mode"),
        (status = 409, description = "Version already published"),
    ),
    security(("bearer_token" = [])),
)]
#[allow(clippy::too_many_arguments)]
#[put("/proxy/{registry}/maven2/{path:.*}")]
pub async fn maven_put(
    path: web::Path<(String, String)>,
    identity: AuthIdentity,
    payload: web::Payload,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
    notification_svc: web::Data<Option<Arc<NotificationService>>>,
) -> Result<impl Responder, AppError> {
    let (registry, maven_path) = path.into_inner();
    require_registry_type(&registry, "maven", &map)?;
    require_local_mode(&registry, &mode_map)?;

    let kind = parse_maven_path(&registry, &maven_path)?;

    match kind {
        MavenPathKind::Metadata { .. } => {
            // Silently accept and ignore client-uploaded metadata.xml — generated dynamically.
            Ok(HttpResponse::Ok().finish())
        }
        MavenPathKind::Artifact {
            name,
            version,
            filename,
        } => {
            let bytes = collect_payload(payload).await?;

            if filename == "maven-metadata.xml" {
                return Ok(HttpResponse::Ok().finish());
            }

            if !filename.ends_with(".pom") {
                // Non-POM artifact (jar, sources, checksums, etc.): store directly.
                let storage_key = maven_artifact_storage_key(&registry, &name, &version, &filename);
                local_svc
                    .storage
                    .store(
                        &storage_key,
                        bytes,
                        StorageMeta {
                            content_type: Some(content_type_for(&filename).to_owned()),
                            size: None,
                            checksum: None,
                        },
                    )
                    .await
                    .map_err(AppError::from)?;
                return Ok(HttpResponse::Created().finish());
            }

            // .pom file: parse XML + run three-phase publish. The URL path is
            // the canonical Maven coordinate (it's what a later GET uses), so
            // a POM that declares a different version is rejected rather than
            // silently publishing under whatever the body claims.
            let pom = parse_pom(&bytes)?;
            if !pom.version.is_empty() && pom.version != version {
                return Err(AppError::bad_request(format!(
                    "POM declares version '{}' but URL path specifies '{}'",
                    pom.version, version
                )));
            }
            let resolved_version = version.clone();

            let checksum = hex::encode(Sha256::digest(&bytes));
            let index_metadata = serde_json::json!({
                "group_id": pom.group_id,
                "artifact_id": pom.artifact_id,
                "version": resolved_version,
                "packaging": pom.packaging,
                "description": pom.description,
                "sha256": checksum,
                "yanked": false,
            });

            let actor = identity.0.user_id.clone().unwrap_or_default();

            let quota_check = local_svc
                .publish(PublishRequest {
                    registry: registry.clone(),
                    name: name.clone(),
                    version: resolved_version.clone(),
                    artifact: bytes,
                    checksum,
                    index_metadata,
                    publisher: identity.0,
                    signature_bytes: None,
                    signature_type: None,
                })
                .await
                .map_err(AppError::from)?;

            super::super::common::dispatch_notification(
                &notification_svc,
                NotificationEventType::PackagePublished,
                &registry,
                &name,
                Some(resolved_version),
                &actor,
            );

            let mut resp = HttpResponse::Created();
            if let Some(limit) = quota_check.bytes_limit {
                resp.insert_header(("X-Quota-Used-Bytes", quota_check.bytes_used.to_string()));
                resp.insert_header(("X-Quota-Limit-Bytes", limit.to_string()));
            }
            Ok(resp.finish())
        }
    }
}
