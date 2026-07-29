//! Proxy trust — who is allowed to tell us the client's host, scheme and IP
//! (RFC 0001 §4.5).
//!
//! Three forwarded signals reach us from a reverse proxy, and until now they
//! followed two different rules:
//!
//! | Signal | Header | Rule before this module |
//! | --- | --- | --- |
//! | client IP | `X-Forwarded-For` | trusted only from a listed peer |
//! | host | `Forwarded` / `X-Forwarded-Host` | trusted unconditionally |
//! | scheme | `X-Forwarded-Proto` | trusted unconditionally |
//!
//! [`ProxyTrustMiddleware`] computes the verdict **once per request**, as the
//! outermost layer, and stores it as a [`PeerTrusted`] request extension. Every
//! consumer — the IP-block middleware, the inbound-webhook handler, and the
//! base-URL helpers behind [`trusted_origin`] — reads that one verdict instead of
//! re-deriving it, so all three signals agree within a request.
//!
//! ## Absent is not empty
//!
//! [`ProxyTrust::new`] takes an `Option`, and the distinction survives into
//! [`PeerTrusted`]:
//!
//! - **absent** — no list in either config key. Forwarded host/scheme stay
//!   trusted (that is what every generated URL already did, and tightening it by
//!   default would silently start advertising internal service hosts), while
//!   `X-Forwarded-For` stays ignored (that was already fail-closed). This
//!   reproduces today's behaviour byte for byte.
//! - `Some([])` — trust nobody: forwarded headers are ignored entirely.
//! - `Some(nets)` — honoured only from peers inside those ranges.
//!
//! When the middleware has not run at all — unit tests, integration apps built
//! without it — the extension is missing and consumers fall back to
//! [`PeerTrusted::LEGACY`], i.e. the absent case.

use std::future::{ready, Ready};
use std::net::IpAddr;
use std::rc::Rc;
use std::sync::Arc;

use actix_web::{
    dev::{forward_ready, Service, ServiceRequest, ServiceResponse, Transform},
    http::header,
    Error, HttpMessage, HttpRequest,
};
use futures::future::LocalBoxFuture;
use ipnet::IpNet;

/// The parsed `trusted_proxies` list, shared by the middleware for the lifetime
/// of the server.
///
/// `None` is the legacy state — see the module docs.
#[derive(Debug, Clone, Default)]
pub struct ProxyTrust {
    // `Arc` because the factory is captured by the `HttpServer::new` closure,
    // which is cloned onto every worker thread.
    trusted: Option<Arc<Vec<IpNet>>>,
}

impl ProxyTrust {
    /// Build from the raw config entries, already validated by
    /// `AppConfig::validate`. An entry that somehow still fails to parse is
    /// dropped with a warning rather than taking the server down at request
    /// time — it can only ever make the set *smaller*, i.e. fail closed.
    pub fn new(entries: Option<&[String]>) -> Self {
        let trusted = entries.map(|entries| {
            let nets = entries
                .iter()
                .filter_map(|e| match batlehub_config::trusted_proxies::parse_entry(e) {
                    Ok(net) => Some(net),
                    Err(err) => {
                        tracing::warn!(entry = %e, error = %err, "ignoring invalid trusted_proxies entry");
                        None
                    }
                })
                .collect();
            Arc::new(nets)
        });
        Self { trusted }
    }

    /// The legacy state: no list configured in either key.
    pub fn legacy() -> Self {
        Self { trusted: None }
    }

    /// Evaluate one peer against the configured set.
    pub fn verdict(&self, peer: Option<IpAddr>) -> PeerTrusted {
        match &self.trusted {
            None => PeerTrusted::LEGACY,
            Some(nets) => PeerTrusted {
                configured: true,
                trusted: peer.is_some_and(|ip| nets.iter().any(|n| n.contains(&ip))),
            },
        }
    }
}

/// Per-request proxy-trust verdict, stored in the request extensions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeerTrusted {
    /// A `trusted_proxies` list was configured at all — possibly the empty one.
    pub configured: bool,
    /// The TCP peer falls inside that list. Only meaningful when `configured`.
    pub trusted: bool,
}

impl PeerTrusted {
    /// What consumers assume when no list is configured, or when the middleware
    /// did not run.
    pub const LEGACY: Self = Self {
        configured: false,
        trusted: false,
    };

