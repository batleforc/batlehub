//! Proxy trust — *which* forwarded headers the server believes, and *from whom*.
//!
//! Three signals arrive from a reverse proxy and all three are attacker-settable
//! when the server is exposed directly:
//!
//! | Header                              | Decides                          |
//! |-------------------------------------|----------------------------------|
//! | `Forwarded` / `X-Forwarded-Host`    | the host in every generated URL, and (with host-based routing) which registry serves the request |
//! | `X-Forwarded-Proto`                 | `http` vs `https` in generated URLs |
//! | `X-Forwarded-For`                   | the client IP the fail2ban middleware counts violations against |
//!
//! [`ProxyTrust`] is the configured policy (`[server].trusted_proxies`, falling
//! back to the deprecated `[ip_blocking].trusted_proxies`). It is resolved
//! against the TCP peer **once per request** into a [`PeerTrust`] verdict, which
//! the host-routing middleware stores in the request extensions so the outbound
//! URL helper and the IP-based middlewares all read the same answer instead of
//! each re-deriving it.

use std::net::IpAddr;
use std::sync::Arc;

use actix_web::{dev::ServiceRequest, http::header, web, HttpMessage, HttpRequest};
use ipnet::IpNet;

use batlehub_config::schema::normalise_host;

/// The configured reverse-proxy trust policy, registered as `app_data` and
/// cloned into the host-routing middleware.
///
/// [`Default`] is the legacy-permissive policy, so an app that never registers
/// one (every existing test app, and any deployment with no `trusted_proxies`
/// key) behaves exactly as it did before this policy existed.
#[derive(Clone, Debug, Default)]
pub struct ProxyTrust {
    /// `None` — no list configured anywhere.
    trusted: Option<Arc<Vec<IpNet>>>,
}

impl ProxyTrust {
    /// No policy configured: forwarded host/scheme are believed unconditionally
    /// and `X-Forwarded-For` is ignored. This is what the server did before
    /// `[server].trusted_proxies` existed.
    pub fn legacy_permissive() -> Self {
        Self { trusted: None }
    }

    /// An explicit policy. An empty `networks` list means "trust nobody": every
    /// peer is [`PeerTrust::Untrusted`], so forwarded headers are ignored
    /// entirely.
    pub fn from_networks(networks: Vec<IpNet>) -> Self {
        Self {
            trusted: Some(Arc::new(networks)),
        }
    }

    /// Build from raw config entries, widening bare addresses to `/32` / `/128`.
    /// `None` yields the legacy-permissive policy.
    ///
    /// Malformed entries are already rejected by `AppConfig::validate`; if one
    /// slips through it is dropped with a warning rather than silently widening
    /// the trusted set.
    pub fn from_config(entries: Option<&[String]>) -> Self {
        let Some(entries) = entries else {
            return Self::legacy_permissive();
        };
        match batlehub_config::schema::parse_trusted_proxies(entries) {
            Ok(networks) => Self::from_networks(networks),
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "ignoring malformed trusted_proxies entry; falling back to trusting no peer"
                );
                Self::from_networks(Vec::new())
            }
        }
    }

    /// True when an explicit list is configured (as opposed to legacy permissive).
    pub fn is_configured(&self) -> bool {
        self.trusted.is_some()
    }

    /// Resolve this policy against the request's TCP peer.
    pub fn verdict_for(&self, peer: Option<IpAddr>) -> PeerTrust {
        match &self.trusted {
            None => PeerTrust::LegacyPermissive,
            Some(networks) => {
                if peer.is_some_and(|ip| networks.iter().any(|net| net.contains(&ip))) {
                    PeerTrust::Trusted
                } else {
                    PeerTrust::Untrusted
                }
            }
        }
    }
}

