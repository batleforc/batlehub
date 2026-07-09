mod explore;
mod packages;
mod query;

#[cfg(test)]
mod tests;

use std::future::Future;
use std::sync::Arc;

use crate::entities::{
    AccessAction, AccessEvent, AccessResult, ArtifactVulnerability, Identity, PackageId,
};
use crate::error::CoreError;
use crate::ports::{PackageRepository, VulnerabilityRepository};
use crate::services::explore_cache::ExploreCache;

/// Cap on simultaneous in-flight operations for a single bulk admin action, to
/// avoid a large selection (thousands of packages) opening more concurrent DB
/// connections than the pool can serve.
const BULK_ACTION_CONCURRENCY: usize = 16;

pub struct BulkBlockItem {
    pub package_id: PackageId,
    pub reason: String,
}

pub struct BulkActionResult {
    pub succeeded: Vec<PackageId>,
    pub failed: Vec<(PackageId, String)>,
}

pub struct AdminService {
    pub repo: Arc<dyn PackageRepository>,
    pub explore_cache: Arc<ExploreCache>,
    /// Optional source of vulnerability findings (the periodic SBOM re-scan).
    /// When absent, `list_vulnerabilities` returns an empty list.
    pub vuln_repo: Option<Arc<dyn VulnerabilityRepository>>,
}

impl AdminService {
    pub fn new(repo: Arc<dyn PackageRepository>) -> Self {
        Self {
            repo,
            explore_cache: Arc::new(ExploreCache::new()),
            vuln_repo: None,
        }
    }

    /// Attach a vulnerability repository so package detail views can surface
    /// findings recorded by the periodic SBOM re-scan.
    #[must_use]
    pub fn with_vulnerability_repo(mut self, repo: Arc<dyn VulnerabilityRepository>) -> Self {
        self.vuln_repo = Some(repo);
        self
    }

    /// List recorded vulnerability findings for a package coordinate.
    /// Returns an empty list when no vulnerability repository is attached.
    pub async fn list_vulnerabilities(
        &self,
        registry: &str,
        name: &str,
        version: &str,
    ) -> Result<Vec<ArtifactVulnerability>, CoreError> {
        match &self.vuln_repo {
            Some(repo) => repo.list_for_coordinate(registry, name, version).await,
            None => Ok(vec![]),
        }
    }

    /// Shared audit-write path for admin actions that don't otherwise touch
    /// `PackageRepository` (ownership/visibility edits go through their own
    /// ports, account/network-wide actions have no package at all). Mirrors
    /// the fail-open behaviour of `block_package`/`unblock_package`/
    /// `delete_package`: an audit-write failure is logged but never fails the
    /// calling admin action.
    pub(super) async fn record_admin_action(
        &self,
        package_id: Option<PackageId>,
        action: AccessAction,
        by_identity: &Identity,
    ) {
        self.repo
            .record_access(AccessEvent {
                id: uuid::Uuid::new_v4(),
                user_id: by_identity.user_id.clone(),
                user_role: by_identity.role.clone(),
                package_id,
                action,
                result: AccessResult::Allowed,
                timestamp: chrono::Utc::now(),
                ip_address: None,
                user_agent: None,
            })
            .await
            .unwrap_or_else(|e| tracing::warn!(error = %e, "failed to record admin action"));
    }

    /// Shared fan-out path for bulk admin actions: runs `op` over `items` with
    /// bounded concurrency and aggregates the per-item outcomes into a
    /// [`BulkActionResult`]. `op` reports its own failure message (rather than
    /// a `CoreError`) so callers can report domain-specific failures — e.g.
    /// `bulk_delete_packages`'s "package not found" for a `false` return —
    /// without forcing every bulk action through the same error type.
    pub(super) async fn run_bulk<T, F, Fut>(&self, items: Vec<T>, op: F) -> BulkActionResult
    where
        F: Fn(T) -> Fut,
        Fut: Future<Output = (PackageId, Result<(), String>)>,
    {
        use futures::StreamExt;

        let results: Vec<_> = futures::stream::iter(items)
            .map(op)
            .buffer_unordered(BULK_ACTION_CONCURRENCY)
            .collect()
            .await;

        let mut result = BulkActionResult {
            succeeded: vec![],
            failed: vec![],
        };
        for (pkg, outcome) in results {
            match outcome {
                Ok(()) => result.succeeded.push(pkg),
                Err(msg) => result.failed.push((pkg, msg)),
            }
        }
        result
    }
}
