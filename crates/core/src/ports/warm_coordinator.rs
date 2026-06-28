use std::time::Duration;

use async_trait::async_trait;

/// Distributed coordination gate for the cache warm-up path.
///
/// When multiple replicas restart simultaneously, each would independently
/// discover the same cache miss and download the same artifact from upstream.
/// A `WarmCoordinator` lets the first replica to call `try_claim` win the work;
/// all others skip (returning `skipped: 1`), avoiding redundant upstream downloads.
///
/// The claim is automatically released (or expires via TTL) after the download
/// completes, so a crashed replica does not block future warm-up runs.
#[async_trait]
pub trait WarmCoordinator: Send + Sync {
    /// Attempt to claim the right to warm `key`. Returns `true` if this replica
    /// won the claim, `false` if another replica already holds it.
    async fn try_claim(&self, key: &str, ttl: Duration) -> bool;

    /// Release a previously-claimed key. Called whether the warm succeeded or failed.
    async fn release(&self, key: &str);
}

/// No-op implementation used when Redis is not configured.
/// Always grants the claim; multiple replicas may redundantly warm the same artifact.
pub struct NoopWarmCoordinator;

#[async_trait]
impl WarmCoordinator for NoopWarmCoordinator {
    async fn try_claim(&self, _key: &str, _ttl: Duration) -> bool {
        true
    }

    async fn release(&self, _key: &str) {}
}
