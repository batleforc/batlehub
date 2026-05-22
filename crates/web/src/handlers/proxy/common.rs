use std::sync::Arc;

use actix_web::{HttpResponse, web};
use bytes::{Bytes, BytesMut};
use futures::StreamExt;

use batlehub_config::schema::RegistryMode;
use batlehub_core::{
    entities::PackageId,
    services::{ProxyRequest, ProxyResponse, ProxyService},
};

use crate::{RegistryModeMap, error::AppError, extractors::AuthIdentity};

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
