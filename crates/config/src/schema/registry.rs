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

impl RegistryMode {
    /// The wire name, matching the `serde(rename_all = "lowercase")` above and
    /// the `mode` field on every registry-shaped response.
    ///
    /// The `front_office` registry listing hand-rolled this match; RFC 0004-bis
    /// A2 put a `mode` on the admin health DTO too, and two copies of a mapping
    /// that must agree is one too many.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Proxy => "proxy",
            Self::Local => "local",
            Self::Hybrid => "hybrid",
        }
    }
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
    /// Glob allowlist of upstream paths this registry may serve. Only meaningful
    /// for path-addressed registry types (`deb`/`rpm`/`pacman`/`jetbrains`/`generic`),
    /// where the request path is passed straight through to the upstream — without
    /// it, a registry pointed at a shared host (`storage.googleapis.com`, a CDN
    /// serving many vendors) would relay *every* path on that host.
    ///
    /// Patterns are matched against the upstream-relative path with
    /// [`glob::Pattern`] semantics, where `*` also crosses `/`. Required for
    /// `generic`; `["**"]` is the explicit opt-out that allows everything.
    ///
    /// ```toml
    /// path_allow = ["v*/node-v*-linux-x64.tar.*", "v*/SHASUMS256.txt*"]
    /// ```
    #[serde(default)]
    pub path_allow: Vec<String>,
    /// Extra hostnames whose **root** serves this registry, in addition to
    /// `/proxy/{name}/…`. Everything reachable under the subpath is reachable
    /// identically at the root of each of these hosts.
    ///
    /// ```toml
    /// hosts = ["npm.acme.io"]
    /// ```
    ///
    /// Independent of `[subdomain_routing]`: a registry can have vanity hosts
    /// with no wildcard configured at all, and an explicit entry wins over this
    /// registry's own wildcard host. A host claimed by two registries is a config
    /// error. DNS and TLS for these names are the operator's job.
    #[serde(default)]
    pub hosts: Vec<String>,
    /// Whether `/proxy/{name}/…` serves this registry. Defaults to `true`.
    ///
    /// `false` makes the registry reachable **only** through its `hosts` (or its
    /// wildcard host), so content handed to a team under `npm.acme.io` does not
    /// also answer on the shared main host, where it would inherit that host's
    /// CORS policy, WAF rules and cache keys. The subpath then returns `404`, not
    /// `403` — a disabled ingress should look absent, not forbidden.
    ///
    /// `path_routing = false` on a registry with no reachable host is a config
    /// error: nothing could talk to it.
    #[serde(default = "default_true")]
    pub path_routing: bool,
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
    /// Optional README capture configuration.
    ///
    /// Unlike [`Self::sbom`], **absent means enabled**: for the metadata-borne
    /// registry kinds the text is a field of a document the proxy already
    /// fetches and parses, so the default costs one deserialised field
    /// (RFC 0007 §4.1).
    #[serde(default)]
    pub readme: Option<ReadmeConfig>,
    /// Optional configuration for the console's discovery read — whether this
    /// instance may ask upstream about a package it holds nothing of.
    ///
    /// A separate block from [`Self::readme`] because it is not a README
    /// setting: it governs the version list too, and an operator may want one
    /// without the other. **Absent means enabled** (RFC 0007 §4.1).
    #[serde(default)]
    pub upstream_detail: Option<UpstreamDetailConfig>,
    /// Whether the console may ask this instance to fetch a version from
    /// upstream — the **Fetch this version** button (RFC 0007-bis §4.1, §4.4).
    ///
    /// **On by default**, and it admits nothing: the fetch runs the same
    /// download the caller could already run with `curl`, through every gate
    /// that download would pass, attributed to them in the audit log. The switch
    /// exists for the operator who wants the console strictly read-only, which
    /// is a legitimate posture and not one the software should have to guess at.
    ///
    /// Inert on a `local`-mode registry: there is no upstream to fetch from, and
    /// every version the page lists is already held.
    #[serde(default = "default_true")]
    pub console_fetch: bool,
    /// Optional per-registry feature flags (opt-in/out toggles for cross-cutting
    /// UI/integration features). When absent, every flag takes its default.
    #[serde(default)]
    pub feature_flags: Option<FeatureFlagsConfig>,
    /// Optional artifact integrity (checksum) verification on proxied downloads.
    /// When absent, the defaults apply: verify against any advertised checksum
    /// and block on a mismatch; warn (do not block) when none is advertised.
    #[serde(default)]
    pub integrity: Option<IntegrityConfig>,
    /// Base URL for the Go Vulnerability Database (`govulndb`) proxy endpoints.
    /// Only applies to `goproxy` registries.
    /// Absent → default to `https://vuln.go.dev`.
    /// Set to `""` to disable the `/v1/index.json`, `/v1/ID/{id}.json`, and
    /// `/v1/query` passthrough endpoints for this registry.
    #[serde(default)]
    pub vuln_db_url: Option<String>,
    /// Base URL for the Go checksum database (`GOSUMDB`) proxy endpoint.
    /// Only applies to `goproxy` registries.
    /// Absent → default to `https://sum.golang.org`.
    /// Set to `""` to disable `/sumdb/{path}` for this registry.
    ///
    /// Proxying the checksum database is the other half of `GOPROXY`
    /// (RFC 0009 §7.4). Without it `go mod download` still opens a direct
    /// connection to `sum.golang.org` for every module it has not seen, which
    /// fails closed in an air-gapped estate — so the proxy has moved the egress
    /// rather than removed it. A registry serving only private modules has no
    /// sumdb and should set `""`, because a lookup there would leak private
    /// module paths upstream.
    #[serde(default)]
    pub sumdb_url: Option<String>,
}

