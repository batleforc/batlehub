//! Non-fatal configuration problems.
//!
//! Some config states are wrong enough to be worth telling an operator about but
//! not wrong enough to refuse to start: a registry name that cannot become a DNS
//! label, a deprecated key that is being shadowed, a security setting left at its
//! permissive default. A `tracing::warn!` at startup is not good enough for any
//! of them — they are noticed months later, when a hostname mysteriously does not
//! resolve to a registry.
//!
//! [`ConfigWarning`] is the machine-readable form of one such problem, produced
//! by [`AppConfig::warnings`](super::AppConfig::warnings). Warnings are logged at
//! startup *and* on every reload, and served from
//! `GET /api/v1/admin/config/warnings` so the admin UI can render them.

use serde::Serialize;
use utoipa::ToSchema;

/// One non-fatal configuration problem.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct ConfigWarning {
    /// Stable slug identifying the *kind* of problem, e.g.
    /// `"subdomain.invalid-dns-label"`. Safe to match on; the `message` is not.
    pub code: String,
    /// Human-readable explanation, including what the server did instead.
    pub message: String,
    /// Where in the config the problem is, in a form that can be searched for in
    /// the TOML: `"server.trusted_proxies"`, `"registries[3].name"`.
    pub path: String,
}

impl ConfigWarning {
    pub fn new(
        code: impl Into<String>,
        path: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            path: path.into(),
        }
    }
}

// ── Warning codes ─────────────────────────────────────────────────────────────

/// Both `[server].trusted_proxies` and the deprecated
/// `[ip_blocking].trusted_proxies` carry a list; `[server]` wins.
pub const PROXY_TRUST_SHADOWED_DEPRECATED_KEY: &str = "proxy-trust.shadowed-deprecated-key";

/// No trusted-proxy list anywhere, and no host-based routing to force the issue.
/// Forwarded host and scheme are believed unconditionally — today's default, but
/// worth stating explicitly.
pub const PROXY_TRUST_UNCONFIGURED: &str = "proxy-trust.unconfigured";

/// Host-based routing is configured and the trust policy comes only from the
/// deprecated `[ip_blocking].trusted_proxies`.
pub const PROXY_TRUST_DEPRECATED_KEY_ONLY: &str = "proxy-trust.deprecated-key-only";

/// An entry of the deprecated `[ip_blocking].trusted_proxies` is not an IP or a
/// CIDR range. It was silently dropped before this key gained a validator, so it
/// is still dropped (with this warning) rather than refusing to start.
pub const PROXY_TRUST_INVALID_DEPRECATED_ENTRY: &str = "proxy-trust.invalid-deprecated-entry";

/// `[subdomain_routing]` is enabled but a registry name is not a valid DNS
/// label, so no wildcard host is derived for it.
pub const SUBDOMAIN_INVALID_DNS_LABEL: &str = "subdomain.invalid-dns-label";

/// `[server].cors_allowed_origins` contains `"*"`, so any website may issue
/// cross-origin requests to this server and read the responses. Legitimate for a
/// public mirror, rarely what an internal deployment wants — and since 1.1.0 it
/// only happens when someone wrote it down, which is the point of the warning.
pub const CORS_ANY_ORIGIN: &str = "cors.any-origin";
