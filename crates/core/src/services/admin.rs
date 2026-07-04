use std::future::Future;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use futures::StreamExt;

use crate::entities::{
    AccessAction, AccessEvent, AccessResult, ArtifactVulnerability, EventFilter, ExploreEntry,
    ExploreFilter, Identity, PackageFilter, PackageId, PackageStatus, PackageSummary, RegistryStat,
};
use crate::error::CoreError;
use crate::ports::{PackageRepository, VulnerabilityRepository};
use crate::services::explore_cache::{
    packages_cache_key, packages_entry_registries, stats_cache_key, ExploreCache,
};

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

    pub async fn block_package(
        &self,
        pkg: &PackageId,
        reason: String,
        by_identity: &Identity,
    ) -> Result<(), CoreError> {
        let blocked_by = by_identity
            .user_id
            .clone()
            .unwrap_or_else(|| by_identity.role.to_string());

        self.repo
            .set_status(
                pkg,
                PackageStatus::Blocked {
                    reason: reason.clone(),
                    blocked_by: blocked_by.clone(),
                    blocked_at: Utc::now(),
                },
            )
            .await?;

        self.record_admin_action(Some(pkg.clone()), AccessAction::Block, by_identity)
            .await;

        tracing::info!(package = %pkg, blocked_by = %blocked_by, reason = %reason, "package blocked");
        Ok(())
    }

    pub async fn unblock_package(
        &self,
        pkg: &PackageId,
        by_identity: &Identity,
    ) -> Result<(), CoreError> {
        self.repo.set_status(pkg, PackageStatus::Available).await?;

        self.record_admin_action(Some(pkg.clone()), AccessAction::Unblock, by_identity)
            .await;

        tracing::info!(package = %pkg, "package unblocked");
        Ok(())
    }

    /// Shared audit-write path for admin actions that don't otherwise touch
    /// `PackageRepository` (ownership/visibility edits go through their own
    /// ports, account/network-wide actions have no package at all). Mirrors
    /// the fail-open behaviour of `block_package`/`unblock_package`/
    /// `delete_package`: an audit-write failure is logged but never fails the
    /// calling admin action.
    async fn record_admin_action(
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
                timestamp: Utc::now(),
                ip_address: None,
                user_agent: None,
            })
            .await
            .unwrap_or_else(|e| tracing::warn!(error = %e, "failed to record admin action"));
    }

    /// Record an audit event for a package-scoped admin action performed
    /// through a port other than `PackageRepository` (e.g. ownership grants,
    /// visibility changes).
    pub async fn record_package_action(
        &self,
        pkg: &PackageId,
        action: AccessAction,
        by_identity: &Identity,
    ) {
        self.record_admin_action(Some(pkg.clone()), action, by_identity)
            .await;
    }

    /// Record an audit event for an account-wide or network-wide admin
    /// action that is not scoped to any specific package (user block/unblock,
    /// IP block/unblock).
    pub async fn record_account_action(&self, action: AccessAction, by_identity: &Identity) {
        self.record_admin_action(None, action, by_identity).await;
    }

    /// Shared fan-out path for bulk admin actions: runs `op` over `items` with
    /// bounded concurrency and aggregates the per-item outcomes into a
    /// [`BulkActionResult`]. `op` reports its own failure message (rather than
    /// a `CoreError`) so callers can report domain-specific failures — e.g.
    /// `bulk_delete_packages`'s "package not found" for a `false` return —
    /// without forcing every bulk action through the same error type.
    async fn run_bulk<T, F, Fut>(&self, items: Vec<T>, op: F) -> BulkActionResult
    where
        F: Fn(T) -> Fut,
        Fut: Future<Output = (PackageId, Result<(), String>)>,
    {
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

    pub async fn bulk_block_packages(
        &self,
        items: Vec<BulkBlockItem>,
        by_identity: &Identity,
    ) -> BulkActionResult {
        self.run_bulk(items, |item| async move {
            let outcome = self
                .block_package(&item.package_id, item.reason, by_identity)
                .await;
            (item.package_id, outcome.map_err(|e| e.to_string()))
        })
        .await
    }

    pub async fn bulk_unblock_packages(
        &self,
        items: Vec<PackageId>,
        by_identity: &Identity,
    ) -> BulkActionResult {
        self.run_bulk(items, |pkg| async move {
            let outcome = self.unblock_package(&pkg, by_identity).await;
            (pkg, outcome.map_err(|e| e.to_string()))
        })
        .await
    }

    /// Remove a package's administrative record.
    ///
    /// Returns `true` if the row existed and was deleted. The caller is responsible
    /// for also purging the cached artifact from storage when desired.
    pub async fn delete_package(
        &self,
        pkg: &PackageId,
        by_identity: &Identity,
    ) -> Result<bool, CoreError> {
        let deleted = self.repo.delete_package(pkg).await?;
        if deleted {
            self.record_admin_action(Some(pkg.clone()), AccessAction::Delete, by_identity)
                .await;
            tracing::info!(package = %pkg, "package record deleted");
        }
        Ok(deleted)
    }

    pub async fn bulk_delete_packages(
        &self,
        items: Vec<PackageId>,
        by_identity: &Identity,
    ) -> BulkActionResult {
        self.run_bulk(items, |pkg| async move {
            let outcome = match self.delete_package(&pkg, by_identity).await {
                Ok(true) => Ok(()),
                Ok(false) => Err("package not found".to_string()),
                Err(e) => Err(e.to_string()),
            };
            (pkg, outcome)
        })
        .await
    }

    pub async fn list_packages(
        &self,
        filter: PackageFilter,
    ) -> Result<Vec<PackageSummary>, CoreError> {
        self.repo.list_packages(filter).await
    }

    pub async fn count_packages(&self, filter: PackageFilter) -> Result<u64, CoreError> {
        self.repo.count_packages(filter).await
    }

    pub async fn list_events(&self, filter: EventFilter) -> Result<Vec<AccessEvent>, CoreError> {
        self.repo.list_events(filter).await
    }

    /// Purge access-audit rows older than `before`. Records one `AuditPurge`
    /// event capturing who ran the purge and the resulting row count, so the
    /// compliance trail survives even though the purge itself removes history.
    pub async fn purge_events_before(
        &self,
        before: DateTime<Utc>,
        by_identity: &Identity,
    ) -> Result<u64, CoreError> {
        let deleted = self.repo.purge_events_before(before).await?;

        self.record_account_action(AccessAction::AuditPurge, by_identity)
            .await;

        tracing::info!(
            user_id = by_identity.user_id.as_deref().unwrap_or(""),
            cutoff = %before,
            deleted,
            "access-audit log purged"
        );
        Ok(deleted)
    }

    pub async fn get_package_status(&self, pkg: &PackageId) -> Result<PackageStatus, CoreError> {
        self.repo.get_status(pkg).await
    }

    /// Shared cache-first-then-repo-with-stale-fallback flow for
    /// [`Self::explore_packages`] and [`Self::count_explore_packages`], which
    /// both read/write the same `(items, count)`-shaped cache entry (each
    /// caller only cares about one half; see `PackageEntry` — every
    /// `set_packages` call carries real data for the half it computed and a
    /// placeholder for the other, then blindly overwrites the entry).
    ///
    /// `upstream_unavailable` (third tuple element) is `true` only when the DB
    /// is unreachable **and** no stale cache entry exists. When stale data is
    /// served the flag stays `false` so callers can distinguish "stale but
    /// usable" from "nothing at all".
    ///
    /// Not shared with [`Self::registry_explore_stats`]: that method reads a
    /// differently-shaped cache entry (`Vec<RegistryStat>` via
    /// `get_stats`/`set_stats`/`get_stale_stats`, no `(items, count)` pair), so
    /// folding it into this helper too would mean genericizing over the cache
    /// accessor methods as well as the fetch — not worth it for one call site.
    async fn cached_packages_query(
        &self,
        key: &str,
        filter: &ExploreFilter,
        fetch: impl Future<Output = Result<(Vec<ExploreEntry>, u64), CoreError>>,
    ) -> Result<(Vec<ExploreEntry>, u64, bool), CoreError> {
        if let Some((items, count)) = self.explore_cache.get_packages(key).await {
            return Ok((items, count, false));
        }
        match fetch.await {
            Ok((items, count)) => {
                let regs = packages_entry_registries(filter);
                self.explore_cache
                    .set_packages(key, items.clone(), count, regs)
                    .await;
                Ok((items, count, false))
            }
            Err(_) => {
                if let Some((items, count)) = self.explore_cache.get_stale_packages(key).await {
                    Ok((items, count, false))
                } else {
                    Ok((vec![], 0, true))
                }
            }
        }
    }

    /// Returns `(entries, upstream_unavailable)`.
    pub async fn explore_packages(
        &self,
        filter: ExploreFilter,
    ) -> Result<(Vec<ExploreEntry>, bool), CoreError> {
        // Namespaced so an items-lookup and a count-lookup can never collide on
        // the same cache entry, even if a future caller passes matching
        // limit/offset filters to both.
        let key = format!("items:{}", packages_cache_key(&filter));
        let fetch = async {
            let items = self.repo.explore_packages(filter.clone()).await?;
            Ok((items, 0))
        };
        let (items, _count, unavailable) = self.cached_packages_query(&key, &filter, fetch).await?;
        Ok((items, unavailable))
    }

    /// Returns `(count, upstream_unavailable)`.
    pub async fn count_explore_packages(
        &self,
        filter: ExploreFilter,
    ) -> Result<(u64, bool), CoreError> {
        let key = format!("count:{}", packages_cache_key(&filter));
        let fetch = async {
            let count = self.repo.count_explore_packages(filter.clone()).await?;
            Ok((vec![], count))
        };
        let (_items, count, unavailable) = self.cached_packages_query(&key, &filter, fetch).await?;
        Ok((count, unavailable))
    }

    /// Returns `(stats, upstream_unavailable)`.
    pub async fn registry_explore_stats(
        &self,
        accessible_registries: &[String],
    ) -> Result<(Vec<RegistryStat>, bool), CoreError> {
        let key = stats_cache_key(accessible_registries);
        if let Some(stats) = self.explore_cache.get_stats(&key).await {
            return Ok((stats, false));
        }
        match self
            .repo
            .registry_explore_stats(accessible_registries)
            .await
        {
            Ok(stats) => {
                self.explore_cache
                    .set_stats(&key, stats.clone(), accessible_registries.to_vec())
                    .await;
                Ok((stats, false))
            }
            Err(_) => {
                if let Some(stats) = self.explore_cache.get_stale_stats(&key).await {
                    Ok((stats, false))
                } else {
                    Ok((vec![], true))
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;

    use super::*;
    use crate::entities::{
        AccessEvent, AccessResult, EventFilter, Identity, PackageFilter, PackageId, PackageStatus,
        PackageSummary, Role,
    };
    use crate::error::CoreError;
    use crate::ports::PackageRepository;

    struct MemRepo {
        statuses: Mutex<HashMap<String, PackageStatus>>,
        events: Mutex<Vec<AccessEvent>>,
    }

    impl MemRepo {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                statuses: Mutex::new(HashMap::new()),
                events: Mutex::new(vec![]),
            })
        }

        fn events(&self) -> Vec<AccessEvent> {
            self.events.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl PackageRepository for MemRepo {
        async fn record_access(&self, event: AccessEvent) -> Result<(), CoreError> {
            self.events.lock().unwrap().push(event);
            Ok(())
        }
        async fn get_status(&self, pkg: &PackageId) -> Result<PackageStatus, CoreError> {
            Ok(self
                .statuses
                .lock()
                .unwrap()
                .get(&pkg.cache_key())
                .cloned()
                .unwrap_or(PackageStatus::Available))
        }
        async fn set_status(
            &self,
            pkg: &PackageId,
            status: PackageStatus,
        ) -> Result<(), CoreError> {
            self.statuses
                .lock()
                .unwrap()
                .insert(pkg.cache_key(), status);
            Ok(())
        }
        async fn list_packages(&self, _f: PackageFilter) -> Result<Vec<PackageSummary>, CoreError> {
            Ok(vec![])
        }
        async fn count_packages(&self, _f: PackageFilter) -> Result<u64, CoreError> {
            Ok(0)
        }
        async fn list_events(&self, _f: EventFilter) -> Result<Vec<AccessEvent>, CoreError> {
            Ok(self.events.lock().unwrap().clone())
        }
        async fn delete_package(&self, pkg: &PackageId) -> Result<bool, CoreError> {
            Ok(self
                .statuses
                .lock()
                .unwrap()
                .remove(&pkg.cache_key())
                .is_some())
        }
    }

    fn admin_identity(user_id: &str) -> Identity {
        Identity {
            user_id: Some(user_id.to_owned()),
            role: Role::Admin,
            auth_provider: None,
            groups: vec![],
        }
    }

    fn anon_identity() -> Identity {
        Identity::anonymous()
    }

    #[tokio::test]
    async fn block_package_sets_blocked_status() {
        let repo = MemRepo::new();
        let svc = AdminService::new(repo.clone());
        let pkg = PackageId::new("npm", "evil", "1.0.0");

        svc.block_package(
            &pkg,
            "supply chain risk".to_owned(),
            &admin_identity("alice"),
        )
        .await
        .unwrap();

        let status = repo.get_status(&pkg).await.unwrap();
        assert!(status.is_blocked());
    }

    #[tokio::test]
    async fn block_package_uses_user_id_as_blocked_by() {
        let repo = MemRepo::new();
        let svc = AdminService::new(repo.clone());
        let pkg = PackageId::new("npm", "evil", "1.0.0");

        svc.block_package(&pkg, "reason".to_owned(), &admin_identity("alice"))
            .await
            .unwrap();

        let status = repo.get_status(&pkg).await.unwrap();
        match status {
            PackageStatus::Blocked { blocked_by, .. } => assert_eq!(blocked_by, "alice"),
            PackageStatus::Available => panic!("expected Blocked"),
        }
    }

    #[tokio::test]
    async fn block_package_falls_back_to_role_when_no_user_id() {
        let repo = MemRepo::new();
        let svc = AdminService::new(repo.clone());
        let pkg = PackageId::new("npm", "evil", "1.0.0");

        svc.block_package(&pkg, "reason".to_owned(), &anon_identity())
            .await
            .unwrap();

        let status = repo.get_status(&pkg).await.unwrap();
        match status {
            PackageStatus::Blocked { blocked_by, .. } => assert_eq!(blocked_by, "anonymous"),
            PackageStatus::Available => panic!("expected Blocked"),
        }
    }

    #[tokio::test]
    async fn block_package_records_audit_event() {
        let repo = MemRepo::new();
        let svc = AdminService::new(repo.clone());
        let pkg = PackageId::new("npm", "evil", "1.0.0");

        svc.block_package(&pkg, "reason".to_owned(), &admin_identity("alice"))
            .await
            .unwrap();

        let events = repo.events();
        assert_eq!(events.len(), 1, "one event expected");
        assert!(matches!(events[0].result, AccessResult::Allowed));
    }

    #[tokio::test]
    async fn record_package_action_records_package_scoped_event() {
        let repo = MemRepo::new();
        let svc = AdminService::new(repo.clone());
        let pkg = PackageId::new("npm", "left-pad", "1.0.0");

        svc.record_package_action(
            &pkg,
            crate::entities::AccessAction::AddOwner,
            &admin_identity("alice"),
        )
        .await;

        let events = repo.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].package_id, Some(pkg));
        assert!(matches!(
            events[0].action,
            crate::entities::AccessAction::AddOwner
        ));
        assert!(matches!(events[0].result, AccessResult::Allowed));
    }

    #[tokio::test]
    async fn record_account_action_records_event_with_no_package() {
        let repo = MemRepo::new();
        let svc = AdminService::new(repo.clone());

        svc.record_account_action(
            crate::entities::AccessAction::BlockUser,
            &admin_identity("alice"),
        )
        .await;

        let events = repo.events();
        assert_eq!(events.len(), 1);
        assert!(events[0].package_id.is_none());
        assert!(matches!(
            events[0].action,
            crate::entities::AccessAction::BlockUser
        ));
    }

    #[tokio::test]
    async fn purge_events_before_records_audit_purge_event() {
        let repo = MemRepo::new();
        let svc = AdminService::new(repo.clone());

        svc.purge_events_before(Utc::now(), &admin_identity("alice"))
            .await
            .unwrap();

        let events = repo.events();
        assert_eq!(events.len(), 1, "purge itself must leave an audit trail");
        assert!(events[0].package_id.is_none());
        assert_eq!(events[0].user_id.as_deref(), Some("alice"));
        assert!(matches!(events[0].action, AccessAction::AuditPurge));
        assert!(matches!(events[0].result, AccessResult::Allowed));
    }

    #[tokio::test]
    async fn unblock_package_sets_available_status() {
        let repo = MemRepo::new();
        let svc = AdminService::new(repo.clone());
        let pkg = PackageId::new("npm", "evil", "1.0.0");

        // Block first, then unblock
        svc.block_package(&pkg, "r".to_owned(), &admin_identity("a"))
            .await
            .unwrap();
        svc.unblock_package(&pkg, &admin_identity("a"))
            .await
            .unwrap();

        let status = repo.get_status(&pkg).await.unwrap();
        assert!(!status.is_blocked());
    }

    #[tokio::test]
    async fn unblock_package_records_audit_event() {
        let repo = MemRepo::new();
        let svc = AdminService::new(repo.clone());
        let pkg = PackageId::new("npm", "evil", "1.0.0");

        svc.block_package(&pkg, "r".to_owned(), &admin_identity("a"))
            .await
            .unwrap();
        svc.unblock_package(&pkg, &admin_identity("a"))
            .await
            .unwrap();

        let events = repo.events();
        assert_eq!(events.len(), 2, "block + unblock events expected");
    }

    #[tokio::test]
    async fn get_package_status_delegates_to_repo() {
        let repo = MemRepo::new();
        let svc = AdminService::new(repo.clone());
        let pkg = PackageId::new("npm", "serde", "1.0.0");

        // Fresh package defaults to Available
        let status = svc.get_package_status(&pkg).await.unwrap();
        assert!(!status.is_blocked());
    }

    #[tokio::test]
    async fn list_packages_returns_repo_results() {
        let repo = MemRepo::new();
        let svc = AdminService::new(repo.clone());

        let result = svc.list_packages(PackageFilter::new()).await.unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn list_events_returns_repo_results() {
        let repo = MemRepo::new();
        let svc = AdminService::new(repo.clone());
        let pkg = PackageId::new("npm", "pkg", "1.0.0");

        svc.block_package(&pkg, "r".to_owned(), &admin_identity("a"))
            .await
            .unwrap();

        let events = svc.list_events(EventFilter::new()).await.unwrap();
        assert!(!events.is_empty());
    }

    #[tokio::test]
    async fn bulk_block_succeeds_for_all_valid_packages() {
        let repo = MemRepo::new();
        let svc = AdminService::new(repo.clone());
        let items = vec![
            BulkBlockItem {
                package_id: PackageId::new("npm", "a", "1.0.0"),
                reason: "r1".into(),
            },
            BulkBlockItem {
                package_id: PackageId::new("npm", "b", "2.0.0"),
                reason: "r2".into(),
            },
        ];
        let result = svc
            .bulk_block_packages(items, &admin_identity("alice"))
            .await;
        assert_eq!(result.succeeded.len(), 2);
        assert_eq!(result.failed.len(), 0);
        assert!(repo
            .get_status(&PackageId::new("npm", "a", "1.0.0"))
            .await
            .unwrap()
            .is_blocked());
        assert!(repo
            .get_status(&PackageId::new("npm", "b", "2.0.0"))
            .await
            .unwrap()
            .is_blocked());
    }

    #[tokio::test]
    async fn bulk_delete_reports_success_and_not_found() {
        let repo = MemRepo::new();
        let svc = AdminService::new(repo.clone());
        let pkg_a = PackageId::new("npm", "a", "1.0.0");
        let pkg_missing = PackageId::new("npm", "missing", "1.0.0");
        svc.block_package(&pkg_a, "r".into(), &admin_identity("alice"))
            .await
            .unwrap();

        let result = svc
            .bulk_delete_packages(
                vec![pkg_a.clone(), pkg_missing.clone()],
                &admin_identity("alice"),
            )
            .await;
        assert_eq!(result.succeeded, vec![pkg_a]);
        assert_eq!(result.failed.len(), 1);
        assert_eq!(result.failed[0].0, pkg_missing);
        assert_eq!(result.failed[0].1, "package not found");
    }

    #[tokio::test]
    async fn bulk_block_handles_more_items_than_the_concurrency_cap() {
        // BULK_ACTION_CONCURRENCY is 16; exercise a batch larger than that to
        // confirm the bounded-concurrency stream still processes every item
        // (not just the first `buffer_unordered` window).
        let repo = MemRepo::new();
        let svc = AdminService::new(repo.clone());
        let items: Vec<BulkBlockItem> = (0..40)
            .map(|i| BulkBlockItem {
                package_id: PackageId::new("npm", format!("pkg-{i}"), "1.0.0"),
                reason: "bulk".into(),
            })
            .collect();

        let result = svc
            .bulk_block_packages(items, &admin_identity("alice"))
            .await;
        assert_eq!(result.succeeded.len(), 40);
        assert_eq!(result.failed.len(), 0);
        for i in 0..40 {
            assert!(repo
                .get_status(&PackageId::new("npm", format!("pkg-{i}"), "1.0.0"))
                .await
                .unwrap()
                .is_blocked());
        }
    }

    #[tokio::test]
    async fn bulk_unblock_succeeds_for_all_packages() {
        let repo = MemRepo::new();
        let svc = AdminService::new(repo.clone());
        let pkg_a = PackageId::new("npm", "a", "1.0.0");
        let pkg_b = PackageId::new("npm", "b", "2.0.0");
        svc.block_package(&pkg_a, "r".into(), &admin_identity("alice"))
            .await
            .unwrap();
        svc.block_package(&pkg_b, "r".into(), &admin_identity("alice"))
            .await
            .unwrap();

        let result = svc
            .bulk_unblock_packages(vec![pkg_a.clone(), pkg_b.clone()], &admin_identity("alice"))
            .await;
        assert_eq!(result.succeeded.len(), 2);
        assert_eq!(result.failed.len(), 0);
        assert!(!repo.get_status(&pkg_a).await.unwrap().is_blocked());
        assert!(!repo.get_status(&pkg_b).await.unwrap().is_blocked());
    }

    // ── explore cache integration ─────────────────────────────────────────────

    use std::time::Duration;

    use crate::entities::{ExploreEntry, ExploreFilter, PackageSource, RegistryStat};
    use crate::services::explore_cache::ExploreCache;

    /// A repo that always returns a fixed set of explore results.
    struct StubExploreRepo {
        entries: Vec<ExploreEntry>,
        count: u64,
        stats: Vec<RegistryStat>,
    }

    impl StubExploreRepo {
        fn arc(entries: Vec<ExploreEntry>, count: u64, stats: Vec<RegistryStat>) -> Arc<Self> {
            Arc::new(Self {
                entries,
                count,
                stats,
            })
        }
    }

    #[async_trait]
    impl PackageRepository for StubExploreRepo {
        async fn record_access(&self, _: AccessEvent) -> Result<(), CoreError> {
            Ok(())
        }
        async fn get_status(&self, _: &PackageId) -> Result<PackageStatus, CoreError> {
            Ok(PackageStatus::Available)
        }
        async fn set_status(&self, _: &PackageId, _: PackageStatus) -> Result<(), CoreError> {
            Ok(())
        }
        async fn list_packages(&self, _: PackageFilter) -> Result<Vec<PackageSummary>, CoreError> {
            Ok(vec![])
        }
        async fn count_packages(&self, _: PackageFilter) -> Result<u64, CoreError> {
            Ok(0)
        }
        async fn list_events(&self, _: EventFilter) -> Result<Vec<AccessEvent>, CoreError> {
            Ok(vec![])
        }
        async fn explore_packages(&self, _: ExploreFilter) -> Result<Vec<ExploreEntry>, CoreError> {
            Ok(self.entries.clone())
        }
        async fn count_explore_packages(&self, _: ExploreFilter) -> Result<u64, CoreError> {
            Ok(self.count)
        }
        async fn registry_explore_stats(
            &self,
            _: &[String],
        ) -> Result<Vec<RegistryStat>, CoreError> {
            Ok(self.stats.clone())
        }
        async fn delete_package(&self, _: &PackageId) -> Result<bool, CoreError> {
            Ok(false)
        }
    }

    /// A repo whose explore methods always return an error.
    struct FailingExploreRepo;

    impl FailingExploreRepo {
        fn arc() -> Arc<Self> {
            Arc::new(Self)
        }
    }

    #[async_trait]
    impl PackageRepository for FailingExploreRepo {
        async fn record_access(&self, _: AccessEvent) -> Result<(), CoreError> {
            Ok(())
        }
        async fn get_status(&self, _: &PackageId) -> Result<PackageStatus, CoreError> {
            Ok(PackageStatus::Available)
        }
        async fn set_status(&self, _: &PackageId, _: PackageStatus) -> Result<(), CoreError> {
            Ok(())
        }
        async fn list_packages(&self, _: PackageFilter) -> Result<Vec<PackageSummary>, CoreError> {
            Ok(vec![])
        }
        async fn count_packages(&self, _: PackageFilter) -> Result<u64, CoreError> {
            Ok(0)
        }
        async fn list_events(&self, _: EventFilter) -> Result<Vec<AccessEvent>, CoreError> {
            Ok(vec![])
        }
        async fn explore_packages(&self, _: ExploreFilter) -> Result<Vec<ExploreEntry>, CoreError> {
            Err(CoreError::Database("simulated failure".into()))
        }
        async fn count_explore_packages(&self, _: ExploreFilter) -> Result<u64, CoreError> {
            Err(CoreError::Database("simulated failure".into()))
        }
        async fn registry_explore_stats(
            &self,
            _: &[String],
        ) -> Result<Vec<RegistryStat>, CoreError> {
            Err(CoreError::Database("simulated failure".into()))
        }
        async fn delete_package(&self, _: &PackageId) -> Result<bool, CoreError> {
            Ok(false)
        }
    }

    fn sample_entry() -> ExploreEntry {
        ExploreEntry {
            registry: "npm".into(),
            name: "lodash".into(),
            version_count: 1,
            total_downloads: 10,
            last_accessed: None,
            source: PackageSource::Proxied,
            has_blocked: false,
        }
    }

    fn sample_stat() -> RegistryStat {
        RegistryStat {
            registry: "npm".into(),
            package_count: 1,
            total_downloads: 10,
        }
    }

    fn default_filter() -> ExploreFilter {
        ExploreFilter {
            registry: Some("npm".into()),
            registries: vec![],
            ..ExploreFilter::default()
        }
    }

    fn make_svc_with_cache(
        repo: Arc<dyn PackageRepository>,
        cache: Arc<ExploreCache>,
    ) -> AdminService {
        AdminService {
            repo,
            explore_cache: cache,
            vuln_repo: None,
        }
    }

    // ── explore_packages ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn explore_packages_cache_hit_skips_repo() {
        let cache = Arc::new(ExploreCache::new());
        let filter = default_filter();
        // Pre-warm the cache with known data
        cache
            .set_packages("pre", vec![sample_entry()], 1, vec!["npm".into()])
            .await;

        // Use the same key the service would compute
        let key = format!(
            "items:{}",
            crate::services::explore_cache::packages_cache_key(&filter)
        );
        cache
            .set_packages(&key, vec![sample_entry()], 1, vec!["npm".into()])
            .await;

        // FailingRepo would return Err, but the cache hit prevents it from being called
        let svc = make_svc_with_cache(FailingExploreRepo::arc(), Arc::clone(&cache));
        let (items, unavailable) = svc.explore_packages(filter).await.unwrap();
        assert!(!unavailable);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "lodash");
    }

    #[tokio::test]
    async fn explore_packages_cache_miss_queries_repo_and_caches() {
        let cache = Arc::new(ExploreCache::new());
        let filter = default_filter();
        let repo = StubExploreRepo::arc(vec![sample_entry()], 1, vec![]);
        let svc = make_svc_with_cache(repo, Arc::clone(&cache));

        let (items, unavailable) = svc.explore_packages(filter.clone()).await.unwrap();
        assert!(!unavailable);
        assert_eq!(items.len(), 1);

        // A second call with a failing repo should now be served from cache
        let svc2 = make_svc_with_cache(FailingExploreRepo::arc(), Arc::clone(&cache));
        let (cached_items, unavailable2) = svc2.explore_packages(filter).await.unwrap();
        assert!(!unavailable2);
        assert_eq!(cached_items.len(), 1);
    }

    #[tokio::test]
    async fn explore_packages_db_fail_stale_exists_serves_stale() {
        // Use 0-TTL cache so the entry is immediately stale
        let cache = Arc::new(ExploreCache::with_ttl(Duration::ZERO));
        let filter = default_filter();
        let key = format!(
            "items:{}",
            crate::services::explore_cache::packages_cache_key(&filter)
        );
        cache
            .set_packages(&key, vec![sample_entry()], 1, vec!["npm".into()])
            .await;

        let svc = make_svc_with_cache(FailingExploreRepo::arc(), Arc::clone(&cache));
        let (items, unavailable) = svc.explore_packages(filter).await.unwrap();
        // Stale served → upstream_unavailable must be false
        assert!(!unavailable);
        assert_eq!(items.len(), 1);
    }

    #[tokio::test]
    async fn explore_packages_db_fail_no_cache_returns_unavailable() {
        let cache = Arc::new(ExploreCache::new());
        let svc = make_svc_with_cache(FailingExploreRepo::arc(), cache);
        let (items, unavailable) = svc.explore_packages(default_filter()).await.unwrap();
        assert!(unavailable);
        assert!(items.is_empty());
    }

    // ── count_explore_packages ────────────────────────────────────────────────

    #[tokio::test]
    async fn count_explore_packages_cache_hit_skips_repo() {
        let cache = Arc::new(ExploreCache::new());
        let filter = default_filter();
        let key = format!(
            "count:{}",
            crate::services::explore_cache::packages_cache_key(&filter)
        );
        // set_packages stores the count too
        cache
            .set_packages(&key, vec![], 42, vec!["npm".into()])
            .await;

        let svc = make_svc_with_cache(FailingExploreRepo::arc(), Arc::clone(&cache));
        let (count, unavailable) = svc.count_explore_packages(filter).await.unwrap();
        assert!(!unavailable);
        assert_eq!(count, 42);
    }

    #[tokio::test]
    async fn count_explore_packages_db_fail_no_cache_returns_unavailable() {
        let cache = Arc::new(ExploreCache::new());
        let svc = make_svc_with_cache(FailingExploreRepo::arc(), cache);
        let (count, unavailable) = svc.count_explore_packages(default_filter()).await.unwrap();
        assert!(unavailable);
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn count_explore_packages_db_fail_stale_returns_count() {
        let cache = Arc::new(ExploreCache::with_ttl(Duration::ZERO));
        let filter = default_filter();
        let key = format!(
            "count:{}",
            crate::services::explore_cache::packages_cache_key(&filter)
        );
        cache
            .set_packages(&key, vec![], 7, vec!["npm".into()])
            .await;

        let svc = make_svc_with_cache(FailingExploreRepo::arc(), Arc::clone(&cache));
        let (count, unavailable) = svc.count_explore_packages(filter).await.unwrap();
        assert!(!unavailable);
        assert_eq!(count, 7);
    }

    #[tokio::test]
    async fn explore_and_count_do_not_clobber_each_other_with_matching_filters() {
        // Same limit/offset (as would happen if a caller ever passed `per_page=0`
        // to both the items and the count query) must not let one call's cached
        // write stomp the other's, since each only carries real data for the
        // half it computed (items+0, or empty-items+count).
        let cache = Arc::new(ExploreCache::new());
        let filter = ExploreFilter {
            registry: Some("npm".into()),
            registries: vec![],
            limit: 0,
            offset: 0,
            ..ExploreFilter::default()
        };
        let repo = StubExploreRepo::arc(vec![sample_entry()], 42, vec![]);
        let svc = make_svc_with_cache(repo, Arc::clone(&cache));

        let (items, items_unavailable) = svc.explore_packages(filter.clone()).await.unwrap();
        let (count, count_unavailable) = svc.count_explore_packages(filter.clone()).await.unwrap();
        assert!(!items_unavailable && !count_unavailable);
        assert_eq!(items.len(), 1);
        assert_eq!(count, 42);

        // Re-read both from cache (repo now fails) — neither should have been
        // poisoned by the other's write.
        let svc2 = make_svc_with_cache(FailingExploreRepo::arc(), Arc::clone(&cache));
        let (cached_items, _) = svc2.explore_packages(filter.clone()).await.unwrap();
        let (cached_count, _) = svc2.count_explore_packages(filter).await.unwrap();
        assert_eq!(
            cached_items.len(),
            1,
            "items must not be clobbered by the count write"
        );
        assert_eq!(
            cached_count, 42,
            "count must not be clobbered by the items write"
        );
    }

    // ── registry_explore_stats ────────────────────────────────────────────────

    #[tokio::test]
    async fn registry_explore_stats_cache_hit_skips_repo() {
        let cache = Arc::new(ExploreCache::new());
        let regs = vec!["npm".to_string()];
        let key = crate::services::explore_cache::stats_cache_key(&regs);
        cache
            .set_stats(&key, vec![sample_stat()], regs.clone())
            .await;

        let svc = make_svc_with_cache(FailingExploreRepo::arc(), Arc::clone(&cache));
        let (stats, unavailable) = svc.registry_explore_stats(&regs).await.unwrap();
        assert!(!unavailable);
        assert_eq!(stats.len(), 1);
        assert_eq!(stats[0].registry, "npm");
    }

    #[tokio::test]
    async fn registry_explore_stats_cache_miss_queries_repo_and_caches() {
        let cache = Arc::new(ExploreCache::new());
        let regs = vec!["npm".to_string()];
        let repo = StubExploreRepo::arc(vec![], 0, vec![sample_stat()]);
        let svc = make_svc_with_cache(repo, Arc::clone(&cache));

        let (stats, unavailable) = svc.registry_explore_stats(&regs).await.unwrap();
        assert!(!unavailable);
        assert_eq!(stats.len(), 1);

        // Second call with failing repo should serve from cache
        let svc2 = make_svc_with_cache(FailingExploreRepo::arc(), Arc::clone(&cache));
        let (cached, _) = svc2.registry_explore_stats(&regs).await.unwrap();
        assert_eq!(cached.len(), 1);
    }

    #[tokio::test]
    async fn registry_explore_stats_db_fail_stale_serves_stale() {
        let cache = Arc::new(ExploreCache::with_ttl(Duration::ZERO));
        let regs = vec!["npm".to_string()];
        let key = crate::services::explore_cache::stats_cache_key(&regs);
        cache
            .set_stats(&key, vec![sample_stat()], regs.clone())
            .await;

        let svc = make_svc_with_cache(FailingExploreRepo::arc(), Arc::clone(&cache));
        let (stats, unavailable) = svc.registry_explore_stats(&regs).await.unwrap();
        assert!(!unavailable);
        assert_eq!(stats.len(), 1);
    }

    #[tokio::test]
    async fn registry_explore_stats_db_fail_no_cache_returns_unavailable() {
        let cache = Arc::new(ExploreCache::new());
        let svc = make_svc_with_cache(FailingExploreRepo::arc(), cache);
        let (stats, unavailable) = svc.registry_explore_stats(&["npm".into()]).await.unwrap();
        assert!(unavailable);
        assert!(stats.is_empty());
    }

    // ── cache invalidation on admin action ────────────────────────────────────

    #[tokio::test]
    async fn explore_cache_invalidation_clears_subsequent_reads() {
        let cache = Arc::new(ExploreCache::new());
        let filter = default_filter();
        let key = format!(
            "items:{}",
            crate::services::explore_cache::packages_cache_key(&filter)
        );
        cache
            .set_packages(&key, vec![sample_entry()], 1, vec!["npm".into()])
            .await;

        // Invalidate npm
        cache.invalidate(Some("npm")).await;

        // Now the failing repo controls the response → unavailable
        let svc = make_svc_with_cache(FailingExploreRepo::arc(), Arc::clone(&cache));
        let (_, unavailable) = svc.explore_packages(filter).await.unwrap();
        assert!(unavailable);
    }

    // ── list_vulnerabilities ──────────────────────────────────────────────────

    struct OneFindingVulnRepo;

    #[async_trait]
    impl VulnerabilityRepository for OneFindingVulnRepo {
        async fn replace_findings_for_artifact(
            &self,
            _artifact_key: &str,
            _findings: Vec<crate::entities::ArtifactVulnerability>,
        ) -> Result<(), CoreError> {
            Ok(())
        }
        async fn list_for_coordinate(
            &self,
            registry: &str,
            name: &str,
            version: &str,
        ) -> Result<Vec<crate::entities::ArtifactVulnerability>, CoreError> {
            Ok(vec![crate::entities::ArtifactVulnerability {
                id: uuid::Uuid::new_v4(),
                artifact_key: format!("artifact:{registry}/{name}/{version}"),
                registry: registry.to_owned(),
                package_name: name.to_owned(),
                version: version.to_owned(),
                osv_id: "RUSTSEC-2021-0001".to_owned(),
                severity: crate::entities::Severity::High,
                summary: "boom".to_owned(),
                fixed_version: Some("0.3.1".to_owned()),
                purl: format!("pkg:cargo/{name}@{version}"),
                detected_at: Utc::now(),
            }])
        }
    }

    #[tokio::test]
    async fn list_vulnerabilities_empty_without_repo() {
        let svc = AdminService::new(MemRepo::new());
        let out = svc
            .list_vulnerabilities("cargo", "yaml", "0.3.0")
            .await
            .unwrap();
        assert!(out.is_empty(), "no vuln repo attached → empty");
    }

    #[tokio::test]
    async fn list_vulnerabilities_delegates_to_repo() {
        let svc =
            AdminService::new(MemRepo::new()).with_vulnerability_repo(Arc::new(OneFindingVulnRepo));
        let out = svc
            .list_vulnerabilities("cargo", "yaml", "0.3.0")
            .await
            .unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].osv_id, "RUSTSEC-2021-0001");
        assert_eq!(out[0].fixed_version.as_deref(), Some("0.3.1"));
    }
}
