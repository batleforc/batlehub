use std::future::{ready, Ready};
use std::net::IpAddr;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use actix_web::{
    Error, HttpResponse,
    body::EitherBody,
    dev::{Service, ServiceRequest, ServiceResponse, Transform, forward_ready},
    http::StatusCode,
};
use futures::future::LocalBoxFuture;

use batlehub_config::schema::IpBlockingConfig;
use batlehub_core::ports::IpBlockStore;

/// Extract the client IP. `X-Forwarded-For` is only trusted when the TCP peer
/// address appears in `trusted_proxies`; otherwise the peer address is used
/// directly to prevent spoofed-header bypass.
pub(crate) fn extract_client_ip(req: &ServiceRequest, trusted_proxies: &[String]) -> String {
    let peer_ip: Option<IpAddr> = req.peer_addr().map(|a| a.ip());
    let peer = peer_ip.map(|ip| ip.to_string()).unwrap_or_default();

    let peer_is_trusted = peer_ip.is_some_and(|ip| {
        trusted_proxies
            .iter()
            .filter_map(|t| t.parse::<IpAddr>().ok())
            .any(|t| t == ip)
    });

    if !trusted_proxies.is_empty() && peer_is_trusted {
        if let Some(xff) = req.headers().get("x-forwarded-for") {
            if let Ok(s) = xff.to_str() {
                if let Some(first) = s.split(',').next() {
                    let trimmed = first.trim().to_owned();
                    if !trimmed.is_empty() {
                        return trimmed;
                    }
                }
            }
        }
    }

    if peer.is_empty() { "unknown".to_owned() } else { peer }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

// ── Factory ───────────────────────────────────────────────────────────────────

pub struct IpBlockMiddlewareFactory {
    store: Arc<dyn IpBlockStore>,
    config: Arc<IpBlockingConfig>,
}

impl IpBlockMiddlewareFactory {
    pub fn new(store: Arc<dyn IpBlockStore>, config: IpBlockingConfig) -> Self {
        Self { store, config: Arc::new(config) }
    }
}

impl<S, B> Transform<S, ServiceRequest> for IpBlockMiddlewareFactory
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    B: 'static,
{
    type Response = ServiceResponse<EitherBody<B>>;
    type Error = Error;
    type InitError = ();
    type Transform = IpBlockMiddleware<S>;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(IpBlockMiddleware {
            service: Rc::new(service),
            store: self.store.clone(),
            config: self.config.clone(),
        }))
    }
}

// ── Middleware ────────────────────────────────────────────────────────────────

pub struct IpBlockMiddleware<S> {
    service: Rc<S>,
    store: Arc<dyn IpBlockStore>,
    config: Arc<IpBlockingConfig>,
}

