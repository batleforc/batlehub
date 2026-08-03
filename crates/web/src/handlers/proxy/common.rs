use std::sync::Arc;

use actix_web::{web, HttpRequest, HttpResponse};
use bytes::{Bytes, BytesMut};
use futures::StreamExt;

use batlehub_config::schema::RegistryMode;
use batlehub_core::{
    entities::{NotificationEvent, NotificationEventType, PackageId},
    error::CoreError,
    ports::ByteStream,
    services::{LocalRegistryService, ProxyRequest, ProxyResponse, ProxyService, PublishRequest},
};

use crate::{
    error::AppError,
    extractors::AuthIdentity,
    middleware::{host_routing::host_routed_registry, proxy_trust::trusted_origin},
    services::NotificationService,
    RegistryHostMap, RegistryMap, RegistryModeMap,
};

/// The public base URL of `registry` **as seen by this client** — the prefix every
/// self-referencing URL the server generates should be built on.
///
/// ```text
/// https://npm.acme.io                     on a request that arrived on npm1's host
/// https://hub.example.com/proxy/npm1      otherwise
/// ```
///
/// Callers format `{base}/…` with no literal `/proxy/` anywhere: the shape of the
/// ingress is this function's business, not theirs. The same contract applies to
/// the `base_url` parameter of the `LocalRegistryService` methods that build
/// metadata documents — it is a *registry* base, not a server origin.
///
/// The scheme and host come from [`trusted_origin`], so this is also the single
/// place that decides whether a forwarded header may influence a generated URL.
pub fn registry_public_base(req: &HttpRequest, registry: &str) -> String {
    let (scheme, host) = trusted_origin(req);
    let routed = host_routed_registry(req);

    // The client reached this very registry at a host root, so its URLs are
    // rooted there too.
    if routed.as_deref() == Some(registry) {
        return format!("{scheme}://{host}");
    }

    // Two cases where `{host}/proxy/{registry}` would not resolve for the client:
    // the registry has no subpath ingress at all (`path_routing = false`), or the
    // request arrived on *another* registry's host, where every path already
    // belongs to that other registry. Both need this registry's own advertised
    // URL, which is only known when host routing is configured.
    if let Some(map) = req.app_data::<web::Data<RegistryHostMap>>() {
        if map.is_host_only(registry) || routed.is_some() {
            if let Some(public) = map.public_url_for(registry) {
                return public;
            }
        }
    }

    format!("{scheme}://{host}/proxy/{registry}")
}

/// Decode `X-Artifact-Signature` (base64) and `X-Signature-Type` headers from a request.
///
/// Returns `(signature_bytes, signature_type)`. Either or both may be `None`.
pub fn extract_signature_headers(req: &HttpRequest) -> (Option<Vec<u8>>, Option<String>) {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    let sig_bytes = req
        .headers()
        .get("X-Artifact-Signature")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| STANDARD.decode(s.trim()).ok());
    let sig_type = req
        .headers()
        .get("X-Signature-Type")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    (sig_bytes, sig_type)
}

/// Append `X-Artifact-Signature` (base64) and `X-Signature-Type` headers to a response
/// if the package version has a stored signature.
pub async fn append_signature_headers(
    resp: &mut actix_web::HttpResponseBuilder,
    local_svc: &LocalRegistryService,
    registry: &str,
    name: &str,
    version: &str,
) {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    if let Some(meta) = local_svc.get_version_meta(registry, name, version).await {
        if let Some(ref sig) = meta.signature_bytes {
            resp.insert_header(("X-Artifact-Signature", STANDARD.encode(sig)));
        }
        if let Some(ref sig_type) = meta.signature_type {
            resp.insert_header(("X-Signature-Type", sig_type.as_str()));
        }
    }
}

/// Upload ceiling shared by every publish path (raw payloads and multipart
/// alike): prevents OOM from unbounded uploads before the service-layer size
/// check fires.
pub const MAX_UPLOAD_BYTES: u64 = 500 * 1024 * 1024;