    /// May `Forwarded` / `X-Forwarded-Host` / `X-Forwarded-Proto` decide the
    /// client-facing origin?
    ///
    /// Permissive when nothing is configured — that is the behaviour every
    /// generated URL has had, and changing it silently would break deployments
    /// that never opted in.
    pub fn origin_headers(&self) -> bool {
        !self.configured || self.trusted
    }

    /// May `X-Forwarded-For` decide the client IP?
    ///
    /// Strict when nothing is configured: an unlisted peer must never be able to
    /// pick its own identity for rate limiting or IP blocking.
    pub fn client_ip_header(&self) -> bool {
        self.configured && self.trusted
    }
}

/// Read the verdict computed for this request, or [`PeerTrusted::LEGACY`] when
/// the middleware did not run.
pub fn peer_trusted(req: &impl HttpMessage) -> PeerTrusted {
    req.extensions()
        .get::<PeerTrusted>()
        .copied()
        .unwrap_or(PeerTrusted::LEGACY)
}

/// The client-facing `(scheme, host)` of this request.
///
/// This is the trust-aware replacement for `req.connection_info()`, which
/// honours `X-Forwarded-Host` / `X-Forwarded-Proto` from anyone. When the peer
/// is not trusted, the `Host` header and the listener's own TLS state decide,
/// so a forged header cannot appear in a generated (and then cached) URL.
pub fn trusted_origin(req: &HttpRequest) -> (String, String) {
    if peer_trusted(req).origin_headers() {
        let info = req.connection_info();
        return (info.scheme().to_owned(), info.host().to_owned());
    }

    let scheme = if req.app_config().secure() {
        "https"
    } else {
        "http"
    };
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .map(str::trim)
        .filter(|h| !h.is_empty())
        .map(str::to_owned)
        .or_else(|| req.uri().authority().map(|a| a.to_string()))
        .unwrap_or_else(|| req.app_config().host().to_owned());

    (scheme.to_owned(), host)
}

/// The client-facing origin as a single `scheme://host` string — the shape every
/// base-URL helper wants.
pub fn trusted_base_url(req: &HttpRequest) -> String {
    let (scheme, host) = trusted_origin(req);
    format!("{scheme}://{host}")
}

/// The first `X-Forwarded-For` entry, or `None` when absent/empty.
fn first_xff_ip(req: &impl HttpMessage) -> Option<String> {
    let headers = req.headers();
    let xff = headers.get("x-forwarded-for")?.to_str().ok()?;
    let first = xff.split(',').next()?.trim();
    (!first.is_empty()).then(|| first.to_owned())
}

/// The client IP for this request, honouring `X-Forwarded-For` only from a
/// trusted peer.
///
/// `peer` is the TCP peer address, which is not reachable through
/// [`HttpMessage`] — callers pass `req.peer_addr().map(|a| a.ip())`, which both
/// `ServiceRequest` and `HttpRequest` provide.
///
/// Returns `None` only when the peer is unknown *and* no trusted header supplied
/// one; callers decide what to record in that case.
pub fn trusted_client_ip(req: &impl HttpMessage, peer: Option<IpAddr>) -> Option<String> {
    if peer_trusted(req).client_ip_header() {
        if let Some(ip) = first_xff_ip(req) {
            return Some(ip);
        }
    }
    peer.map(|ip| ip.to_string())
}

// ── Factory ───────────────────────────────────────────────────────────────────

/// Wraps the app so every request carries a [`PeerTrusted`] verdict.
///
/// Must be the **outermost** layer: the IP-block middleware and every handler
/// below it read the extension this sets.
pub struct ProxyTrustMiddlewareFactory {
    trust: ProxyTrust,
}

impl ProxyTrustMiddlewareFactory {
    pub fn new(trust: ProxyTrust) -> Self {
        Self { trust }
    }
}

impl<S, B> Transform<S, ServiceRequest> for ProxyTrustMiddlewareFactory
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type InitError = ();
    type Transform = ProxyTrustMiddleware<S>;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(ProxyTrustMiddleware {
            service: Rc::new(service),
            trust: self.trust.clone(),
        }))
    }
}

// ── Middleware ────────────────────────────────────────────────────────────────

pub struct ProxyTrustMiddleware<S> {
    service: Rc<S>,
    trust: ProxyTrust,
}

