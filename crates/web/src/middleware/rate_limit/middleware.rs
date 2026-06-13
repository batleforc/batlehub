//! Actix-web `Transform` and `Service` impls for the rate-limit middleware.

use std::future::{ready, Ready};
use std::rc::Rc;
use std::sync::Arc;

use actix_web::{
    body::EitherBody,
    dev::{forward_ready, Service, ServiceRequest, ServiceResponse, Transform},
    http::StatusCode,
    Error, HttpMessage, HttpResponse,
};
use futures::future::LocalBoxFuture;

use batlehub_config::schema::RateLimitEnforcement;
use batlehub_core::entities::Identity;

use super::store::RateLimitService;

/// Extract the registry name from a proxy path like `/proxy/{registry}/...`.
pub fn extract_registry_from_path(path: &str) -> Option<&str> {
    let mut segments = path.splitn(4, '/');
    segments.next(); // leading ""
    let prefix = segments.next()?; // "proxy"
    if prefix != "proxy" {
        return None;
    }
    segments.next() // registry name
}

// ── Middleware factory ────────────────────────────────────────────────────────

/// Actix-web `Transform` factory for the rate-limit middleware.
///
/// Register this last (outermost) so that `AuthMiddleware` runs before it and populates
/// the `Identity` extension that `RateLimitMiddleware` reads to determine the user key.
///
/// ```ignore
/// app.wrap(RateLimitMiddlewareFactory::new(rate_limit_svc.clone()))
///    .wrap(AuthMiddlewareFactory::new(auth_providers.clone()))
/// ```
pub struct RateLimitMiddlewareFactory {
    svc: Arc<RateLimitService>,
}

impl RateLimitMiddlewareFactory {
    /// Create the factory, sharing the given `RateLimitService` across all worker threads.
    pub fn new(svc: Arc<RateLimitService>) -> Self {
        Self { svc }
    }
}

impl<S, B> Transform<S, ServiceRequest> for RateLimitMiddlewareFactory
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    B: 'static,
{
    type Response = ServiceResponse<EitherBody<B>>;
    type Error = Error;
    type InitError = ();
    type Transform = RateLimitMiddleware<S>;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(RateLimitMiddleware {
            service: Rc::new(service),
            svc: self.svc.clone(),
        }))
    }
}

// ── Middleware service ────────────────────────────────────────────────────────

pub struct RateLimitMiddleware<S> {
    service: Rc<S>,
    svc: Arc<RateLimitService>,
}

impl<S, B> Service<ServiceRequest> for RateLimitMiddleware<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    B: 'static,
{
    type Response = ServiceResponse<EitherBody<B>>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    forward_ready!(service);

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let service = self.service.clone();
        let svc = self.svc.clone();

        Box::pin(async move {
            // Only rate-limit proxy routes: /proxy/{registry}/...
            let Some(registry) = extract_registry_from_path(req.path()) else {
                return service.call(req).await.map(|r| r.map_into_left_body());
            };
            let registry = registry.to_owned();

            // Extract user key and group membership from the identity set by auth middleware.
            let (user_key, groups) = {
                let identity = req.extensions().get::<Identity>().cloned();
                match identity {
                    Some(id) => {
                        let key = id.user_id.clone().unwrap_or_else(|| {
                            let addr = req
                                .peer_addr()
                                .map(|a| a.ip().to_string())
                                .unwrap_or_else(|| "unknown".to_owned());
                            format!("ip:{addr}")
                        });
                        (key, id.groups)
                    }
                    None => {
                        let addr = req
                            .peer_addr()
                            .map(|a| a.ip().to_string())
                            .unwrap_or_else(|| "unknown".to_owned());
                        (format!("ip:{addr}"), vec![])
                    }
                }
            };

            match svc.check(&registry, &user_key, &groups).await {
                None => {
                    // No rate limit configured for this registry.
                    service.call(req).await.map(|r| r.map_into_left_body())
                }
                Some(Ok(limit)) => {
                    // Allowed — forward request and annotate response.
                    let mut res = service.call(req).await?.map_into_left_body();
                    res.headers_mut().insert(
                        header_name("x-ratelimit-limit"),
                        header_value(&limit.to_string()),
                    );
                    Ok(res)
                }
                Some(Err((wait, limit, enforcement, reset_unix))) => match enforcement {
                    RateLimitEnforcement::Block => {
                        let retry_after = wait.as_secs().max(1);

                        let body = serde_json::json!({
                            "error": "Too Many Requests",
                            "message": format!("rate limit exceeded; retry after {retry_after}s"),
                        });
                        let http_res = HttpResponse::build(StatusCode::TOO_MANY_REQUESTS)
                            .insert_header(("Retry-After", retry_after.to_string()))
                            .insert_header(("X-RateLimit-Limit", limit.to_string()))
                            .insert_header(("X-RateLimit-Reset", reset_unix.to_string()))
                            .content_type("application/json")
                            .body(serde_json::to_string(&body).unwrap_or_default());

                        Ok(req.into_response(http_res).map_into_right_body())
                    }
                    RateLimitEnforcement::Warn => {
                        let retry_after = wait.as_secs().max(1);
                        let mut res = service.call(req).await?.map_into_left_body();
                        res.headers_mut().insert(
                            header_name("x-ratelimit-warning"),
                            header_value("rate-limit-exceeded"),
                        );
                        res.headers_mut().insert(
                            header_name("x-ratelimit-limit"),
                            header_value(&limit.to_string()),
                        );
                        res.headers_mut().insert(
                            header_name("retry-after"),
                            header_value(&retry_after.to_string()),
                        );
                        Ok(res)
                    }
                },
            }
        })
    }
}

fn header_name(s: &'static str) -> actix_web::http::header::HeaderName {
    actix_web::http::header::HeaderName::from_static(s)
}

fn header_value(s: &str) -> actix_web::http::header::HeaderValue {
    actix_web::http::header::HeaderValue::from_str(s)
        .unwrap_or(actix_web::http::header::HeaderValue::from_static("0"))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_registry_proxy_path() {
        assert_eq!(
            extract_registry_from_path("/proxy/myregistry/v1/foo"),
            Some("myregistry")
        );
        assert_eq!(
            extract_registry_from_path("/proxy/npm/package/-/tarball"),
            Some("npm")
        );
    }

    #[test]
    fn extract_registry_non_proxy_path() {
        assert_eq!(extract_registry_from_path("/api/v1/admin/quota"), None);
        assert_eq!(extract_registry_from_path("/healthz"), None);
        assert_eq!(extract_registry_from_path("/metrics"), None);
    }
}