/// Drain an actix streaming body into a contiguous `Bytes` buffer, bounded by
/// [`MAX_UPLOAD_BYTES`].
pub async fn collect_payload(mut payload: web::Payload) -> Result<Bytes, AppError> {
    let mut raw = BytesMut::new();
    while let Some(chunk) = payload.next().await {
        let chunk = chunk.map_err(|e| AppError::bad_request(e.to_string()))?;
        if raw.len() as u64 + chunk.len() as u64 > MAX_UPLOAD_BYTES {
            return Err(AppError::from(CoreError::PayloadTooLarge(format!(
                "upload exceeds the {MAX_UPLOAD_BYTES}-byte limit"
            ))));
        }
        raw.extend_from_slice(&chunk);
    }
    Ok(raw.freeze())
}

/// Drain a storage `ByteStream` into contiguous `Bytes`.
pub async fn collect_storage_stream(mut stream: ByteStream) -> Result<Bytes, AppError> {
    let mut buf = Vec::new();
    while let Some(chunk) = stream.next().await {
        buf.extend_from_slice(&chunk?);
    }
    Ok(Bytes::from(buf))
}

/// Fire-and-forget notification dispatch.
///
/// Silently skips if notifications are not configured (`None`).
pub fn dispatch_notification(
    svc: &web::Data<Option<Arc<NotificationService>>>,
    event_type: NotificationEventType,
    registry: &str,
    package_name: &str,
    version: Option<String>,
    actor: &str,
) {
    if let Some(svc) = svc.as_ref().as_ref() {
        let event = NotificationEvent::new(event_type, registry, package_name, version, actor);
        svc.dispatch_event_background(event);
    }
}

/// Publish an artifact, fire a `PackagePublished` notification, and build a JSON
/// response carrying the publish quota headers.
///
/// Collapses the publish→notify→respond tail shared by every local/hybrid publish
/// handler. `status` is the success status (200/201) and `body` the JSON payload.
pub async fn publish_and_respond(
    local_svc: &LocalRegistryService,
    notification_svc: &web::Data<Option<Arc<NotificationService>>>,
    req: PublishRequest,
    status: actix_web::http::StatusCode,
    body: serde_json::Value,
) -> Result<HttpResponse, AppError> {
    let registry = req.registry.clone();
    let name = req.name.clone();
    let version = req.version.clone();
    let actor = req.publisher.user_id.clone().unwrap_or_default();

    let quota = local_svc.publish(req).await.map_err(AppError::from)?;
    dispatch_notification(
        notification_svc,
        NotificationEventType::PackagePublished,
        &registry,
        &name,
        Some(version),
        &actor,
    );

    let mut resp = HttpResponse::build(status);
    for (header, value) in quota.headers() {
        resp.insert_header((header, value));
    }
    Ok(resp.json(body))
}

/// Reject the request if `registry` is not of the expected type.
///
/// Returns `404 Not Found` with a descriptive message for both "wrong type" and
/// "registry does not exist" — the two cases are indistinguishable to the caller.
pub fn require_registry_type(
    registry: &str,
    expected: &str,
    map: &RegistryMap,
) -> Result<(), AppError> {
    match map.type_of(registry).as_deref() {
        Some(t) if t == expected => Ok(()),
        Some(_) => Err(AppError::not_found(format!(
            "registry '{registry}' is not a {expected} registry"
        ))),
        None => Err(AppError::not_found(format!(
            "unknown registry '{registry}'"
        ))),
    }
}

/// Reject registries that are not in local or hybrid mode.
pub fn require_local_mode(registry: &str, mode_map: &RegistryModeMap) -> Result<(), AppError> {
    match mode_map.get(registry) {
        RegistryMode::Local | RegistryMode::Hybrid => Ok(()),
        RegistryMode::Proxy => Err(AppError::not_found(format!(
            "registry '{registry}' is not a local registry (mode = proxy)"
        ))),
    }
}

