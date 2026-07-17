mod run;

#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::time::Duration;

use crate::ports::{ArtifactCacheMeta, RegistryClient, StorageBackend, WarmCoordinator};
use crate::services::metrics::ProxyMetrics;

/// How long a warm-up claim is held in the coordinator. Long enough to cover the
/// full fetch+store cycle for large artifacts; short enough to unblock other replicas
/// when the winning replica crashes mid-download.
const WARM_CLAIM_TTL: Duration = Duration::from_secs(600);

/// Result of a warming run (a single package or a batch).
#[derive(Debug, Default, Clone)]
pub struct WarmingReport {
    /// Artifact versions fetched and stored during this run.
    pub warmed: usize,
    /// Artifact versions already present in storage (skipped).
    pub skipped: usize,
    /// Versions that failed to fetch or store.
    pub errors: usize,
}

impl std::ops::AddAssign for WarmingReport {
    fn add_assign(&mut self, other: Self) {
        self.warmed += other.warmed;
        self.skipped += other.skipped;
        self.errors += other.errors;
    }
}

/// Pre-fetches artifact versions from an upstream registry and stores them in
/// the local cache so they are available with zero latency on first request.
///
/// `Clone` is derived so `warm_one_version`'s spawn sites can pass a single
/// `self.clone()` (four cheap `Arc` bumps + a `String`/two `usize`s) instead of
/// naming each field individually at every call site.
#[derive(Clone)]
pub struct WarmingService {
    pub client: Arc<dyn RegistryClient>,
    pub storage: Arc<dyn StorageBackend>,
    pub artifact_meta: Arc<dyn ArtifactCacheMeta>,
    pub registry_name: String,
    /// How many of the most-recent versions to warm per package.
    /// Ignored when the package string includes a pinned version (e.g. `"lodash@4.17.21"`).
    pub latest_n: usize,
    /// Maximum concurrent artifact downloads.
    pub concurrency: usize,
    /// Cross-replica coordination: prevents multiple replicas from downloading
    /// the same artifact simultaneously. Defaults to `NoopWarmCoordinator`.
    pub coordinator: Arc<dyn WarmCoordinator>,
    /// Shared with `ProxyService` so warming traffic feeds the same
    /// upstream-health signal as regular proxy reads.
    pub metrics: Arc<ProxyMetrics>,
}
