use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use regex::Regex;
use tokio::sync::RwLock;

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

/// Signing configuration stored in the service (mirrors config-layer `SigningConfig`).
#[derive(Debug, Default, Clone)]
pub struct SigningConfig {
    pub required: bool,
    pub allowed_types: Vec<String>,
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
    /// Per-registry feature flags (Clone, cheap).
    pub feature_flags: HashMap<String, FeatureFlags>,
    /// Per-registry beta-channel gate ports.
    pub beta_channel: HashMap<String, Arc<dyn BetaChannelPort>>,
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
            feature_flags: HashMap::new(),
            beta_channel: HashMap::new(),
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
