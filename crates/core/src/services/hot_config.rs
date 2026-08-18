use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use regex::Regex;
use tokio::sync::RwLock;

use crate::entities::{ResolutionPolicy, Role};
use crate::ports::{BetaChannelPort, RegistryClient};
use crate::rules::Rule;

/// Per-registry behaviour configuration wired in at startup (or on reload).
pub struct RegistryPolicy {
    pub metadata_ttl: Option<Duration>,
    /// Rules evaluated in order for every request to this registry.
    pub rules: Vec<Box<dyn Rule>>,
    /// When `true`, skip artifact storage entirely and stream directly from upstream.
    pub firewall_only: bool,
    /// When `true`, serve stale (expired) cached metadata if upstream returns a transient
    /// `Registry` error. Allows cached artifacts to keep being served during outages.
    pub serve_stale_metadata: bool,
    /// When set, artifacts are re-fetched from upstream after this duration even if
    /// present in storage.
    pub artifact_ttl: Option<Duration>,
}

/// Versioning policy enforced at publish time for a single registry.
#[derive(Default, Clone)]
pub struct VersioningPolicy {
    /// Reject versions that are not valid semver.
    pub enforce_semver: bool,
    /// If `enforce_semver` is true, also reject pre-release versions (e.g. `1.0.0-beta.1`).
    pub allow_prerelease: bool,
    /// Optional compiled regex; publish is rejected when the version string does not match.
    pub version_pattern: Option<Regex>,
}

/// Per-registry artifact integrity policy (mirrors config-layer `IntegrityConfig`).
///
/// Verification runs on the proxy fetch-and-cache path: once the upstream bytes
/// are buffered, they are hashed and compared against the checksum advertised in
/// the registry metadata (`PackageMetadata.checksum`). It does **not** apply to
/// `firewall_only` registries, which stream straight through without buffering.
#[derive(Debug, Clone)]
pub struct IntegrityPolicy {
    /// Master switch. When `false`, no verification is performed.
    pub enabled: bool,
    /// When a mismatch is detected, fail the download (and never cache the bytes).
    /// A mismatch is never bypassable — it means the bytes are wrong.
    pub block_on_mismatch: bool,
    /// When the upstream provides no usable checksum, block the download unless
    /// the caller holds one of `bypass_roles`. When `false` (default), a missing
    /// checksum is only warned about.
    pub require_metadata: bool,
    /// Roles allowed to bypass the `require_metadata` gate.
    pub bypass_roles: Vec<Role>,
    /// Re-verify stored bytes against a self-computed SHA-256 on every serve
    /// (cache hit / local read), not just on first fetch. Off by default.
    pub verify_on_serve: bool,
}

impl Default for IntegrityPolicy {
    fn default() -> Self {
        // Verify-and-block-on-mismatch by default: a mismatch indicates
        // corruption or tampering and should essentially never fire in normal
        // operation. Missing metadata only warns (registries like NuGet/Maven
        // advertise no checksum), so the default never blocks a healthy fetch.
        // Re-serve verification stays opt-in (it costs a read+hash per serve).
        Self {
            enabled: true,
            block_on_mismatch: true,
            require_metadata: false,
            bypass_roles: Vec::new(),
            verify_on_serve: false,
        }
    }
}

/// Signing configuration stored in the service (mirrors config-layer `SigningConfig`).
#[derive(Debug, Default, Clone)]
pub struct SigningConfig {
    pub required: bool,
    pub allowed_types: Vec<String>,
    /// Verify a stored `ed25519` detached signature against `trusted_keys` on download.
    pub verify_on_download: bool,
    /// Hex-encoded 32-byte Ed25519 public keys trusted to sign artifacts.
    pub trusted_keys: Vec<String>,
}

/// SBOM configuration stored in the service (mirrors config-layer `SbomConfig`).
#[derive(Debug, Default, Clone)]
pub struct SbomConfig {
    pub enabled: bool,
    pub formats: Vec<String>,
    pub required: bool,
    pub fetch_upstream: bool,
    /// The registry adapter type (e.g. "cargo", "npm") — used for archive extraction.
    pub registry_type: String,
}

/// What the renderer does with an `<img>` pointing at a third-party host.
///
/// There is deliberately no `Allow`: the SPA's CSP is baked into the document at
/// build time (`img-src 'self' data:`), so a setting that only worked in a custom
/// UI build would be a trap — the operator would set it and see broken images
/// with no error anywhere (RFC 0007 §4.1).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum RemoteImagePolicy {
    /// Replace the image with an inline chip carrying its `alt` text and its
    /// host, so the reader can see that an image was there and where it pointed.
    #[default]
    Strip,
    /// Rewrite it to fetch through this server. Every page view would otherwise
    /// beacon to a host the package author chose, from inside the network.
    Proxy,
}