/// Per-request proxy-trust verdict, stored in the request extensions.
///
/// RFC 0001 sketches this as a `PeerTrusted(bool)`, but two booleans' worth of
/// state are actually needed: "absent" and "trusted" agree about the forwarded
/// host and scheme yet disagree about `X-Forwarded-For` (absent must keep
/// ignoring it, or adopting this type would silently make the client IP
/// spoofable for every deployment that has no list). Three variants say that
/// without a second field that can contradict the first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PeerTrust {
    /// No `trusted_proxies` list configured anywhere. Forwarded host/scheme are
    /// believed (today's behaviour, and what keeps generated URLs stable for
    /// existing deployments); `X-Forwarded-For` is not.
    LegacyPermissive,
    /// A list is configured and the TCP peer falls inside it.
    Trusted,
    /// A list is configured and the TCP peer does not fall inside it — including
    /// the `trusted_proxies = []` case, where no peer ever does.
    Untrusted,
}

impl PeerTrust {
    /// Whether `Forwarded` / `X-Forwarded-Host` / `X-Forwarded-Proto` may decide
    /// the client-facing origin.
    pub fn honours_forwarded_origin(self) -> bool {
        !matches!(self, Self::Untrusted)
    }

    /// Whether `X-Forwarded-For` may decide the client IP.
    pub fn honours_forwarded_for(self) -> bool {
        matches!(self, Self::Trusted)
    }
}

/// The verdict for this request.
///
/// Normally the host-routing middleware has already computed and inserted it.
/// When it has not — an app that does not wrap that middleware, i.e. most unit
/// and integration tests — the policy is resolved from `app_data` instead, and
/// failing that the request is treated as legacy permissive. Both fallbacks
/// reproduce the pre-RFC behaviour rather than inventing a stricter one.
pub fn peer_trust(req: &HttpRequest) -> PeerTrust {
    if let Some(verdict) = req.extensions().get::<PeerTrust>().copied() {
        return verdict;
    }
    req.app_data::<web::Data<ProxyTrust>>()
        .map(|trust| trust.verdict_for(req.peer_addr().map(|a| a.ip())))
        .unwrap_or(PeerTrust::LegacyPermissive)
}

/// [`peer_trust`] for middleware, which holds a [`ServiceRequest`].
pub fn peer_trust_of_service(req: &ServiceRequest) -> PeerTrust {
    peer_trust(req.request())
}

/// The client-facing `(scheme, host)` of this request, honouring forwarded
/// headers only when the peer is trusted.
///
/// The host keeps whatever case and port the client sent — this is the origin to
/// render into URLs, not the routing key. Use [`normalise_host`] for lookups.
pub fn trusted_origin(req: &HttpRequest) -> (String, String) {
    if peer_trust(req).honours_forwarded_origin() {
        let conn = req.connection_info();
        (conn.scheme().to_owned(), conn.host().to_owned())
    } else {
        (connection_scheme(req).to_owned(), connection_host(req))
    }
}

/// The scheme of the underlying connection, ignoring `X-Forwarded-Proto`.
fn connection_scheme(req: &HttpRequest) -> &str {
    req.uri().scheme_str().unwrap_or("http")
}

/// The `Host` header (or HTTP/2 `:authority`), ignoring `X-Forwarded-Host`.
///
/// Falls back to the server's own configured host name, which is also what
/// actix's `ConnectionInfo` does when a request carries no host at all.
fn connection_host(req: &HttpRequest) -> String {
    req.headers()
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|host| !host.is_empty())
        .map(str::to_owned)
        .or_else(|| req.uri().authority().map(|a| a.as_str().to_owned()))
        .unwrap_or_else(|| req.app_config().host().to_owned())
}

/// The normalised host this request should be routed on.
///
/// Normalisation lives in `batlehub_config::schema::normalise_host` so the host
/// that validates in the config is exactly the host that routes here.
pub fn routing_host(req: &HttpRequest) -> String {
    normalise_host(&trusted_origin(req).1)
}

