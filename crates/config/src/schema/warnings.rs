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

/// A registry has `signed_downloads = true` *and* still grants the anonymous
/// read that signing exists to remove. Legal — belt and braces — but almost
/// certainly a migration someone stopped halfway: the signed URLs are minted
/// and verified, and the registry is open to everyone anyway, so nothing is
/// actually closed (RFC 0012 §7).
pub const SIGNED_URLS_ANONYMOUS_STILL_GRANTED: &str = "signed-urls.anonymous-still-granted";

/// `[server.signed_urls]` is configured and no registry sets
/// `signed_downloads = true`, so the secret signs nothing. Harmless, and worth
/// saying: it is the shape of a feature enabled on the wrong side.
pub const SIGNED_URLS_UNUSED: &str = "signed-urls.unused";

/// A `license_gate` rule is configured on a registry whose type has no manifest
/// parser, so the licence of every version is permanently unknown and the gate
/// can never observe what it claims to govern.
///
/// This is the warning RFC 0004-bis §13.1 owes: licence extraction covers five
/// of the twenty-one registry types, and on the other sixteen a `license_gate`
/// with the default `allow_unknown = true` never fires. An operator who wrote
/// an allow list and saw no errors would reasonably believe they had a licence
/// policy. They have an inert rule.
pub const LICENSE_GATE_NO_EXTRACTOR: &str = "license-gate.no-extractor";

/// A `license_gate` on a registry whose `[registries.sbom]` block is absent or
/// `enabled = false`.
///
/// The licence is recorded as a side effect of SBOM generation
/// (`ProxyService::maybe_trigger_sbom` returns early when SBOM is off for the
/// registry), so with SBOM disabled *nothing is ever extracted* and the gate
/// sees an unknown licence for every version — no matter how good the parser
/// for that registry type is. Found by running the thing rather than reading
/// it: the parser worked, the rule was loaded, and no licence was ever stored.
pub const LICENSE_GATE_SBOM_DISABLED: &str = "license-gate.sbom-disabled";

/// A `[registries.readme]` block written down on a registry type that has no
/// README to give — `maven`, the source-hosting kinds, the path-addressed kinds.
///
/// Accepted and inert. Only raised when the operator wrote the block: the
/// feature is on by default, so warning about every absent block would put a
/// notice on the admin panel for every `maven` registry in every deployment,
/// which is noise rather than information. Written down, it is a belief about
/// what the server will do, and it is wrong.
pub const README_UNSUPPORTED_TYPE: &str = "readme.unsupported-type";

/// `from_archive = true` on a registry kind whose README is metadata-borne only.
///
/// The artifact is never opened for it, so the setting does nothing. Distinct
/// from [`README_UNSUPPORTED_TYPE`]: READMEs *are* stored for this registry, and
/// an operator who set this to control cost should know it was already free.
pub const README_FROM_ARCHIVE_INERT: &str = "readme.from-archive-inert";

/// `from_archive = true` on a `firewall_only` registry.
///
/// `firewall_only` streams without buffering, so no artifact is ever cached to
/// extract from. Metadata-borne sources still work; archive-borne ones never
/// will — which on an archive-only kind (cargo, NuGet, Go, …) means this
/// registry stores no README at all, however the block is written.
pub const README_FROM_ARCHIVE_FIREWALL_ONLY: &str = "readme.from-archive-firewall-only";

/// `[registries.upstream_detail]` enabled on a `local`-mode registry.
///
/// Accepted and inert: there is no upstream to ask, and the package page is
/// already complete from local rows.
pub const UPSTREAM_DETAIL_LOCAL_MODE: &str = "upstream-detail.local-mode";

/// `[registries.upstream_detail]` enabled on a kind that cannot be asked.
///
/// The path-addressed kinds have no package identity to ask about, and the
/// source-hosting kinds' release listings are not what a package page shows.
/// Accepted and inert; the detail page answers from local rows only.
pub const UPSTREAM_DETAIL_UNSUPPORTED_KIND: &str = "upstream-detail.unsupported-kind";

/// `[registries.upstream_detail]` enabled on a registry with no reachable
/// upstream configured.
///
/// Warned rather than rejected because an air-gapped estate is a supported
/// deployment (RFC 0008), and its operator should be told the setting will
/// produce one failed attempt per TTL rather than have the server refuse to
/// start.
pub const UPSTREAM_DETAIL_NO_UPSTREAM: &str = "upstream-detail.no-upstream";

/// The same missing parser, but with `allow_unknown = false` and `block = true`
/// — so instead of never firing, the gate refuses **every** download from that
/// registry. Separate code from [`LICENSE_GATE_NO_EXTRACTOR`] because the
/// consequence is the opposite one and an operator triaging an outage needs to
/// find this by name.
pub const LICENSE_GATE_DENIES_EVERYTHING: &str = "license-gate.denies-everything";

/// `[search] readmes = true` while every registry has README capture off.
///
/// Accepted. The index will exist and stay empty, because nothing is ever stored
/// to put in it — so the search box grows an option that can only ever answer
/// "no package here says that" (RFC 0007-bis §4.5).
pub const SEARCH_READMES_NOTHING_STORED: &str = "search.readmes-nothing-stored";