// ── Artifact integrity ──────────────────────────────────────────────────────────

/// Per-registry artifact integrity verification, applied on the proxy
/// fetch-and-cache path. Once upstream bytes are buffered they are hashed and
/// compared against the checksum advertised in the registry metadata
/// (Cargo SHA-256, npm SRI/`shasum`, PyPI SHA-256). Registries that advertise no
/// checksum (NuGet, Maven, GitHub, Go, …) fall through to the "missing" path.
///
/// Does **not** apply to `firewall_only` registries, which stream straight
/// through without buffering.
///
/// ```toml
/// [registries.integrity]
/// enabled = true            # verify when a checksum is advertised
/// block_on_mismatch = true  # fail the download on a hash mismatch (never bypassable)
/// require_metadata = false  # block downloads with no advertised checksum
/// bypass_roles = ["admin"]  # roles exempt from the require_metadata gate
/// ```
#[derive(Debug, Clone, Deserialize)]
pub struct IntegrityConfig {
    /// Master switch. When `false`, no verification is performed.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Fail the download (and skip caching) when the computed digest does not
    /// match the advertised one. A mismatch is never bypassable.
    #[serde(default = "default_true")]
    pub block_on_mismatch: bool,
    /// Block downloads for which the upstream advertises no usable checksum,
    /// unless the caller holds one of `bypass_roles`. Defaults to `false`
    /// (missing checksums are only warned about).
    #[serde(default)]
    pub require_metadata: bool,
    /// Roles allowed to bypass the `require_metadata` gate.
    #[serde(default)]
    pub bypass_roles: Vec<String>,
    /// Re-verify cached/stored bytes against a self-computed SHA-256 on **every**
    /// serve (cache hit on the proxy path, and local-registry reads), not just on
    /// the first fetch. Catches storage corruption or tampering of already-cached
    /// artifacts. Off by default: it reads and hashes the bytes on each serve (the
    /// proxy path streams them through the hash, so memory stays bounded, then
    /// re-opens the entry to serve it). A mismatch fails the download (`502`) and
    /// evicts the bad entry.
    #[serde(default)]
    pub verify_on_serve: bool,
}

impl Default for IntegrityConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            block_on_mismatch: true,
            require_metadata: false,
            bypass_roles: Vec::new(),
            verify_on_serve: false,
        }
    }
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
    /// When `true`, verify a stored `ed25519` detached signature against
    /// `trusted_keys` on every download. A stored signature that fails to verify
    /// (or was signed by an untrusted key) fails the download with `502`.
    /// A stored signature of an *unsupported* type (anything other than
    /// `ed25519`, which cannot be verified here) is likewise **rejected** with
    /// `502` — the download fails closed rather than serving unverified bytes.
    /// Only artifacts with *no* stored signature are exempt (their presence is
    /// governed by `required` at publish time).
    #[serde(default)]
    pub verify_on_download: bool,
    /// Hex-encoded 32-byte Ed25519 public keys trusted to sign artifacts in this
    /// registry. A download verifies against each in turn; any match passes.
    #[serde(default)]
    pub trusted_keys: Vec<String>,
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

// ── README capture ────────────────────────────────────────────────────────────

fn default_readme_max_bytes() -> usize {
    batlehub_core::services::DEFAULT_README_MAX_BYTES
}

fn default_remote_images() -> String {
    "strip".to_owned()
}

fn default_image_max_bytes() -> usize {
    batlehub_core::services::DEFAULT_README_IMAGE_MAX_BYTES
}