/// The client IP: the first `X-Forwarded-For` entry when the peer is trusted,
/// otherwise the TCP peer address.
pub fn client_ip(req: &HttpRequest, trust: PeerTrust) -> String {
    if trust.honours_forwarded_for() {
        if let Some(ip) = first_xff_ip(req) {
            return ip;
        }
    }
    match req.peer_addr() {
        Some(addr) => addr.ip().to_string(),
        None => "unknown".to_owned(),
    }
}

/// The first entry of `X-Forwarded-For`, or `None` when absent or empty.
fn first_xff_ip(req: &HttpRequest) -> Option<String> {
    let xff = req.headers().get("x-forwarded-for")?.to_str().ok()?;
    let first = xff.split(',').next()?.trim();
    if first.is_empty() {
        None
    } else {
        Some(first.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use actix_web::test::TestRequest;

    use super::*;

    fn nets(entries: &[&str]) -> ProxyTrust {
        let owned: Vec<String> = entries.iter().map(|s| (*s).to_owned()).collect();
        ProxyTrust::from_config(Some(&owned))
    }

    // ── verdict ───────────────────────────────────────────────────────────────

    #[test]
    fn absent_list_is_legacy_permissive_for_every_peer() {
        let trust = ProxyTrust::from_config(None);
        assert!(!trust.is_configured());
        assert_eq!(
            trust.verdict_for(Some("203.0.113.7".parse().unwrap())),
            PeerTrust::LegacyPermissive
        );
        assert_eq!(trust.verdict_for(None), PeerTrust::LegacyPermissive);
    }

    #[test]
    fn empty_list_trusts_nobody() {
        let trust = ProxyTrust::from_config(Some(&[]));
        assert!(trust.is_configured());
        assert_eq!(
            trust.verdict_for(Some("10.42.0.1".parse().unwrap())),
            PeerTrust::Untrusted
        );
    }

    #[test]
    fn cidr_contains_peer() {
        let trust = nets(&["10.42.0.0/16"]);
        assert_eq!(
            trust.verdict_for(Some("10.42.7.9".parse().unwrap())),
            PeerTrust::Trusted
        );
    }

    #[test]
    fn peer_just_outside_cidr_is_untrusted() {
        let trust = nets(&["10.42.0.0/16"]);
        assert_eq!(
            trust.verdict_for(Some("10.43.0.1".parse().unwrap())),
            PeerTrust::Untrusted
        );
    }

    #[test]
    fn bare_address_matches_exactly_and_nothing_else() {
        let trust = nets(&["192.168.1.10"]);
        assert_eq!(
            trust.verdict_for(Some("192.168.1.10".parse().unwrap())),
            PeerTrust::Trusted
        );
        assert_eq!(
            trust.verdict_for(Some("192.168.1.11".parse().unwrap())),
            PeerTrust::Untrusted
        );
    }

    #[test]
    fn ipv6_cidr_and_bare_address() {
        let trust = nets(&["2001:db8::/32", "::1"]);
        assert_eq!(
            trust.verdict_for(Some("2001:db8::dead:beef".parse().unwrap())),
            PeerTrust::Trusted
        );
        assert_eq!(
            trust.verdict_for(Some("::1".parse().unwrap())),
            PeerTrust::Trusted
        );
        assert_eq!(
            trust.verdict_for(Some("2001:db9::1".parse().unwrap())),
            PeerTrust::Untrusted
        );
    }

    #[test]
    fn missing_peer_addr_is_untrusted_when_a_list_exists() {
        assert_eq!(
            nets(&["10.0.0.0/8"]).verdict_for(None),
            PeerTrust::Untrusted
        );
    }

    // ── trusted_origin ────────────────────────────────────────────────────────

    fn origin_with(trust: PeerTrust, build: TestRequest) -> (String, String) {
        let req = build.to_http_request();
        req.extensions_mut().insert(trust);
        trusted_origin(&req)
    }

    #[test]
    fn forwarded_headers_decide_origin_for_a_trusted_peer() {
        let (scheme, host) = origin_with(
            PeerTrust::Trusted,
            TestRequest::default()
                .insert_header(("host", "internal.svc:8080"))
                .insert_header(("x-forwarded-host", "npm.acme.io"))
                .insert_header(("x-forwarded-proto", "https")),
        );
        assert_eq!(scheme, "https");
        assert_eq!(host, "npm.acme.io");
    }

    #[test]
    fn forwarded_headers_are_ignored_for_an_untrusted_peer() {
        let (scheme, host) = origin_with(
            PeerTrust::Untrusted,
            TestRequest::default()
                .insert_header(("host", "internal.svc:8080"))
                .insert_header(("x-forwarded-host", "npm.acme.io"))
                .insert_header(("x-forwarded-proto", "https")),
        );
        assert_eq!(scheme, "http", "X-Forwarded-Proto must not be believed");
        assert_eq!(host, "internal.svc:8080");
    }

    #[test]
    fn legacy_permissive_reproduces_todays_unconditional_trust() {
        let (scheme, host) = origin_with(
            PeerTrust::LegacyPermissive,
            TestRequest::default()
                .insert_header(("host", "internal.svc:8080"))
                .insert_header(("x-forwarded-host", "npm.acme.io"))
                .insert_header(("x-forwarded-proto", "https")),
        );
        assert_eq!(scheme, "https");
        assert_eq!(host, "npm.acme.io");
    }

    #[test]
    fn origin_without_any_extension_defaults_to_legacy_permissive() {
        let req = TestRequest::default()
            .insert_header(("x-forwarded-host", "npm.acme.io"))
            .to_http_request();
        assert_eq!(trusted_origin(&req).1, "npm.acme.io");
    }

    #[test]
    fn origin_falls_back_to_the_app_data_policy_when_no_middleware_ran() {
        let req = TestRequest::default()
            .app_data(web::Data::new(ProxyTrust::from_networks(Vec::new())))
            .insert_header(("host", "internal.svc"))
            .insert_header(("x-forwarded-host", "npm.acme.io"))
            .to_http_request();
        assert_eq!(
            trusted_origin(&req).1,
            "internal.svc",
            "the registered policy must apply even without the middleware"
        );
    }

    // ── routing_host ──────────────────────────────────────────────────────────

    #[test]
    fn routing_host_normalises_the_trusted_origin() {
        let req = TestRequest::default()
            .insert_header(("host", "NPM.Acme.io:8443"))
            .to_http_request();
        req.extensions_mut().insert(PeerTrust::Untrusted);
        assert_eq!(routing_host(&req), "npm.acme.io");
    }

    // ── client_ip ─────────────────────────────────────────────────────────────

    #[test]
    fn client_ip_uses_xff_only_for_a_trusted_peer() {
        let build = || {
            TestRequest::default()
                .peer_addr("10.0.0.1:1234".parse().unwrap())
                .insert_header(("x-forwarded-for", "203.0.113.5, 172.16.0.1"))
                .to_http_request()
        };
        assert_eq!(client_ip(&build(), PeerTrust::Trusted), "203.0.113.5");
        assert_eq!(client_ip(&build(), PeerTrust::Untrusted), "10.0.0.1");
        assert_eq!(
            client_ip(&build(), PeerTrust::LegacyPermissive),
            "10.0.0.1",
            "an unconfigured deployment must keep ignoring X-Forwarded-For"
        );
    }

    #[test]
    fn client_ip_falls_back_to_peer_when_xff_is_blank() {
        let req = TestRequest::default()
            .peer_addr("10.0.0.1:1234".parse().unwrap())
            .insert_header(("x-forwarded-for", "  "))
            .to_http_request();
        assert_eq!(client_ip(&req, PeerTrust::Trusted), "10.0.0.1");
    }

    #[test]
    fn client_ip_is_unknown_without_a_peer() {
        let req = TestRequest::default().to_http_request();
        assert_eq!(client_ip(&req, PeerTrust::Untrusted), "unknown");
    }
}
