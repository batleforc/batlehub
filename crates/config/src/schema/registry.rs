use serde::Deserialize;

use super::network::{RateLimitConfig, UpstreamAuthConfig, UpstreamProxyConfig, UpstreamTlsConfig};
use super::rules::{RbacConfig, RuleConfig};

// ── Registry mode ─────────────────────────────────────────────────────────────

/// Controls whether a registry acts as a caching proxy, a private authoritative
/// registry, or both.
#[derive(Debug, Deserialize, Default, Clone, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RegistryMode {
    /// Forward all requests to upstream registries and cache responses.
    #[default]
    Proxy,
    /// BatleHub is the authoritative source; no upstream is consulted.
    Local,
    /// Check local publications first; fall back to upstream if not found.
    Hybrid,
}

// ── Registry config ───────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct RegistryConfig {
    #[serde(rename = "type")]
    pub registry_type: String,
    pub name: String,
    /// Upstream URLs tried in order; if a registry returns 404 the next one is tried.
    /// When empty the adapter's built-in default (e.g. registry.npmjs.org) is used.
    #[serde(default)]
    pub upstreams: Vec<String>,
    /// Cargo only: URL of the sparse crate index.
    /// Defaults to `https://index.crates.io` when the upstream is crates.io.
    /// Set this for self-hosted registries (e.g. Gitea/Forgejo package feeds).
    #[serde(default)]
    pub index_url: Option<String>,
    #[serde(default)]
    pub cache: CachePolicy,
    #[serde(default)]
    pub rbac: RbacConfig,
    #[serde(default)]
    pub rules: Vec<RuleConfig>,
    /// Name of the storage backend to use for this registry's artifacts.
    /// Must match one of the backend names in `[[storage.backends]]`.
    /// When absent, the default backend is used.
    #[serde(default)]
    pub storage: Option<String>,
    /// When `true` the registry acts as a pure firewall: rules are evaluated but
    /// artifacts are never cached. Requests that pass rules are streamed directly
    /// from upstream with nothing written to storage.
    #[serde(default)]
    pub firewall_only: bool,
    /// Credentials to send on every upstream request for this registry.
    #[serde(default)]
    pub upstream_auth: Option<UpstreamAuthConfig>,
    /// TLS settings for upstream connections (e.g. custom CA certificate).
    #[serde(default)]
    pub tls: Option<UpstreamTlsConfig>,
    /// Optional HTTP/SOCKS proxy for upstream connections.
    #[serde(default)]
    pub proxy: Option<UpstreamProxyConfig>,
    /// Controls proxy vs. local vs. hybrid behaviour for this registry.
    #[serde(default)]
    pub mode: RegistryMode,
    /// Optional publish quota enforced on local/hybrid registries.
    #[serde(default)]
    pub quota: Option<QuotaConfig>,
    /// Optional per-user request rate limit for this registry.
    #[serde(default)]
    pub rate_limit: Option<RateLimitConfig>,
    /// Optional versioning policy enforced at publish time (local/hybrid mode only).
    #[serde(default)]
    pub versioning: Option<VersioningPolicy>,
    /// Optional artifact signing configuration (local/hybrid mode only).
    #[serde(default)]
    pub signing: Option<SigningConfig>,
    /// Optional Ed25519 OpenPGP key for signing generated Deb/RPM repository
    /// metadata (`Release`/`InRelease`/`Release.gpg`, `repomd.xml.asc`). When
    /// absent, the hosted repository is unsigned.
    #[serde(default)]
    pub repo_signing: Option<RepoSigningConfig>,
    /// Optional beta-channel configuration (local/hybrid mode only).
    /// When enabled, pre-release versions are only visible to registered beta-channel members.
    #[serde(default)]
    pub beta_channel: Option<BetaChannelConfig>,
    /// Base URL of the upstream search API used by the Package Explorer.
    ///
    /// When absent, each registry type falls back to its built-in default:
    /// - `maven`    → `https://search.maven.org`
    /// - `composer` → `https://packagist.org` (for packagist.org-based repos)
    ///
    /// Set to `""` (empty string) to disable upstream search for this registry.
    /// Has no effect on registry types that do not support upstream search.
    #[serde(default)]
    pub search_url: Option<String>,
    /// Optional SBOM generation configuration. When absent, SBOM is disabled.
    #[serde(default)]
    pub sbom: Option<SbomConfig>,
    /// Optional per-registry feature flags (opt-in/out toggles for cross-cutting
    /// UI/integration features). When absent, every flag takes its default.
    #[serde(default)]
    pub feature_flags: Option<FeatureFlagsConfig>,
}