impl<S, B> Service<ServiceRequest> for IpBlockMiddleware<S>
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
        let store = self.store.clone();
        let config = self.config.clone();

        Box::pin(async move {
            let ip = extract_client_ip(&req, &config.trusted_proxies);

            // Check if the IP is already blocked.
            match store.is_blocked(&ip).await {
                Ok(Some(unblock_at)) => {
                    let body = serde_json::json!({
                        "error": "Forbidden",
                        "message": "your IP address has been temporarily blocked",
                    });
                    let http_res = HttpResponse::build(StatusCode::FORBIDDEN)
                        .insert_header(("X-Block-Expires", unblock_at.to_string()))
                        .content_type("application/json")
                        .body(serde_json::to_string(&body).unwrap_or_default());
                    return Ok(req.into_response(http_res).map_into_right_body());
                }
                Ok(None) => {}
                Err(e) => {
                    // Fail-open: log but allow the request through.
                    tracing::warn!(error = %e, ip = %ip, "ip-block store unavailable; failing open");
                }
            }

            let res = service.call(req).await?.map_into_left_body();

            let status = res.status().as_u16();
            if config.trigger_on_status.contains(&status) {
                match store.record_violation(&ip, config.violation_window_secs).await {
                    Ok((count, _)) => {
                        if count >= config.violation_threshold as u64 {
                            let unblock_at = now_unix() + config.ban_duration_secs;
                            if let Err(e) = store.block_ip(&ip, unblock_at, "auto").await {
                                tracing::warn!(error = %e, ip = %ip, "failed to auto-block ip");
                            } else {
                                tracing::info!(ip = %ip, unblock_at, "auto-blocked ip after violation threshold");
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, ip = %ip, "failed to record violation");
                    }
                }
            }

            Ok(res)
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    use actix_web::{
        App, HttpResponse,
        http::StatusCode,
        test::{self, TestRequest},
        web,
    };
    use batlehub_adapters::rate_limit::InMemoryIpBlockStore;
    use batlehub_config::schema::IpBlockingConfig;
    use batlehub_core::ports::IpBlockStore;

    use super::*;

    fn now() -> u64 {
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
    }

    fn default_config() -> IpBlockingConfig {
        IpBlockingConfig {
            enabled: true,
            violation_threshold: 3,
            violation_window_secs: 300,
            ban_duration_secs: 3600,
            trigger_on_status: vec![429, 401],
            trusted_proxies: Vec::new(),
        }
    }

    // ── extract_client_ip ─────────────────────────────────────────────────────

    #[test]
    fn xff_ignored_when_no_trusted_proxies() {
        // Without trusted_proxies, XFF must be ignored — peer addr wins.
        let req = TestRequest::get()
            .peer_addr("10.0.0.99:1234".parse().unwrap())
            .insert_header(("x-forwarded-for", "203.0.113.5, 10.0.0.1"))
            .to_srv_request();
        let ip = extract_client_ip(&req, &[]);
        assert_eq!(ip, "10.0.0.99", "XFF must not be trusted without trusted_proxies");
    }

    #[test]
    fn xff_used_when_peer_is_trusted_proxy() {
        let req = TestRequest::get()
            .peer_addr("10.0.0.1:1234".parse().unwrap())
            .insert_header(("x-forwarded-for", "203.0.113.5, 172.16.0.1"))
            .to_srv_request();
        let trusted = vec!["10.0.0.1".to_owned()];
        assert_eq!(extract_client_ip(&req, &trusted), "203.0.113.5");
    }

    #[test]
    fn xff_single_entry_with_trusted_proxy() {
        let req = TestRequest::get()
            .peer_addr("10.0.0.1:1234".parse().unwrap())
            .insert_header(("x-forwarded-for", "198.51.100.7"))
            .to_srv_request();
        let trusted = vec!["10.0.0.1".to_owned()];
        assert_eq!(extract_client_ip(&req, &trusted), "198.51.100.7");
    }

    #[test]
    fn xff_not_used_when_peer_not_in_trusted_list() {
        let req = TestRequest::get()
            .peer_addr("192.0.2.1:1234".parse().unwrap())
            .insert_header(("x-forwarded-for", "1.1.1.1"))
            .to_srv_request();
        let trusted = vec!["10.0.0.1".to_owned()]; // 192.0.2.1 is not trusted
        assert_eq!(extract_client_ip(&req, &trusted), "192.0.2.1");
    }

    #[test]
    fn ip_falls_back_to_unknown_when_no_peer() {
        let req = TestRequest::get().to_srv_request();
        let ip = extract_client_ip(&req, &[]);
        assert!(!ip.is_empty());
    }

    // ── middleware integration via actix App ──────────────────────────────────

    macro_rules! make_app {
        ($store:expr, $config:expr) => {
            test::init_service(
                App::new()
                    .wrap(IpBlockMiddlewareFactory::new($store, $config))
                    .route("/ok", web::get().to(|| async { HttpResponse::Ok().finish() }))
                    .route("/rate", web::get().to(|| async {
                        HttpResponse::TooManyRequests().finish()
                    }))
                    .route("/auth", web::get().to(|| async {
                        HttpResponse::Unauthorized().finish()
                    })),
            )
            .await
        };
    }

    #[actix_web::test]
    async fn allowed_ip_passes_through() {
        let store = Arc::new(InMemoryIpBlockStore::new());
        let app = make_app!(store, default_config());
        let req = TestRequest::get().uri("/ok").to_request();
        let res = test::call_service(&app, req).await;
        assert_eq!(res.status(), StatusCode::OK);
    }

    #[actix_web::test]
    async fn blocked_ip_gets_403() {
        let store: Arc<dyn IpBlockStore> = Arc::new(InMemoryIpBlockStore::new());
        store.block_ip("198.51.100.1", now() + 3600, "test").await.unwrap();
        let app = make_app!(Arc::clone(&store), default_config());
        let req = TestRequest::get()
            .peer_addr("198.51.100.1:1234".parse().unwrap())
            .uri("/ok")
            .to_request();
        let res = test::call_service(&app, req).await;
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
    }

    #[actix_web::test]
    async fn blocked_ip_response_has_x_block_expires_header() {
        let store: Arc<dyn IpBlockStore> = Arc::new(InMemoryIpBlockStore::new());
        let unblock_at = now() + 3600;
        store.block_ip("203.0.113.1", unblock_at, "test").await.unwrap();
        let app = make_app!(Arc::clone(&store), default_config());
        let req = TestRequest::get()
            .peer_addr("203.0.113.1:1234".parse().unwrap())
            .uri("/ok")
            .to_request();
        let res = test::call_service(&app, req).await;
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
        let hdr = res.headers().get("x-block-expires").unwrap();
        assert_eq!(hdr.to_str().unwrap(), unblock_at.to_string());
    }

    #[actix_web::test]
    async fn auto_block_after_violation_threshold() {
        let store: Arc<dyn IpBlockStore> = Arc::new(InMemoryIpBlockStore::new());
        let config = IpBlockingConfig {
            violation_threshold: 2,
            trigger_on_status: vec![429],
            ..default_config()
        };
        let app = make_app!(Arc::clone(&store), config);

        // First two 429s → violations recorded but threshold not yet reached.
        for _ in 0..2 {
            let req = TestRequest::get()
                .peer_addr("10.0.0.1:1234".parse().unwrap())
                .uri("/rate")
                .to_request();
            let res = test::call_service(&app, req).await;
            assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
        }

        // Now the IP should be auto-blocked.
        assert!(store.is_blocked("10.0.0.1").await.unwrap().is_some());

        // Next request returns 403, not 429.
        let req = TestRequest::get()
            .peer_addr("10.0.0.1:1234".parse().unwrap())
            .uri("/ok")
            .to_request();
        let res = test::call_service(&app, req).await;
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
    }

    #[actix_web::test]
    async fn violation_not_recorded_for_non_trigger_status() {
        let store: Arc<dyn IpBlockStore> = Arc::new(InMemoryIpBlockStore::new());
        let config = IpBlockingConfig {
            violation_threshold: 1,
            trigger_on_status: vec![429],
            ..default_config()
        };
        let app = make_app!(Arc::clone(&store), config);

        // 401 is not in trigger_on_status, so no violation recorded.
        let req = TestRequest::get()
            .peer_addr("10.0.0.2:1234".parse().unwrap())
            .uri("/auth")
            .to_request();
        test::call_service(&app, req).await;

        assert!(store.is_blocked("10.0.0.2").await.unwrap().is_none());
    }

    #[actix_web::test]
    async fn different_ips_are_independent() {
        let store: Arc<dyn IpBlockStore> = Arc::new(InMemoryIpBlockStore::new());
        let config = IpBlockingConfig {
            violation_threshold: 2,
            trigger_on_status: vec![429],
            ..default_config()
        };
        let app = make_app!(Arc::clone(&store), config);

        // Two violations from IP A reach the threshold.
        for _ in 0..2 {
            let req = TestRequest::get()
                .peer_addr("10.0.0.10:1234".parse().unwrap())
                .uri("/rate")
                .to_request();
            test::call_service(&app, req).await;
        }

        // IP B still allowed.
        let req = TestRequest::get()
            .peer_addr("10.0.0.20:1234".parse().unwrap())
            .uri("/ok")
            .to_request();
        let res = test::call_service(&app, req).await;
        assert_eq!(res.status(), StatusCode::OK);
    }
}
