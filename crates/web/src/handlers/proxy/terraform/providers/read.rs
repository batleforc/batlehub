use super::{
    append_signature_headers, collect_storage_stream, get, proxy_stream, registry_public_base,
    require_registry_type, terraform_provider_binary_storage_key, terraform_versions_response, web,
    AppError, Arc, AuthIdentity, HttpRequest, HttpResponse, LocalRegistryService, PackageId,
    ProxyService, RegistryMap, RegistryMode, RegistryModeMap, Responder, TerraformPlatform,
};
use crate::handlers::schemas::{ArtifactBytes, UpstreamDocument};

/// List available versions for a Terraform provider.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/v1/providers/{namespace}/{ptype}/versions",
    tag = "proxy/terraform",
    params(
        ("registry"  = String, Path, description = "Registry name"),
        ("namespace" = String, Path, description = "Provider namespace"),
        ("ptype"     = String, Path, description = "Provider type"),
    ),
    responses(
        (status = 200, description = "Provider versions JSON", body = UpstreamDocument),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Provider not found"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/v1/providers/{namespace}/{ptype}/versions")]
pub async fn terraform_provider_versions(
    path: web::Path<(String, String, String)>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
) -> Result<impl Responder, AppError> {
    let (registry, namespace, ptype) = path.into_inner();
    require_registry_type(&registry, "terraform", &map)?;

    let name = format!("providers/{namespace}/{ptype}");
    let mode = mode_map.get(&registry);

    let local_result = if matches!(mode, RegistryMode::Local | RegistryMode::Hybrid) {
        Some(
            local_svc
                .get_terraform_provider_versions_response(&registry, &name, &identity)
                .await,
        )
    } else {
        None
    };

    terraform_versions_response(&registry, name, identity, svc, mode, local_result).await
}

/// Get download information for a specific Terraform provider version and platform.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/v1/providers/{namespace}/{ptype}/{version}/download/{os}/{arch}",
    tag = "proxy/terraform",
    params(
        ("registry"  = String, Path, description = "Registry name"),
        ("namespace" = String, Path, description = "Provider namespace"),
        ("ptype"     = String, Path, description = "Provider type"),
        ("version"   = String, Path, description = "Provider version"),
        ("os"        = String, Path, description = "Target OS"),
        ("arch"      = String, Path, description = "Target architecture"),
    ),
    responses(
        (status = 200, description = "Provider download info JSON (includes binary URL and checksums)", body = UpstreamDocument),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Provider not found"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/v1/providers/{namespace}/{ptype}/{version}/download/{os}/{arch}")]