impl<S, B> Service<ServiceRequest> for ProxyTrustMiddleware<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    forward_ready!(service);

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let verdict = self.trust.verdict(req.peer_addr().map(|a| a.ip()));
        req.extensions_mut().insert(verdict);

        let service = self.service.clone();
        Box::pin(async move { service.call(req).await })
    }
}

#[cfg(test)]
mod tests {
    use actix_web::{test::TestRequest, web, App, HttpResponse};

    use super::*;

    fn list(entries: &[&str]) -> Vec<String> {
        entries.iter().map(|s| (*s).to_owned()).collect()
    }

    fn peer(addr: &str) -> Option<IpAddr> {
        Some(addr.parse().unwrap())
    }

    // ── verdict ───────────────────────────────────────────────────────────────

    #[test]
    fn absent_list_is_the_legacy_verdict() {
        let v = ProxyTrust::legacy().verdict(peer("10.0.0.1"));
        assert_eq!(v, PeerTrusted::LEGACY);
        assert!(v.origin_headers(), "forwarded host/scheme stay trusted");
        assert!(!v.client_ip_header(), "X-Forwarded-For stays ignored");
    }

    #[test]
    fn empty_list_trusts_nobody() {
        let v = ProxyTrust::new(Some(&[])).verdict(peer("10.0.0.1"));
        assert!(v.configured);
        assert!(!v.trusted);
        assert!(!v.origin_headers());
        assert!(!v.client_ip_header());
    }

    #[test]
    fn a_listed_peer_is_trusted_for_all_three_signals() {
        let v = ProxyTrust::new(Some(&list(&["10.42.0.0/16"]))).verdict(peer("10.42.7.9"));
        assert!(v.origin_headers());
        assert!(v.client_ip_header());
    }

    #[test]
    fn an_unlisted_peer_is_trusted_for_nothing() {
        let v = ProxyTrust::new(Some(&list(&["10.42.0.0/16"]))).verdict(peer("10.43.0.1"));
        assert!(!v.origin_headers());
        assert!(!v.client_ip_header());
    }

    #[test]
    fn a_bare_entry_still_matches_exactly() {
        let trust = ProxyTrust::new(Some(&list(&["10.0.0.1"])));
        assert!(trust.verdict(peer("10.0.0.1")).trusted);
        assert!(!trust.verdict(peer("10.0.0.2")).trusted);
    }

    #[test]
    fn a_request_without_a_peer_is_never_trusted() {
        let v = ProxyTrust::new(Some(&list(&["10.0.0.0/8"]))).verdict(None);
        assert!(!v.trusted);
    }

    #[test]
    fn invalid_entries_are_dropped_rather_than_widening_the_set() {
        // Validation happens at config load; if one still slips through it must
        // fail closed.
        let trust = ProxyTrust::new(Some(&list(&["not-an-ip", "10.0.0.1"])));
        assert!(trust.verdict(peer("10.0.0.1")).trusted);
        assert!(!trust.verdict(peer("192.0.2.1")).trusted);
    }

    // ── trusted_origin ────────────────────────────────────────────────────────

    /// Build a request carrying the verdict for `trusted`, as the middleware would.
    fn origin_for(trusted: Option<&[String]>, peer_addr: &str, headers: &[(&str, &str)]) -> String {
        let mut builder = TestRequest::get().peer_addr(peer_addr.parse().unwrap());
        for (k, v) in headers {
            builder = builder.insert_header((*k, *v));
        }
        let req = builder.to_http_request();
        let verdict = ProxyTrust::new(trusted).verdict(peer(peer_addr.rsplit_once(':').unwrap().0));
        req.extensions_mut().insert(verdict);
        trusted_base_url(&req)
    }

    #[test]
    fn forwarded_host_is_honoured_from_a_trusted_peer() {
        let url = origin_for(
            Some(&list(&["10.0.0.1"])),
            "10.0.0.1:1234",
            &[
                ("host", "internal.svc:8080"),
                ("x-forwarded-host", "hub.example.com"),
                ("x-forwarded-proto", "https"),
            ],
        );
        assert_eq!(url, "https://hub.example.com");
    }