/// Serve a RubyGems binary specs index (`specs`, `latest_specs`, or `prerelease_specs`).
///
/// Returns `404` for local-only registries — these compact indexes are only
/// meaningful in proxy/hybrid mode; local registries expose
/// `/api/v1/versions/{name}.json` instead.
pub async fn proxy_gem_specs(
    registry: &str,
    spec_type: &str,
    svc: web::Data<Arc<ProxyService>>,
    identity: AuthIdentity,
    map: &RegistryMap,
    mode_map: &RegistryModeMap,
) -> Result<HttpResponse, AppError> {
    require_registry_type(registry, "rubygems", map)?;

    if mode_map.get(registry) == RegistryMode::Local {
        return Err(AppError::not_found(
            "binary specs index is not available for local-only registries; use /api/v1/versions/{name}.json".to_owned(),
        ));
    }

    let pkg = PackageId::new(registry, "_index", spec_type);
    proxy_stream(
        svc,
        pkg,
        identity,
        batlehub_core::rules::resource_type::RELEASES_READ,
        Some("application/octet-stream"),
    )
    .await
}

/// Send a proxy request and stream the result back to the HTTP client.
///
/// Pass `content_type = Some("application/json")` to set an explicit `Content-Type`
/// header on the response; pass `None` to let actix-web use its default.
pub async fn proxy_stream(
    svc: web::Data<Arc<ProxyService>>,
    pkg: PackageId,
    identity: AuthIdentity,
    resource_type: &str,
    content_type: Option<&str>,
) -> Result<HttpResponse, AppError> {
    let req = ProxyRequest {
        package_id: pkg,
        identity: identity.0,
        resource_type: resource_type.to_owned(),
        ip_address: None,
        user_agent: None,
    };
    match svc.handle(req).await.map_err(AppError::from)? {
        ProxyResponse::Denied { reason } => Err(AppError::forbidden(reason)),
        ProxyResponse::Stream(stream) => {
            let body = stream
                .filter_map(|chunk| async move { chunk.ok().map(Ok::<Bytes, actix_web::Error>) });
            let mut resp = HttpResponse::Ok();
            if let Some(ct) = content_type {
                resp.content_type(ct);
            }
            Ok(resp.streaming(body))
        }
    }
}

/// Options controlling [`serve_local_or_proxy_artifact`]'s behaviour.
pub struct LocalOrProxyArtifactOpts<'a> {
    /// Suffix passed to `PackageId::with_artifact(...)` on the proxy fallback,
    /// e.g. `"gem"`, `"dl"`, `"tarball"`, or a full filename.
    pub artifact_suffix: &'a str,
    /// `Content-Type` set on a local/hybrid hit.
    pub local_content_type: &'static str,
    /// `Content-Type` passed to [`proxy_stream`] on the proxy fallback.
    pub proxy_content_type: Option<&'static str>,
    /// `resource_type` passed to [`proxy_stream`], e.g. `"releases:read"`, `"source:read"`.
    pub resource_type: &'a str,
    /// Call `local_svc.check_prerelease_access(...)` before `get_artifact` in
    /// the Local/Hybrid branches.
    pub check_prerelease: bool,
    /// Call [`append_signature_headers`] on a local/hybrid hit.
    pub append_signature: bool,
}

