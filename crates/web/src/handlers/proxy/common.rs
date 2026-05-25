use std::sync::Arc;

use actix_web::{HttpRequest, HttpResponse, web};
use bytes::{Bytes, BytesMut};
use futures::StreamExt;

use batlehub_config::schema::RegistryMode;
use batlehub_core::{
    entities::PackageId,
    services::{LocalRegistryService, ProxyRequest, ProxyResponse, ProxyService},
};

use crate::{RegistryModeMap, error::AppError, extractors::AuthIdentity};

/// Decode `X-Artifact-Signature` (base64) and `X-Signature-Type` headers from a request.
///
/// Returns `(signature_bytes, signature_type)`. Either or both may be `None`.
pub fn extract_signature_headers(req: &HttpRequest) -> (Option<Vec<u8>>, Option<String>) {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
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
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    if let Some(meta) = local_svc.get_version_meta(registry, name, version).await {
        if let Some(ref sig) = meta.signature_bytes {
            resp.insert_header(("X-Artifact-Signature", STANDARD.encode(sig)));
        }
        if let Some(ref sig_type) = meta.signature_type {
            resp.insert_header(("X-Signature-Type", sig_type.as_str()));
        }
    }
}

/// Drain an actix streaming body into a contiguous `Bytes` buffer.
pub async fn collect_payload(mut payload: web::Payload) -> Result<Bytes, AppError> {
    let mut raw = BytesMut::new();
    while let Some(chunk) = payload.next().await {
        raw.extend_from_slice(&chunk.map_err(|e| AppError::bad_request(e.to_string()))?);
    }
    Ok(raw.freeze())
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
    };
    match svc.handle(req).await.map_err(AppError::from)? {
        ProxyResponse::Denied { reason } => Err(AppError::forbidden(reason)),
        ProxyResponse::Stream(stream) => {
            let body = stream.filter_map(|chunk| async move {
                chunk.ok().map(Ok::<Bytes, actix_web::Error>)
            });
            let mut resp = HttpResponse::Ok();
            if let Some(ct) = content_type {
                resp.content_type(ct);
            }
            Ok(resp.streaming(body))
        }
    }
}