pub async fn terraform_provider_download(
    path: web::Path<(String, String, String, String, String, String)>,
    req: HttpRequest,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
) -> Result<impl Responder, AppError> {
    let (registry, namespace, ptype, version, os, arch) = path.into_inner();
    require_registry_type(&registry, "terraform", &map)?;

    let name = format!("providers/{namespace}/{ptype}");
    let mode = mode_map.get(&registry);

    if matches!(mode, RegistryMode::Local | RegistryMode::Hybrid) {
        let base_url = registry_public_base(&req, &registry);
        if let Some(resp) = try_local_provider_download(
            &local_svc, &registry, &name, &version, &os, &arch, &base_url, &identity, mode,
        )
        .await?
        {
            return Ok(resp);
        }
    }

    // Proxy mode. This document used to be streamed through verbatim, so its
    // `download_url` named the upstream CDN and the client fetched the provider
    // zip from there: the rule chain ran on *this* request but never on the
    // bytes, which meant no cache, no download audit and no integrity check
    // (RFC 0006 §13.6, RFC 0009 §7.2).
    //
    // Fetched as a document and repointed at our own artifact route instead.
    //
    // Two documents, for two different jobs. The **listing** is fetched under
    // the real package name, so authorization and RFC 0006's version filtering
    // run against `providers/{ns}/{type}` as they do everywhere else — a blocked
    // version is absent from it, and that is what makes this endpoint refuse.
    // The **download document** is then fetched under a name that addresses it
    // in full.
    //
    // It used to be one fetch: the listing, returned as the answer. Terraform
    // needs `os`, `arch`, `filename`, `shasum` and the real `signing_keys`, none
    // of which a listing has, so it refused every provider (§12.12).
    let base_pkg = PackageId::new(&registry, &name, &version).with_artifact(format!("{os}/{arch}"));
    let req_ctx = batlehub_core::services::ProxyRequest {
        package_id: base_pkg,
        identity: identity.0.clone(),
        resource_type: batlehub_core::rules::resource_type::RELEASES_READ.to_owned(),
        ip_address: None,
        user_agent: None,
    };
    let listing = svc
        .version_document(
            &req_ctx,
            batlehub_core::ports::DocumentKind::Versions,
            &registry_public_base(&req, &registry),
        )
        .await
        .map_err(AppError::from)?;
    if !listing_offers_version(&listing, &version) {
        return Err(AppError::not_found(format!(
            "terraform provider {namespace}/{ptype} has no version {version} available \
             in registry '{registry}'"
        )));
    }

    let download_name = format!("{name}/{version}/download/{os}/{arch}");
    let download_ctx = batlehub_core::services::ProxyRequest {
        package_id: PackageId::new(&registry, &download_name, &version),
        identity: identity.0,
        resource_type: batlehub_core::rules::resource_type::RELEASES_READ.to_owned(),
        ip_address: None,
        user_agent: None,
    };
    let mut doc = svc
        .version_document(
            &download_ctx,
            batlehub_core::ports::DocumentKind::PROVIDER_DOWNLOAD,
            &registry_public_base(&req, &registry),
        )
        .await
        .map_err(AppError::from)?;

    let base = registry_public_base(&req, &registry);
    if let Some(obj) = doc.body.as_json_mut().and_then(|j| j.as_object_mut()) {
        obj.insert(
            "download_url".to_owned(),
            serde_json::json!(format!(
                "{}/v1/providers/{namespace}/{ptype}/{version}/artifact/{os}/{arch}",
                base.trim_end_matches('/')
            )),
        );
        // Terraform verifies the zip against these before installing it, so the
        // key set has to be present even when empty — an absent `signing_keys`
        // makes Terraform refuse a provider outright rather than skip the check.
        //
        // Upstream's keys are kept when it sent them: the checksums we proxy are
        // upstream's bytes, so they verify against upstream's key and against no
        // other. This default is for a registry that signs nothing.
        obj.entry("signing_keys")
            .or_insert_with(|| serde_json::json!({ "gpg_public_keys": [] }));

        // ...and the checksums it verifies against come through this proxy too,
        // or the last step of an air-gapped install reaches the internet.
        let root = base.trim_end_matches('/');
        obj.insert(
            "shasums_url".to_owned(),
            serde_json::json!(format!(
                "{root}/v1/providers/{namespace}/{ptype}/{version}/shasums"
            )),
        );
        obj.insert(
            "shasums_signature_url".to_owned(),
            serde_json::json!(format!(
                "{root}/v1/providers/{namespace}/{ptype}/{version}/shasums.sig"
            )),
        );
    }

    Ok(HttpResponse::Ok()
        .content_type("application/json")
        .body(match doc.body {
            batlehub_core::ports::DocumentBody::Json(v) => {
                serde_json::to_string(&v).unwrap_or_default()
            }
            batlehub_core::ports::DocumentBody::Text(t) => t,
        }))
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn try_local_provider_download(
    local_svc: &LocalRegistryService,
    registry: &str,
    name: &str,
    version: &str,
    os: &str,
    arch: &str,
    base_url: &str,
    identity: &AuthIdentity,
    mode: RegistryMode,
) -> Result<Option<HttpResponse>, AppError> {
    match local_svc
        .get_terraform_provider_download_response(
            registry,
            name,
            version,
            TerraformPlatform { os, arch },
            base_url,
            identity,
        )
        .await
    {
        Ok(json) => {
            let mut resp = HttpResponse::Ok();
            append_signature_headers(&mut resp, local_svc, registry, name, version).await;
            Ok(Some(resp.json(json)))
        }
        Err(batlehub_core::error::CoreError::NotFound(_)) if mode == RegistryMode::Hybrid => {
            Ok(None)
        }
        Err(batlehub_core::error::CoreError::NotFound(msg)) => Err(AppError::not_found(msg)),
        Err(e) => Err(AppError::from(e)),
    }
}

/// The provider's checksum manifest (`SHA256SUMS`) and its detached signature.
///
/// Terraform verifies the archive against these, so leaving them pointing at
/// the upstream made an otherwise air-gapped install reach the internet at the
/// last step — gated archive, ungated checksums (RFC 0009 §12.8). The URL is
/// named inside the download document rather than addressed by a path, so the
/// adapter resolves that document and follows the field.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/v1/providers/{namespace}/{ptype}/{version}/shasums",
    tag = "proxy/terraform",
    params(
        ("registry"  = String, Path, description = "Registry name"),
        ("namespace" = String, Path, description = "Provider namespace"),
        ("ptype"     = String, Path, description = "Provider type"),
        ("version"   = String, Path, description = "Provider version"),
    ),
    responses(
        (status = 200, description = "SHA256SUMS manifest", body = ArtifactBytes, content_type = "text/plain"),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Unknown provider or no checksum manifest"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/v1/providers/{namespace}/{ptype}/{version}/shasums")]