/// Per-registry README capture configuration.
///
/// ```toml
/// [registries.readme]
/// enabled       = true      # store and serve READMEs for this registry
/// from_archive  = true      # extract from the cached artifact when the metadata carries none
/// max_bytes       = 262144    # cap on stored source (256 KiB); larger is truncated and flagged
/// remote_images   = "strip"   # "strip" | "proxy"
/// image_max_bytes = 2097152   # cap on one proxied image (2 MiB); larger is not served
/// ```
///
/// The whole block is optional and its absence means **on**, which is why every
/// field defaults to the enabled shape rather than to `false`/zero. `from_archive`
/// is the one part of that default that is not free: it rides the artifact read
/// SBOM already performs when SBOM is on, and adds one storage read per
/// newly-cached version when it is not.
#[derive(Debug, Clone, Deserialize)]
pub struct ReadmeConfig {
    /// Store and serve READMEs for this registry.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Read the README out of the artifact when the metadata carries none. Inert
    /// on kinds whose README is metadata-borne only, and on `firewall_only`
    /// registries, which never cache an artifact to extract from — both warned
    /// about rather than rejected.
    #[serde(default = "default_true")]
    pub from_archive: bool,
    /// Cap on the stored source, in bytes, applied after decompression.
    /// Truncation is recorded and surfaced, never silent.
    #[serde(default = "default_readme_max_bytes")]
    pub max_bytes: usize,
    /// `"strip"` (default) or `"proxy"`. There is no `"allow"`: the SPA's CSP is
    /// baked into the document at build time, so it would silently do nothing.
    #[serde(default = "default_remote_images")]
    pub remote_images: String,
    /// Cap on **one proxied image**, in bytes. Separate from `max_bytes`, which
    /// caps the stored *text*: a 256 KiB text cap and a 2 MiB image cap are not
    /// the same number for the same reason, and sharing one would make raising
    /// either a decision about the other (RFC 0007-bis §4.1).
    ///
    /// Inert under `remote_images = "strip"`, where nothing is fetched.
    #[serde(default = "default_image_max_bytes")]
    pub image_max_bytes: usize,
}

impl Default for ReadmeConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            from_archive: true,
            max_bytes: default_readme_max_bytes(),
            remote_images: default_remote_images(),
            image_max_bytes: default_image_max_bytes(),
        }
    }
}

// ── The console's discovery read ──────────────────────────────────────────────

fn default_upstream_max_versions() -> usize {
    batlehub_core::services::DEFAULT_UPSTREAM_MAX_VERSIONS
}

fn default_upstream_negative_ttl_secs() -> u64 {
    batlehub_core::services::DEFAULT_UPSTREAM_NEGATIVE_TTL_SECS
}

/// Whether the console may ask upstream about a package this instance holds
/// nothing of, and how much of the answer it may show.
///
/// ```toml
/// [registries.upstream_detail]
/// enabled           = true    # the console may ask upstream about a package we hold nothing of
/// max_versions      = 300     # cap on upstream-only versions returned for one package
/// negative_ttl_secs = 300     # how long an upstream "no such package" is remembered
/// ```
///
/// **There is no TTL of its own.** The document lands in the existing metadata
/// cache, so it obeys the registry's `metadata_ttl_secs` and its `serve_stale`.
/// A second, independently clocked expiry for the same bytes is how two caches
/// come to disagree about one document.
///
/// Inert on a `local`-mode registry — there is no upstream to ask — and on the
/// path-addressed kinds, which have no package identity to ask about. Both are
/// warned about rather than rejected.
#[derive(Debug, Clone, Deserialize)]
pub struct UpstreamDetailConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Cap on the upstream-only versions one package's page is handed. Applied
    /// newest-first, and the response says it was applied: a silently shortened
    /// list is a lie about the registry.
    #[serde(default = "default_upstream_max_versions")]
    pub max_versions: usize,
    /// How long an upstream `404` is remembered, so a bad URL, a typo or a
    /// crawler cannot turn every reload into an upstream request. A connection
    /// failure is not a fact about the package and is never remembered.
    #[serde(default = "default_upstream_negative_ttl_secs")]
    pub negative_ttl_secs: u64,
}

impl Default for UpstreamDetailConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_versions: default_upstream_max_versions(),
            negative_ttl_secs: default_upstream_negative_ttl_secs(),
        }
    }
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
    /// Upstream artifact paths to pre-fetch, for path-addressed registries
    /// (`deb`/`rpm`/`jetbrains`) that have no per-package version model. Each entry
    /// is the upstream-relative path, e.g. `"idea/idea-2026.1.3.tar.gz"` for a
    /// JetBrains registry or `"dists/stable/Release"` for a Deb registry. Warmed on
    /// startup and via the `/warm` admin endpoint (`paths`).
    #[serde(default)]
    pub warm_paths: Vec<String>,
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
            warm_paths: vec![],
            warm_latest_n: default_warm_latest_n(),
            warm_concurrency: default_warm_concurrency(),
        }
    }
}