impl RemoteImagePolicy {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Strip => "strip",
            Self::Proxy => "proxy",
        }
    }

    /// `None` for anything else — an unrecognised value must not silently become
    /// the default, because the two behaviours differ in what leaves the network.
    /// [`crate::entities::RegistryKind`]-style parse: the config validator turns
    /// the `None` into a refusal to start.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "strip" => Some(Self::Strip),
            "proxy" => Some(Self::Proxy),
            _ => None,
        }
    }
}

/// README capture configuration stored in the service (mirrors config-layer
/// `ReadmeConfig`).
///
/// Unlike [`SbomConfig`], the absence of the config block means **enabled**: for
/// the metadata-borne registry kinds the text is a field of a document the proxy
/// already fetches and parses, so the default costs one deserialised field
/// (RFC 0007 §4.1).
#[derive(Debug, Clone)]
pub struct ReadmeConfig {
    pub enabled: bool,
    /// Extract from the cached artifact when the metadata carries none. Rides
    /// the artifact read SBOM already performs when SBOM is on, and adds one
    /// storage read per newly-cached version when it is not.
    pub from_archive: bool,
    /// Cap on the **stored source**, applied after decompression at the point of
    /// extraction. Truncation is recorded and surfaced, never silent.
    pub max_bytes: usize,
    pub remote_images: RemoteImagePolicy,
    /// The registry adapter type (e.g. "cargo", "npm") — used to pick the
    /// extraction family, exactly as [`SbomConfig::registry_type`] is.
    pub registry_type: String,
}

impl Default for ReadmeConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            from_archive: true,
            max_bytes: DEFAULT_README_MAX_BYTES,
            remote_images: RemoteImagePolicy::Strip,
            registry_type: String::new(),
        }
    }
}

/// 256 KiB. Large enough for essentially every real README, small enough that
/// the row stays cheap to read on a page load.
pub const DEFAULT_README_MAX_BYTES: usize = 262_144;

/// The console's discovery read, per registry (mirrors config-layer
/// `UpstreamDetailConfig`).
///
/// A separate block from [`ReadmeConfig`] because it is not a README setting:
/// it governs the *version list* too, and an operator may want one without the
/// other (RFC 0007 §4.1).
///
/// **There is no TTL of its own.** The document lands in the existing metadata
/// cache under the key `cached_version_document` already builds, so it obeys the
/// registry's `metadata_ttl_secs` and its `serve_stale_metadata`. A second,
/// independently clocked expiry for the same bytes is how two caches come to
/// disagree about one document.
#[derive(Debug, Clone)]
pub struct UpstreamDetailConfig {
    pub enabled: bool,
    /// Cap on upstream-only versions returned for one package.
    ///
    /// Bounds the *response*, not the fetch: the document is one document
    /// whatever its size. Applied newest-first, and the response says it was
    /// applied — a silently shortened list is a lie about the registry.
    pub max_versions: usize,
    /// How long an upstream "no such package" is remembered, so a bad URL, a
    /// typo or a crawler cannot turn every reload into an upstream request.
    pub negative_ttl: Duration,
}

impl Default for UpstreamDetailConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_versions: DEFAULT_UPSTREAM_MAX_VERSIONS,
            negative_ttl: Duration::from_secs(DEFAULT_UPSTREAM_NEGATIVE_TTL_SECS),
        }
    }
}

/// 300 versions. Long enough for essentially every real package's table, short
/// enough that one page's JSON stays a page's worth.
pub const DEFAULT_UPSTREAM_MAX_VERSIONS: usize = 300;

/// Five minutes. Long enough to absorb a reload loop, short enough that a
/// package published a moment ago appears without an operator waiting.
pub const DEFAULT_UPSTREAM_NEGATIVE_TTL_SECS: u64 = 300;

/// Per-registry feature flags (mirrors config-layer `FeatureFlagsConfig`).
/// A "feature flag" category of optional, cross-cutting UI/integration toggles.
#[derive(Debug, Clone)]
pub struct FeatureFlags {
    /// Show the socket.dev supply-chain badge for each package version in the UI.
    pub socket_badge: bool,
}

impl Default for FeatureFlags {
    fn default() -> Self {
        // Flags default to "on" so a registry without a `[registries.feature_flags]`
        // block still gets the badge; it is disabled explicitly per registry.
        Self { socket_badge: true }
    }
}