/// Serve an artifact from local storage (Local/Hybrid mode) or fall back to
/// streaming it from the upstream registry (Proxy mode, or a Hybrid miss).
///
/// This is the shared shape behind `gem_download`, `download_crate`,
/// `download_tarball`, and similar registry-artifact download handlers.
#[allow(clippy::too_many_arguments)]
pub async fn serve_local_or_proxy_artifact(
    svc: web::Data<Arc<ProxyService>>,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    mode_map: &RegistryModeMap,
    registry: &str,
    name: &str,
    version: &str,
    identity: AuthIdentity,
    opts: LocalOrProxyArtifactOpts<'_>,
) -> Result<HttpResponse, AppError> {
    let mode = mode_map.get(registry);

    if matches!(mode, RegistryMode::Local | RegistryMode::Hybrid) {
        // Enforce the registry's RBAC (`[registries.rbac]`) before serving from
        // local storage. `get_artifact` only checks per-package Visibility and
        // pre-release gating — it never runs the registry rule chain, which is
        // only evaluated on the proxy fall-through. Without this, a local hit
        // would bypass a registry that denies e.g. anonymous `releases:read`
        // while its packages keep the default Public visibility. Mirrors the
        // deb/rpm `repo_get` guard so local and proxy reads stay consistent.
        svc.authorize_read(
            &PackageId::new(registry, name, version),
            &identity.0,
            opts.resource_type,
        )
        .await
        .map_err(AppError::from)?;
        if opts.check_prerelease {
            local_svc
                .check_prerelease_access(registry, version, &identity)
                .await
                .map_err(AppError::from)?;
        }
        match local_svc
            .get_artifact(registry, name, version, &identity)
            .await
        {
            Ok(bytes) => {
                let mut resp = HttpResponse::Ok();
                resp.content_type(opts.local_content_type);
                if opts.append_signature {
                    append_signature_headers(&mut resp, &local_svc, registry, name, version).await;
                }
                return Ok(resp.body(bytes));
            }
            // Not found locally in hybrid mode; fall through to upstream.
            Err(CoreError::NotFound(_)) if matches!(mode, RegistryMode::Hybrid) => {}
            Err(e) => return Err(AppError::from(e)),
        }
    }

    let pkg = PackageId::new(registry, name, version).with_artifact(opts.artifact_suffix);
    proxy_stream(
        svc,
        pkg,
        identity,
        opts.resource_type,
        opts.proxy_content_type,
    )
    .await
}

/// Serve JSON metadata from local storage (Local/Hybrid mode) or fall back to
/// streaming it from the upstream registry (Proxy mode, or a Hybrid miss).
///
/// This is the shared shape behind `get_packument`, `get_version`, `gem_info`,
/// `gem_versions`, `goproxy_latest`, and `composer_p2_metadata`: check the
/// registry mode, try `local_fetch` in Local/Hybrid mode, fall through to
/// `proxy_stream` on a Hybrid miss (or directly in Proxy mode).
#[allow(clippy::too_many_arguments)]
pub async fn serve_local_or_proxy_json<T, F, Fut>(
    svc: web::Data<Arc<ProxyService>>,
    mode_map: &RegistryModeMap,
    registry: &str,
    identity: AuthIdentity,
    local_fetch: F,
    not_found_msg: String,
    pkg: PackageId,
    resource_type: &str,
    proxy_content_type: Option<&str>,
) -> Result<HttpResponse, AppError>
where
    T: serde::Serialize,
    F: FnOnce(batlehub_core::entities::Identity) -> Fut,
    Fut: std::future::Future<Output = Result<T, CoreError>>,
{
    let mode = mode_map.get(registry);
    if matches!(mode, RegistryMode::Local | RegistryMode::Hybrid) {
        // Enforce the registry's RBAC before serving metadata from local storage:
        // the local fetch only checks per-package Visibility, not the registry
        // rule chain (which the proxy fall-through would run). See the artifact
        // helper above for the full rationale.
        svc.authorize_read(&pkg, &identity.0, resource_type)
            .await
            .map_err(AppError::from)?;
        match local_fetch(identity.0.clone()).await {
            Ok(x) => return Ok(HttpResponse::Ok().content_type("application/json").json(x)),
            Err(CoreError::NotFound(_)) if matches!(mode, RegistryMode::Hybrid) => {}
            Err(CoreError::NotFound(_)) => return Err(AppError::not_found(not_found_msg)),
            Err(e) => return Err(AppError::from(e)),
        }
    }
    proxy_stream(svc, pkg, identity, resource_type, proxy_content_type).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn map_with(registry: &str, type_: &str) -> RegistryMap {
        let mut m = HashMap::new();
        m.insert(registry.to_owned(), type_.to_owned());
        RegistryMap::from(m)
    }

    #[test]
    fn require_registry_type_ok() {
        let map = map_with("r1", "nuget");
        assert!(require_registry_type("r1", "nuget", &map).is_ok());
    }

    #[test]
    fn require_registry_type_wrong_type() {
        let map = map_with("r1", "cargo");
        assert!(require_registry_type("r1", "nuget", &map).is_err());
    }

    #[test]
    fn require_registry_type_unknown_registry() {
        let map = RegistryMap::from(HashMap::new());
        assert!(require_registry_type("nonexistent", "nuget", &map).is_err());
    }
}
