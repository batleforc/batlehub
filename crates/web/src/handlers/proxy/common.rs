use std::sync::Arc;

use actix_web::{HttpResponse, web};
use bytes::Bytes;
use futures::StreamExt;

use proxy_cache_core::{
    entities::PackageId,
    services::{ProxyRequest, ProxyResponse, ProxyService},
};

use crate::{error::AppError, extractors::AuthIdentity};

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