/// All registry state that can be hot-reloaded without restarting the process.
///
/// Stored behind `Arc<RwLock<>>` inside `ProxyService` and `LocalRegistryService`.
/// When config is reloaded, the write lock is acquired, the struct is replaced in-place,
/// and in-flight requests finish with the previous data before seeing the update.
pub struct HotConfig {
    /// Per-registry proxy clients. `Arc` allows cheap cloning before releasing the read lock.
    pub registries: HashMap<String, Arc<dyn RegistryClient>>,
    /// Per-registry access policies. `Arc` allows cheap cloning (rules are not Clone).
    pub policies: HashMap<String, Arc<RegistryPolicy>>,
    /// Per-registry versioning policies (Clone, cheap).
    pub versioning: HashMap<String, VersioningPolicy>,
    /// Per-registry artifact signing configs (Clone, cheap).
    pub signing: HashMap<String, SigningConfig>,
    /// Per-registry SBOM generation configs (Clone, cheap).
    pub sbom: HashMap<String, SbomConfig>,
    /// Per-registry README capture configs (Clone, cheap).
    ///
    /// A registry with no entry is *not* one with the feature off: the absence
    /// of a `[registries.readme]` block means enabled, so the builder writes an
    /// entry for every registry and a missing key only happens in a test that
    /// did not care. Readers use `unwrap_or_default()`, which is the enabled
    /// shape (RFC 0007 §4.1).
    pub readme: HashMap<String, ReadmeConfig>,
    /// Per-registry discovery-read configs (Clone, cheap).
    ///
    /// Populated for every registry for the same reason `readme` is: the absence
    /// of a `[registries.upstream_detail]` block means **on**.
    pub upstream_detail: HashMap<String, UpstreamDetailConfig>,
    /// Per-registry feature flags (Clone, cheap).
    pub feature_flags: HashMap<String, FeatureFlags>,
    /// Per-registry artifact integrity policies (Clone, cheap).
    pub integrity: HashMap<String, IntegrityPolicy>,
    /// Per-registry beta-channel gate ports.
    pub beta_channel: HashMap<String, Arc<dyn BetaChannelPort>>,
    /// Per-registry inputs for naming a package's resolution state, as plain
    /// data (Clone, cheap).
    ///
    /// The numbers here are already expressed twice over in `policies` —
    /// `artifact_ttl_secs` in the cache config, `min_age`/`bypass_roles` inside
    /// a `ReleaseAgeGateRule`. Neither is readable back out: the rule is a
    /// `Box<dyn Rule>` with no accessors, by design. The catalog has to answer
    /// "is this quarantined, is this past its TTL" without running the download
    /// path, so the builder records the same values in a shape it can read.
    /// They are built from the identical config fields in one place
    /// (`server/src/builders.rs`) so the two cannot say different things.
    pub resolution: HashMap<String, ResolutionPolicy>,
    /// Maximum artifact size when buffering from upstream; None = 500 MiB default.
    pub max_artifact_size_bytes: Option<u64>,
}

impl Default for HotConfig {
    /// All maps empty, no size limit. Useful as a base for `..Default::default()`
    /// when only `registries`/`policies` (and occasionally one or two other fields)
    /// need to be set.
    fn default() -> Self {
        Self {
            registries: HashMap::new(),
            policies: HashMap::new(),
            versioning: HashMap::new(),
            signing: HashMap::new(),
            sbom: HashMap::new(),
            readme: HashMap::new(),
            upstream_detail: HashMap::new(),
            feature_flags: HashMap::new(),
            integrity: HashMap::new(),
            beta_channel: HashMap::new(),
            resolution: HashMap::new(),
            max_artifact_size_bytes: None,
        }
    }
}

/// Convenience alias: the shared hot-config lock used across services.
pub type HotConfigLock = Arc<RwLock<HotConfig>>;

/// Create a new `HotConfigLock` wrapping the given `HotConfig`.
pub fn new_hot_lock(cfg: HotConfig) -> HotConfigLock {
    Arc::new(RwLock::new(cfg))
}

#[cfg(test)]
mod tests {
    use super::{new_hot_lock, HotConfig};

    fn empty_config() -> HotConfig {
        HotConfig::default()
    }

    #[test]
    fn new_hot_lock_is_readable() {
        let lock = new_hot_lock(empty_config());
        let guard = lock.blocking_read();
        assert!(guard.registries.is_empty());
        assert_eq!(guard.max_artifact_size_bytes, None);
    }

    #[test]
    fn new_hot_lock_is_writable() {
        let lock = new_hot_lock(empty_config());
        {
            let mut guard = lock.blocking_write();
            guard.max_artifact_size_bytes = Some(100);
        }
        let guard = lock.blocking_read();
        assert_eq!(guard.max_artifact_size_bytes, Some(100));
    }
}