// ── Feature flags ─────────────────────────────────────────────────────────────

/// Per-registry "feature flag" category: a set of named boolean toggles for
/// optional, cross-cutting features that can be turned on or off for a whole
/// registry. New opt-in features add a new field here.
///
/// ```toml
/// [registries.feature_flags]
/// socket_badge = false   # hide the socket.dev badge for this registry
/// ```
#[derive(Debug, Clone, Deserialize)]
pub struct FeatureFlagsConfig {
    /// Show a [socket.dev](https://socket.dev) supply-chain badge/link for each
    /// package version in the UI (for registry types socket.dev supports, e.g.
    /// `cargo`, `npm`, `pypi`). Enabled by default; set to `false` to disable
    /// the badge for the whole registry.
    #[serde(default = "default_true")]
    pub socket_badge: bool,
}

impl Default for FeatureFlagsConfig {
    fn default() -> Self {
        Self {
            socket_badge: default_true(),
        }
    }
}

// ── Versioning policy ─────────────────────────────────────────────────────────

/// Per-registry versioning policy enforced at publish time.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct VersioningPolicy {
    /// Reject publish if the version string is not a valid semver (e.g. `1.2.3`, `1.0.0-beta.1`).
    #[serde(default)]
    pub enforce_semver: bool,
    /// Reject publish if the semver pre-release component is non-empty (e.g. `-alpha`, `-beta.1`).
    /// Only effective when `enforce_semver` is also `true`.
    #[serde(default = "default_true")]
    pub allow_prerelease: bool,
    /// Reject publish if the version string does not match this regex.
    #[serde(default)]
    pub version_pattern: Option<String>,
}

pub fn default_true() -> bool {
    true
}

// ── Artifact signing ──────────────────────────────────────────────────────────

/// Per-registry artifact signing configuration.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SigningConfig {
    /// When `true`, reject publish requests that do not include an `X-Artifact-Signature` header.
    #[serde(default)]
    pub required: bool,
    /// Accepted signature types (e.g. `["pgp", "ed25519"]`).
    /// When empty, any type (or no type) is accepted.
    #[serde(default)]
    pub allowed_types: Vec<String>,
}

/// Ed25519 repository-metadata signing key for `deb`/`rpm` registries.
///
/// ```toml
/// [registries.repo_signing]
/// seed_hex = "9d61b19d..."   # 32-byte Ed25519 seed, hex-encoded
/// user_id  = "BatleHub Repo <repo@example.com>"
/// created  = 1700000000      # key creation unix time (stable across restarts)
/// ```
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RepoSigningConfig {
    /// Hex-encoded 32-byte Ed25519 seed.
    pub seed_hex: String,
    /// OpenPGP User ID string. Defaults to `"BatleHub"`.
    #[serde(default)]
    pub user_id: Option<String>,
    /// Key creation time (unix seconds). Part of the fingerprint, so it must stay
    /// stable. Defaults to 0.
    #[serde(default)]
    pub created: Option<u32>,
}

// ── SBOM generation ───────────────────────────────────────────────────────────

fn default_sbom_formats() -> Vec<String> {
    vec!["spdx".to_owned(), "cyclonedx".to_owned()]
}

/// Per-registry SBOM generation configuration.
///
/// ```toml
/// [registries.sbom]
/// enabled        = true
/// formats        = ["spdx", "cyclonedx"]
/// required       = false   # deny publish when no manifest found
/// fetch_upstream = true    # try GitHub/npm upstream SBOM APIs first
/// ```
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SbomConfig {
    /// Enable SBOM generation for this registry.
    #[serde(default)]
    pub enabled: bool,
    /// Formats to generate. Defaults to both when enabled.
    #[serde(default = "default_sbom_formats")]
    pub formats: Vec<String>,
    /// When `true`, deny publish if no dependency manifest can be found in the archive.
    #[serde(default)]
    pub required: bool,
    /// When `true`, attempt to fetch a pre-built SBOM from the upstream before
    /// falling back to extraction / minimal generation.
    #[serde(default = "default_true")]
    pub fetch_upstream: bool,
}

// ── Quota management ──────────────────────────────────────────────────────────