pub async fn terraform_provider_shasums(
    path: web::Path<(String, String, String, String)>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    map: web::Data<RegistryMap>,
) -> Result<impl Responder, AppError> {
    let (registry, namespace, ptype, version) = path.into_inner();
    require_registry_type(&registry, "terraform", &map)?;
    let pkg = PackageId::new(
        &registry,
        format!("providers/{namespace}/{ptype}"),
        &version,
    )
    .with_artifact("shasums");
    proxy_stream(
        svc,
        pkg,
        identity,
        batlehub_core::rules::resource_type::RELEASES_READ,
        Some("text/plain"),
    )
    .await
}

/// The detached signature over the checksum manifest. See
/// [`terraform_provider_shasums`].
#[utoipa::path(
    get,
    path = "/proxy/{registry}/v1/providers/{namespace}/{ptype}/{version}/shasums.sig",
    tag = "proxy/terraform",
    params(
        ("registry"  = String, Path, description = "Registry name"),
        ("namespace" = String, Path, description = "Provider namespace"),
        ("ptype"     = String, Path, description = "Provider type"),
        ("version"   = String, Path, description = "Provider version"),
    ),
    responses(
        (status = 200, description = "Detached signature", body = ArtifactBytes, content_type = "application/octet-stream"),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Unknown provider or no signature"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/v1/providers/{namespace}/{ptype}/{version}/shasums.sig")]
pub async fn terraform_provider_shasums_sig(
    path: web::Path<(String, String, String, String)>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    map: web::Data<RegistryMap>,
) -> Result<impl Responder, AppError> {
    let (registry, namespace, ptype, version) = path.into_inner();
    require_registry_type(&registry, "terraform", &map)?;
    let pkg = PackageId::new(
        &registry,
        format!("providers/{namespace}/{ptype}"),
        &version,
    )
    .with_artifact("shasums.sig");
    proxy_stream(
        svc,
        pkg,
        identity,
        batlehub_core::rules::resource_type::RELEASES_READ,
        Some("application/octet-stream"),
    )
    .await
}

