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

use super::common::{collect_payload, proxy_stream, require_local_mode};
use crate::{error::AppError, extractors::AuthIdentity, RegistryMap, RegistryModeMap};

fn require_maven(registry: &str, map: &RegistryMap) -> Result<(), AppError> {
    match map.type_of(registry).as_deref() {
        Some("maven") => Ok(()),
        Some(_) => Err(AppError::not_found(format!(
            "registry '{registry}' is not a Maven registry"
        ))),
        None => Err(AppError::not_found(format!(
            "unknown registry '{registry}'"
        ))),
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

enum MavenPathKind {
    /// `maven-metadata.xml` request — carries the resolved `groupId:artifactId` name.
    Metadata { name: String },
    /// Normal artifact — jar, pom, checksum, etc.
    Artifact {
        name: String,
        version: String,
        filename: String,
    },
}

fn parse_maven_path(_registry: &str, maven_path: &str) -> Result<MavenPathKind, AppError> {
    if maven_path.is_empty() {
        return Err(AppError::not_found("empty Maven path"));
    }
    let segments: Vec<&str> = maven_path.split('/').filter(|s| !s.is_empty()).collect();
    if segments.is_empty() {
        return Err(AppError::not_found("invalid Maven path"));
    }

    let filename = *segments.last().unwrap();

    if filename == "maven-metadata.xml" {
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
        Ok(MavenPathKind::Metadata {
            name: format!("{group_id}:{artifact_id}"),
        })
    } else {
        if segments.len() < 4 {
            return Err(AppError::bad_request(format!(
                "invalid Maven artifact path '{maven_path}': expected group/artifact/version/filename"
            )));
        }
        let version = segments[segments.len() - 2];
        let artifact_id = segments[segments.len() - 3];
        let group_segs = &segments[..segments.len() - 3];
        let group_id = group_segs.join(".");
        Ok(MavenPathKind::Artifact {
            name: format!("{group_id}:{artifact_id}"),
            version: version.to_owned(),
            filename: filename.to_owned(),
        })
    }
}

/// Build a `maven-metadata.xml` document from locally published versions.
fn build_metadata_xml(
    group_id: &str,
    artifact_id: &str,
    versions: &[batlehub_core::entities::PublishedPackage],
) -> Result<String, AppError> {
    use chrono::Utc;

    let non_yanked: Vec<_> = versions.iter().filter(|v| !v.yanked).collect();

    let release = non_yanked
        .iter()
        .rfind(|v| !v.version.contains("SNAPSHOT"))
        .map(|v| v.version.as_str())
        .unwrap_or("");

    let latest = non_yanked.last().map(|v| v.version.as_str()).unwrap_or("");

    let last_updated = Utc::now().format("%Y%m%d%H%M%S").to_string();

    let mut buf = Vec::new();
    let mut w = Writer::new_with_indent(&mut buf, b' ', 2);

    w.write_event(Event::Decl(BytesDecl::new("1.0", Some("UTF-8"), None)))
        .map_err(|e| AppError::internal(format!("xml write: {e}")))?;

    let metadata = BytesStart::new("metadata");
    w.write_event(Event::Start(metadata))
        .map_err(|e| AppError::internal(format!("xml write: {e}")))?;

    macro_rules! leaf {
        ($w:expr, $tag:expr, $val:expr) => {{
            $w.write_event(Event::Start(BytesStart::new($tag)))
                .map_err(|e| AppError::internal(format!("xml write: {e}")))?;
            $w.write_event(Event::Text(BytesText::new($val)))
                .map_err(|e| AppError::internal(format!("xml write: {e}")))?;
            $w.write_event(Event::End(BytesEnd::new($tag)))
                .map_err(|e| AppError::internal(format!("xml write: {e}")))?;
        }};
    }

    leaf!(w, "groupId", group_id);
    leaf!(w, "artifactId", artifact_id);

    w.write_event(Event::Start(BytesStart::new("versioning")))
        .map_err(|e| AppError::internal(format!("xml write: {e}")))?;

    leaf!(w, "release", release);
    leaf!(w, "latest", latest);

    w.write_event(Event::Start(BytesStart::new("versions")))
        .map_err(|e| AppError::internal(format!("xml write: {e}")))?;
    for v in &non_yanked {
        leaf!(w, "version", &v.version);
    }
    w.write_event(Event::End(BytesEnd::new("versions")))
        .map_err(|e| AppError::internal(format!("xml write: {e}")))?;

    leaf!(w, "lastUpdated", &last_updated);

    w.write_event(Event::End(BytesEnd::new("versioning")))
        .map_err(|e| AppError::internal(format!("xml write: {e}")))?;

    w.write_event(Event::End(BytesEnd::new("metadata")))
        .map_err(|e| AppError::internal(format!("xml write: {e}")))?;

    String::from_utf8(buf).map_err(|e| AppError::internal(format!("xml encode: {e}")))
}

struct PomMetadata {
    group_id: String,
    artifact_id: String,
    version: String,
    packaging: Option<String>,
    description: Option<String>,
}

fn parse_pom(bytes: &[u8]) -> Result<PomMetadata, AppError> {
    use quick_xml::events::Event as XE;
    use quick_xml::Reader;

    let mut reader = Reader::from_reader(bytes);
    reader.config_mut().trim_text(true);

    let mut group_id = None::<String>;
    let mut artifact_id = None::<String>;
    let mut version = None::<String>;
    let mut packaging = None::<String>;
    let mut description = None::<String>;
    let mut depth: u32 = 0;
    let mut current_tag = String::new();

    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(XE::Start(e)) => {
                depth += 1;
                if depth == 2 {
                    current_tag = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
                }
            }
            Ok(XE::Text(e)) if depth == 2 => {
                let raw = e
                    .decode()
                    .map_err(|e| AppError::unprocessable(format!("pom parse: {e}")))?;
                let text = quick_xml::escape::unescape(&raw)
                    .map_err(|e| AppError::unprocessable(format!("pom parse: {e}")))?
                    .into_owned();
                match current_tag.as_str() {
                    "groupId" => group_id = Some(text),
                    "artifactId" => artifact_id = Some(text),
                    "version" => version = Some(text),
                    "packaging" => packaging = Some(text),
                    "description" => description = Some(text),
                    _ => {}
                }
            }
            Ok(XE::End(_)) => {
                if depth == 2 {
                    current_tag.clear();
                }
                depth = depth.saturating_sub(1);
            }
            Ok(XE::Eof) => break,
            Err(e) => return Err(AppError::unprocessable(format!("pom parse error: {e}"))),
            _ => {}
        }
        buf.clear();
    }

    let group_id = group_id
        .filter(|s| !s.is_empty())
        .ok_or_else(|| AppError::unprocessable("POM missing <groupId>"))?;
    let artifact_id = artifact_id
        .filter(|s| !s.is_empty())
        .ok_or_else(|| AppError::unprocessable("POM missing <artifactId>"))?;
    let version = version.unwrap_or_default();

    Ok(PomMetadata {
        group_id,
        artifact_id,
        version,
        packaging,
        description,
    })
}

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
    require_maven(&registry, &map)?;

    let mode = mode_map.get(&registry);
    let kind = parse_maven_path(&registry, &maven_path)?;

    match mode {
        RegistryMode::Local | RegistryMode::Hybrid => {
            match &kind {
                MavenPathKind::Metadata { name } => {
                    match local_svc
                        .get_maven_versions(&registry, name, &identity)
                        .await
                    {
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
                            return Ok(HttpResponse::Ok()
                                .content_type("application/xml")
                                .body(xml));
                        }
                        Err(batlehub_core::error::CoreError::NotFound(_))
                            if mode == RegistryMode::Hybrid => {}
                        Err(batlehub_core::error::CoreError::NotFound(msg)) => {
                            return Err(AppError::not_found(msg));
                        }
                        Err(e) => return Err(AppError::from(e)),
                    }
                }
                MavenPathKind::Artifact {
                    name,
                    version,
                    filename,
                } => {
                    // Gate must be enforced before falling through to upstream: a non-member
                    // must not receive a pre-release artifact from the upstream registry.
                    local_svc
                        .check_prerelease_access(&registry, version, &identity)
                        .await
                        .map_err(AppError::from)?;
                    {
                        let storage_key = if filename.ends_with(".pom") {
                            artifact_storage_key(&registry, name, version)
                        } else {
                            maven_artifact_storage_key(&registry, name, version, filename)
                        };
                        match local_svc.storage.retrieve(&storage_key).await {
                            Ok(Some(artifact)) => {
                                use futures::StreamExt;
                                let mut buf = Vec::new();
                                let mut stream = artifact.stream;
                                while let Some(chunk) = stream.next().await {
                                    buf.extend_from_slice(
                                        &chunk.map_err(|e| AppError::internal(e.to_string()))?,
                                    );
                                }
                                return Ok(HttpResponse::Ok()
                                    .content_type(content_type_for(filename))
                                    .body(buf));
                            }
                            Ok(None) if mode == RegistryMode::Hybrid => {}
                            Ok(None) => {
                                return Err(AppError::not_found(format!(
                                    "{name}@{version}/{filename} not found in local registry"
                                )));
                            }
                            Err(e) if mode == RegistryMode::Hybrid => {
                                tracing::warn!("local storage error, falling back to proxy: {e}");
                            }
                            Err(e) => return Err(AppError::from(e)),
                        }
                    } // close else block for prerelease check
                }
            }
        }
        RegistryMode::Proxy => {}
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
        "releases:read",
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
#[put("/proxy/{registry}/maven2/{path:.*}")]
pub async fn maven_put(
    path: web::Path<(String, String)>,
    identity: AuthIdentity,
    payload: web::Payload,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
) -> Result<impl Responder, AppError> {
    let (registry, maven_path) = path.into_inner();
    require_maven(&registry, &map)?;
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

            // .pom file: parse XML + run three-phase publish.
            let pom = parse_pom(&bytes)?;
            let resolved_version = if pom.version.is_empty() {
                version.clone()
            } else {
                pom.version.clone()
            };

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

            let mut resp = HttpResponse::Created();
            if let Some(limit) = quota_check.bytes_limit {
                resp.insert_header(("X-Quota-Used-Bytes", quota_check.bytes_used.to_string()));
                resp.insert_header(("X-Quota-Limit-Bytes", limit.to_string()));
            }
            Ok(resp.finish())
        }
    }
}