    #[test]
    fn forwarded_host_is_ignored_from_an_untrusted_peer() {
        // The spoofing case: a forged header must not end up in a generated —
        // and then cached — URL.
        let url = origin_for(
            Some(&list(&["10.0.0.1"])),
            "192.0.2.66:1234",
            &[
                ("host", "hub.example.com"),
                ("x-forwarded-host", "evil.example.net"),
                ("x-forwarded-proto", "https"),
            ],
        );
        assert_eq!(url, "http://hub.example.com");
    }

    #[test]
    fn forwarded_host_is_ignored_when_the_list_is_empty() {
        let url = origin_for(
            Some(&[]),
            "10.0.0.1:1234",
            &[
                ("host", "hub.example.com"),
                ("x-forwarded-host", "evil.example.net"),
            ],
        );
        assert_eq!(url, "http://hub.example.com");
    }

    #[test]
    fn absent_list_reproduces_todays_behaviour() {
        let url = origin_for(
            None,
            "192.0.2.66:1234",
            &[
                ("host", "internal.svc:8080"),
                ("x-forwarded-host", "hub.example.com"),
                ("x-forwarded-proto", "https"),
            ],
        );
        assert_eq!(url, "https://hub.example.com");
    }

    #[test]
    fn untrusted_peer_falls_back_to_the_host_header() {
        let url = origin_for(Some(&[]), "192.0.2.66:1234", &[("host", "batlehub.local")]);
        assert_eq!(url, "http://batlehub.local");
    }

    #[test]
    fn missing_extension_behaves_like_no_configuration() {
        // Apps built without the middleware keep generating the URLs they did.
        let req = TestRequest::get()
            .peer_addr("192.0.2.66:1234".parse().unwrap())
            .insert_header(("host", "internal.svc"))
            .insert_header(("x-forwarded-host", "hub.example.com"))
            .to_http_request();
        assert_eq!(trusted_base_url(&req), "http://hub.example.com");
    }

    // ── trusted_client_ip ─────────────────────────────────────────────────────

    #[test]
    fn client_ip_prefers_xff_only_from_a_trusted_peer() {
        let req = TestRequest::get()
            .peer_addr("10.0.0.1:1234".parse().unwrap())
            .insert_header(("x-forwarded-for", "203.0.113.5, 172.16.0.1"))
            .to_http_request();
        req.extensions_mut()
            .insert(ProxyTrust::new(Some(&list(&["10.0.0.1"]))).verdict(peer("10.0.0.1")));
        assert_eq!(
            trusted_client_ip(&req, peer("10.0.0.1")),
            Some("203.0.113.5".to_owned())
        );
    }

    #[test]
    fn client_ip_is_the_peer_when_the_list_is_absent() {
        let req = TestRequest::get()
            .peer_addr("10.0.0.1:1234".parse().unwrap())
            .insert_header(("x-forwarded-for", "203.0.113.5"))
            .to_http_request();
        req.extensions_mut()
            .insert(ProxyTrust::legacy().verdict(peer("10.0.0.1")));
        assert_eq!(
            trusted_client_ip(&req, peer("10.0.0.1")),
            Some("10.0.0.1".to_owned())
        );
    }

    #[test]
    fn client_ip_is_none_without_a_peer_or_a_trusted_header() {
        let req = TestRequest::get().to_http_request();
        assert_eq!(trusted_client_ip(&req, None), None);
    }

    // ── middleware ────────────────────────────────────────────────────────────

    #[actix_web::test]
    async fn middleware_inserts_the_verdict_for_downstream_layers() {
        let app = actix_web::test::init_service(
            App::new()
                .wrap(ProxyTrustMiddlewareFactory::new(ProxyTrust::new(Some(
                    &list(&["10.0.0.0/8"]),
                ))))
                .route(
                    "/who",
                    web::get().to(|req: HttpRequest| async move {
                        let v = peer_trusted(&req);
                        HttpResponse::Ok().body(format!("{}/{}", v.configured, v.trusted))
                    }),
                ),
        )
        .await;

        let req = TestRequest::get()
            .uri("/who")
            .peer_addr("10.1.2.3:1234".parse().unwrap())
            .to_request();
        let body = actix_web::test::call_and_read_body(&app, req).await;
        assert_eq!(body, "true/true".as_bytes());

        let req = TestRequest::get()
            .uri("/who")
            .peer_addr("192.0.2.1:1234".parse().unwrap())
            .to_request();
        let body = actix_web::test::call_and_read_body(&app, req).await;
        assert_eq!(body, "true/false".as_bytes());
    }
}
