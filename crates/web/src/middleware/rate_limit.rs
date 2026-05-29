//! Actix-web middleware that enforces per-registry rate limits.
//!
//! # Architecture
//!
//! ```text
//! RateLimitMiddlewareFactory
//!     └── wraps every incoming ServiceRequest
//!         ├── extracts the registry name from the path (/proxy/{registry}/…)
//!         ├── extracts the user key and group membership from the Identity extension
//!         │   (set earlier by AuthMiddleware)
//!         └── delegates to RateLimitService::check()
//!             ├── None  → no rate-limit config for this registry, pass through
//!             ├── Ok(limit)  → allowed, annotate response with X-RateLimit-Limit
//!             └── Err(wait, limit, enforcement, reset_unix)
//!                 ├── Block → 429 with Retry-After / X-RateLimit-Reset headers
//!                 └── Warn  → forward but add X-RateLimit-Warning header
//! ```
//!
//! # Middleware registration order
//!
//! Actix-web wraps are applied in reverse registration order (last `.wrap()` = outermost =
//! first to execute). `AuthMiddleware` **must** be registered **after**
//! `RateLimitMiddlewareFactory` so that it runs first and populates `Identity` before
//! the rate-limit check reads it.

use std::collections::HashMap;
use std::future::{ready, Ready};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use actix_web::{
    body::EitherBody,
    dev::{forward_ready, Service, ServiceRequest, ServiceResponse, Transform},
    http::StatusCode,
    Error, HttpMessage, HttpResponse,
};
use futures::future::LocalBoxFuture;

use batlehub_config::schema::{RateLimitConfig, RateLimitEnforcement};
use batlehub_core::{entities::Identity, ports::RateLimitStore};

// ── Public service ────────────────────────────────────────────────────────────

/// Distributed rate limiter backed by a pluggable `RateLimitStore`.
///
/// Holds per-registry configuration (limits, windows, group overrides).
/// State is persisted in the configured store (InMemory / Postgres / Redis),
/// so limits survive restarts and are shared across multiple instances when
/// using a shared backend.
pub struct RateLimitService {
    configs: HashMap<String, RateLimitConfig>,
    store: Arc<dyn RateLimitStore>,
}

impl RateLimitService {
    pub fn new(configs: &HashMap<String, RateLimitConfig>, store: Arc<dyn RateLimitStore>) -> Self {
        Self {
            configs: configs.clone(),
            store,
        }
    }