/// How to enforce quota violations.
#[derive(Debug, Deserialize, Default, Clone, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum QuotaEnforcement {
    /// Reject the publish request with HTTP 429 when the quota is exceeded.
    #[default]
    Block,
    /// Allow the publish but include a warning header in the response.
    Warn,
}

/// Per-registry publish quotas for local/hybrid mode.
///
/// Example TOML:
/// ```toml
/// [registries.quota]
/// max_storage_bytes_per_user = 1_073_741_824   # 1 GiB
/// max_packages_per_user      = 100
/// warn_threshold_pct         = 80
/// enforcement                = "block"
/// ```
#[derive(Debug, Deserialize, Clone)]
pub struct QuotaConfig {
    /// Maximum cumulative bytes a single user may publish to this registry.
    pub max_storage_bytes_per_user: Option<u64>,
    /// Maximum number of distinct package versions a single user may publish.
    pub max_packages_per_user: Option<u32>,
    /// Emit a quota-warning response header when usage exceeds this percentage
    /// of the limit. Defaults to 80.
    #[serde(default = "default_warn_pct")]
    pub warn_threshold_pct: u8,
    /// Whether to hard-block or just warn on quota overrun.
    #[serde(default)]
    pub enforcement: QuotaEnforcement,
}

fn default_warn_pct() -> u8 {
    80
}

// ── Beta channel ──────────────────────────────────────────────────────────────

/// Per-registry beta-channel configuration (local/hybrid mode only).
///
/// When `enabled` is `true`, pre-release versions (semver versions with a
/// non-empty pre-release component, e.g. `1.0.0-beta.1`) are hidden from users
/// who are not registered as beta-channel members. Non-members receive 404 on
/// both index listings and artifact downloads for pre-release versions.
///
/// ```toml
/// [registries.beta_channel]
/// enabled = true
/// ```
#[derive(Debug, Clone, Default, Deserialize)]
pub struct BetaChannelConfig {
    /// Enable pre-release gating for this registry.
    #[serde(default)]
    pub enabled: bool,
}

// ── Cache policy ──────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CachePolicy {
    /// TTL for metadata (version lists, release info) in seconds.
    #[serde(default = "default_metadata_ttl")]
    pub metadata_ttl_secs: u64,
    /// When true (the default), serve stale metadata when upstream returns a transient
    /// error instead of propagating a 502. Allows cached artifacts to keep being served
    /// during upstream outages.
    #[serde(default = "default_serve_stale")]
    pub serve_stale: bool,
    /// Evict artifacts older than this many seconds. `null` means never expire by age.
    #[serde(default)]
    pub artifact_ttl_secs: Option<u64>,
    /// Evict artifacts not accessed for this many days. `null` means never expire by idle time.
    #[serde(default)]
    pub idle_days: Option<u64>,
    /// Storage size cap in bytes. When exceeded, the least-recently-used artifacts are evicted
    /// until usage falls below this threshold. `null` means no size cap.
    #[serde(default)]
    pub max_size_bytes: Option<u64>,
    /// Keep only the N most-recently-cached versions per (registry, package). Older versions
    /// are evicted when a new one is stored. `null` means keep all versions.
    #[serde(default)]
    pub keep_latest_n: Option<usize>,
    /// Packages to pre-fetch on startup and via the `/warm` admin endpoint.
    /// Each entry is either a bare package name (`"lodash"`) or a pinned version
    /// (`"lodash@4.17.21"`). Bare names warm the latest `warm_latest_n` versions.
    #[serde(default)]
    pub warm_packages: Vec<String>,
    /// Number of most-recent versions to pre-warm per package (default: 1 = latest only).
    #[serde(default = "default_warm_latest_n")]
    pub warm_latest_n: usize,
    /// Maximum number of concurrent artifact downloads during a warming run (default: 2).
    #[serde(default = "default_warm_concurrency")]
    pub warm_concurrency: usize,
}

fn default_metadata_ttl() -> u64 {
    300
}

fn default_serve_stale() -> bool {
    true
}

fn default_warm_latest_n() -> usize {
    1
}

fn default_warm_concurrency() -> usize {
    2
}

impl Default for CachePolicy {
    fn default() -> Self {
        Self {
            metadata_ttl_secs: default_metadata_ttl(),
            serve_stale: true,
            artifact_ttl_secs: None,
            idle_days: None,
            max_size_bytes: None,
            keep_latest_n: None,
            warm_packages: vec![],
            warm_latest_n: default_warm_latest_n(),
            warm_concurrency: default_warm_concurrency(),
        }
    }
}