/// Download a Terraform provider platform binary from local storage.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/v1/providers/{namespace}/{ptype}/{version}/artifact/{os}/{arch}",
    tag = "proxy/terraform",
    params(
        ("registry"  = String, Path, description = "Registry name"),
        ("namespace" = String, Path, description = "Provider namespace"),
        ("ptype"     = String, Path, description = "Provider type"),
        ("version"   = String, Path, description = "Provider version"),
        ("os"        = String, Path, description = "Target OS"),
        ("arch"      = String, Path, description = "Target architecture"),
    ),
    responses(
        (status = 200, description = "Provider binary", body = ArtifactBytes, content_type = "application/octet-stream"),
        (status = 404, description = "Binary not found"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/v1/providers/{namespace}/{ptype}/{version}/artifact/{os}/{arch}")]
pub async fn terraform_provider_artifact(
    path: web::Path<(String, String, String, String, String, String)>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
) -> Result<impl Responder, AppError> {
    let (registry, namespace, ptype, version, os, arch) = path.into_inner();
    require_registry_type(&registry, "terraform", &map)?;
    // Edge chokepoint: this handler builds a storage key directly from the path
    // components, so reject any traversal attempt with a clean 400 first.
    for (kind, value) in [
        ("namespace", &namespace),
        ("provider type", &ptype),
        ("version", &version),
        ("os", &os),
        ("arch", &arch),
    ] {
        batlehub_core::services::validate_path_safe(kind, value).map_err(AppError::from)?;
    }

    let auth_name = format!("providers/{namespace}/{ptype}");

    // Proxy mode reaches here because `terraform_provider_download` now points
    // `download_url` at this route rather than at upstream's CDN (RFC 0009
    // §7.2). `proxy_stream` runs the rule chain, caches the zip and records the
    // download — none of which happened while the client fetched it directly.
    if mode_map.get(&registry) == RegistryMode::Proxy {
        let pkg =
            PackageId::new(&registry, &auth_name, &version).with_artifact(format!("{os}/{arch}"));
        return proxy_stream(
            svc,
            pkg,
            identity,
            batlehub_core::rules::resource_type::RELEASES_READ,
            Some("application/zip"),
        )
        .await;
    }

    // Local/hybrid: no proxy fall-through, so enforce the registry rule chain
    // here — otherwise `[registries.rbac]` is never applied to a provider-binary
    // read served straight from local storage.
    svc.authorize_read(
        &PackageId::new(&registry, &auth_name, &version),
        &identity.0,
        batlehub_core::rules::resource_type::RELEASES_READ,
    )
    .await
    .map_err(AppError::from)?;

    local_svc
        .check_prerelease_access(&registry, &version, &identity)
        .await
        .map_err(AppError::from)?;

    let key =
        terraform_provider_binary_storage_key(&registry, &namespace, &ptype, &version, &os, &arch);
    let artifact = local_svc
        .storage
        .retrieve(&key)
        .await
        .map_err(AppError::from)?
        .ok_or_else(|| {
            AppError::not_found(format!(
                "provider {namespace}/{ptype}@{version} platform {os}/{arch} not found"
            ))
        })?;

    let buf = collect_storage_stream(artifact.stream).await?;
    Ok(HttpResponse::Ok().content_type("application/zip").body(buf))
}

/// Whether a versions listing still offers `version`.
///
/// The listing is what RFC 0006's filtering acts on, so a blocked version is
/// simply absent from it — checking here is what stops the download document
/// from answering for a version the listing denies. Shape-tolerant on purpose:
/// an upstream that answers with something unrecognisable is not treated as a
/// denial, because the artifact fetch enforces the same rules again.
fn listing_offers_version(doc: &batlehub_core::ports::VersionDocument, version: &str) -> bool {
    let Some(json) = doc.body.as_json() else {
        return true;
    };
    let Some(versions) = json.get("versions").and_then(|v| v.as_array()) else {
        return true;
    };
    versions
        .iter()
        .filter_map(|v| v.get("version").and_then(|v| v.as_str()))
        .any(|v| v == version)
}