    /// Check and consume one request token for the given user and groups in `registry`.
    ///
    /// Returns:
    /// - `None` — no rate limit configured for this registry
    /// - `Some(Ok(limit))` — request allowed; `limit` is the binding ceiling
    /// - `Some(Err((wait, limit, enforcement, reset_unix)))` — rate limited by the most-restrictive check;
    ///   `reset_unix` is the exact Unix timestamp when the violating window resets
    ///
    /// **Multi-limiter semantics:** the user bucket AND all applicable group buckets are checked.
    /// All relevant buckets are incremented; the request is blocked if any bucket exceeds its limit.
    /// On failure the `Err` contains the worst violation (Block beats Warn; longer wait wins).
    ///
    /// **Fail-open design:** if the backing store returns an error, the affected bucket is skipped
    /// and the request is allowed rather than refused. This prevents the rate-limit store becoming
    /// a hard dependency that takes down the proxy when unavailable.
    pub async fn check(
        &self,
        registry: &str,
        user_key: &str,
        user_groups: &[String],
    ) -> Option<Result<u32, (Duration, u32, RateLimitEnforcement, u64)>> {
        let cfg = self.configs.get(registry)?;

        let user_store_key = format!("rl:{registry}:user:{user_key}");
        // Fail-open: if the store is unavailable, skip rate limiting rather than blocking the request.
        let (user_count, user_reset) =
            match self.store.increment(&user_store_key, cfg.window_secs).await {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        registry = %registry,
                        "rate-limit store unavailable for user bucket; failing open"
                    );
                    return None;
                }
            };

        let mut binding_limit = cfg.requests_per_window;
        let mut worst: Option<(Duration, u32, RateLimitEnforcement, u64)> = None;

        if user_count > cfg.requests_per_window as u64 {
            let wait = wait_from_reset(user_reset);
            worst = Some((
                wait,
                cfg.requests_per_window,
                cfg.enforcement.clone(),
                user_reset,
            ));
        }

        for group in user_groups {
            let Some(grp) = cfg.groups.iter().find(|g| &g.name == group) else {
                continue;
            };
            binding_limit = binding_limit.min(grp.requests_per_window);

            let group_store_key = format!("rl:{registry}:group:{group}");
            // Fail-open: skip this group bucket on store error.
            let (grp_count, grp_reset) = match self
                .store
                .increment(&group_store_key, grp.window_secs)
                .await
            {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        registry = %registry,
                        group = %group,
                        "rate-limit store unavailable for group bucket; failing open"
                    );
                    continue;
                }
            };

            if grp_count > grp.requests_per_window as u64 {
                let effective = grp
                    .enforcement
                    .clone()
                    .unwrap_or_else(|| cfg.enforcement.clone());
                let wait = wait_from_reset(grp_reset);
                worst = Some(merge_failure(
                    worst,
                    (wait, grp.requests_per_window, effective, grp_reset),
                ));
            }
        }

        match worst {
            None => Some(Ok(binding_limit)),
            Some(failure) => Some(Err(failure)),
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Compute the wait duration from a window-reset Unix timestamp.
fn wait_from_reset(reset_unix: u64) -> Duration {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    Duration::from_secs(reset_unix.saturating_sub(now).max(1))
}

/// Return the "worse" of two rate-limit failures.
///
/// `Block` enforcement beats `Warn`; among equal enforcement modes, the longer
/// wait time wins (the client must wait longer before retrying).
fn merge_failure(
    a: Option<(Duration, u32, RateLimitEnforcement, u64)>,
    b: (Duration, u32, RateLimitEnforcement, u64),
) -> (Duration, u32, RateLimitEnforcement, u64) {
    let Some(a) = a else { return b };
    let a_blocks = matches!(a.2, RateLimitEnforcement::Block);
    let b_blocks = matches!(b.2, RateLimitEnforcement::Block);
    match (a_blocks, b_blocks) {
        (true, false) => a,
        (false, true) => b,
        _ => {
            if b.0 > a.0 {
                b
            } else {
                a
            }
        }
    }
}

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
    use batlehub_adapters::rate_limit::InMemoryRateLimitStore;
    use batlehub_config::schema::GroupRateLimitConfig;

    fn svc_from(registry: &str, cfg: RateLimitConfig) -> RateLimitService {
        let mut m = HashMap::new();
        m.insert(registry.to_owned(), cfg);
        let store = Arc::new(InMemoryRateLimitStore::new());
        RateLimitService::new(&m, store)
    }

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

    #[tokio::test]
    async fn user_allowed_within_limit() {
        let svc = svc_from(
            "r",
            RateLimitConfig {
                requests_per_window: 10,
                window_secs: 60,
                enforcement: RateLimitEnforcement::Block,
                groups: vec![],
            },
        );
        for _ in 0..10 {
            assert!(svc
                .check("r", "u1", &[])
                .await
                .map(|r| r.is_ok())
                .unwrap_or(false));
        }
    }

    #[tokio::test]
    async fn user_blocked_after_limit() {
        let svc = svc_from(
            "r",
            RateLimitConfig {
                requests_per_window: 2,
                window_secs: 60,
                enforcement: RateLimitEnforcement::Block,
                groups: vec![],
            },
        );
        assert!(svc
            .check("r", "u1", &[])
            .await
            .map(|r| r.is_ok())
            .unwrap_or(false));
        assert!(svc
            .check("r", "u1", &[])
            .await
            .map(|r| r.is_ok())
            .unwrap_or(false));
        assert!(svc
            .check("r", "u1", &[])
            .await
            .map(|r| r.is_err())
            .unwrap_or(false));
    }

    #[tokio::test]
    async fn user_buckets_are_independent() {
        let svc = svc_from(
            "r",
            RateLimitConfig {
                requests_per_window: 1,
                window_secs: 60,
                enforcement: RateLimitEnforcement::Block,
                groups: vec![],
            },
        );
        assert!(svc
            .check("r", "u1", &[])
            .await
            .map(|r| r.is_ok())
            .unwrap_or(false));
        // Different user still allowed.
        assert!(svc
            .check("r", "u2", &[])
            .await
            .map(|r| r.is_ok())
            .unwrap_or(false));
        // u1 is blocked.
        assert!(svc
            .check("r", "u1", &[])
            .await
            .map(|r| r.is_err())
            .unwrap_or(false));
    }

    #[tokio::test]
    async fn no_config_returns_none() {
        let store = Arc::new(InMemoryRateLimitStore::new());
        let svc = RateLimitService::new(&HashMap::new(), store);
        assert!(svc.check("r", "u1", &[]).await.is_none());
    }

    #[tokio::test]
    async fn group_bucket_shared_by_members() {
        let svc = svc_from(
            "r",
            RateLimitConfig {
                requests_per_window: 100,
                window_secs: 60,
                enforcement: RateLimitEnforcement::Block,
                groups: vec![GroupRateLimitConfig {
                    name: "ci-bots".to_owned(),
                    requests_per_window: 2,
                    window_secs: 60,
                    enforcement: None,
                }],
            },
        );

        let groups = vec!["ci-bots".to_owned()];
        // Both bot1 and bot2 draw from the shared "ci-bots" pool (limit = 2).
        assert!(svc
            .check("r", "bot1", &groups)
            .await
            .map(|r| r.is_ok())
            .unwrap_or(false));
        assert!(svc
            .check("r", "bot2", &groups)
            .await
            .map(|r| r.is_ok())
            .unwrap_or(false));
        // Pool exhausted — third request (from any group member) is blocked.
        assert!(svc
            .check("r", "bot3", &groups)
            .await
            .map(|r| r.is_err())
            .unwrap_or(false));
    }

    #[tokio::test]
    async fn non_group_member_not_affected_by_group_limit() {
        let svc = svc_from(
            "r",
            RateLimitConfig {
                requests_per_window: 100,
                window_secs: 60,
                enforcement: RateLimitEnforcement::Block,
                groups: vec![GroupRateLimitConfig {
                    name: "ci-bots".to_owned(),
                    requests_per_window: 1,
                    window_secs: 60,
                    enforcement: None,
                }],
            },
        );

        // Exhaust the ci-bots pool.
        let bot_groups = vec!["ci-bots".to_owned()];
        assert!(svc
            .check("r", "bot1", &bot_groups)
            .await
            .map(|r| r.is_ok())
            .unwrap_or(false));
        assert!(svc
            .check("r", "bot1", &bot_groups)
            .await
            .map(|r| r.is_err())
            .unwrap_or(false));

        // A user not in ci-bots is unaffected by the group limit.
        assert!(svc
            .check("r", "regular-user", &[])
            .await
            .map(|r| r.is_ok())
            .unwrap_or(false));
    }

    #[tokio::test]
    async fn user_and_group_both_checked() {
        // User limit = 3; group limit = 1.
        let svc = svc_from(
            "r",
            RateLimitConfig {
                requests_per_window: 3,
                window_secs: 60,
                enforcement: RateLimitEnforcement::Block,
                groups: vec![GroupRateLimitConfig {
                    name: "g".to_owned(),
                    requests_per_window: 1,
                    window_secs: 60,
                    enforcement: None,
                }],
            },
        );
        let groups = vec!["g".to_owned()];
        // First request OK — both user bucket and group bucket have tokens.
        assert!(svc
            .check("r", "u1", &groups)
            .await
            .map(|r| r.is_ok())
            .unwrap_or(false));
        // Second request: group bucket exhausted (limit=1), blocks even though user bucket still has tokens.
        assert!(svc
            .check("r", "u1", &groups)
            .await
            .map(|r| r.is_err())
            .unwrap_or(false));
    }

    #[tokio::test]
    async fn group_enforcement_overrides_parent() {
        let svc = svc_from(
            "r",
            RateLimitConfig {
                requests_per_window: 10,
                window_secs: 60,
                enforcement: RateLimitEnforcement::Block, // parent = block
                groups: vec![GroupRateLimitConfig {
                    name: "vip".to_owned(),
                    requests_per_window: 2,
                    window_secs: 60,
                    enforcement: Some(RateLimitEnforcement::Warn), // group overrides to warn
                }],
            },
        );
        let groups = vec!["vip".to_owned()];
        svc.check("r", "u1", &groups).await.unwrap().ok();
        svc.check("r", "u1", &groups).await.unwrap().ok();
        // Group bucket exhausted — but enforcement is Warn, so the error carries Warn.
        let result = svc.check("r", "u1", &groups).await.unwrap();
        assert!(result.is_err());
        let (_, _, enforcement, _) = result.unwrap_err();
        assert_eq!(enforcement, RateLimitEnforcement::Warn);
    }

    #[test]
    fn merge_failure_block_beats_warn() {
        let warn = (
            Duration::from_secs(10),
            100u32,
            RateLimitEnforcement::Warn,
            0u64,
        );
        let block = (
            Duration::from_secs(5),
            50u32,
            RateLimitEnforcement::Block,
            0u64,
        );

        let (_, _, e, _) = merge_failure(Some(warn.clone()), block.clone());
        assert_eq!(e, RateLimitEnforcement::Block);

        let (_, _, e, _) = merge_failure(Some(block.clone()), warn.clone());
        assert_eq!(e, RateLimitEnforcement::Block);
    }

    #[test]
    fn merge_failure_longer_wait_wins() {
        let short = (
            Duration::from_secs(5),
            100u32,
            RateLimitEnforcement::Block,
            0u64,
        );
        let long = (
            Duration::from_secs(30),
            50u32,
            RateLimitEnforcement::Block,
            0u64,
        );

        let (wait, _, _, _) = merge_failure(Some(short), long);
        assert_eq!(wait, Duration::from_secs(30));
    }
}
