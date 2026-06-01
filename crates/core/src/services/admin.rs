use std::sync::Arc;

use chrono::Utc;

use crate::entities::{
    AccessEvent, AccessResult, EventFilter, ExploreEntry, ExploreFilter, Identity, PackageFilter,
    PackageId, PackageStatus, PackageSummary, RegistryStat,
};
use crate::error::CoreError;
use crate::ports::PackageRepository;
use crate::services::explore_cache::{
    packages_cache_key, packages_entry_registries, stats_cache_key, ExploreCache,
};

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
}

impl AdminService {
    pub fn new(repo: Arc<dyn PackageRepository>) -> Self {
        Self {
            repo,
            explore_cache: Arc::new(ExploreCache::new()),
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

        self.repo
            .record_access(AccessEvent {
                id: uuid::Uuid::new_v4(),
                user_id: by_identity.user_id.clone(),
                user_role: by_identity.role.clone(),
                package_id: pkg.clone(),
                action: crate::entities::AccessAction::Block,
                result: AccessResult::Allowed,
                timestamp: Utc::now(),
            })
            .await
            .unwrap_or_else(|e| tracing::warn!(error = %e, "failed to record block action"));

        tracing::info!(package = %pkg, blocked_by = %blocked_by, reason = %reason, "package blocked");
        Ok(())
    }

    pub async fn unblock_package(
        &self,
        pkg: &PackageId,
        by_identity: &Identity,
    ) -> Result<(), CoreError> {
        self.repo.set_status(pkg, PackageStatus::Available).await?;

        self.repo
            .record_access(AccessEvent {
                id: uuid::Uuid::new_v4(),
                user_id: by_identity.user_id.clone(),
                user_role: by_identity.role.clone(),
                package_id: pkg.clone(),
                action: crate::entities::AccessAction::Unblock,
                result: AccessResult::Allowed,
                timestamp: Utc::now(),
            })
            .await
            .unwrap_or_else(|e| tracing::warn!(error = %e, "failed to record unblock action"));

        tracing::info!(package = %pkg, "package unblocked");
        Ok(())
    }

    pub async fn bulk_block_packages(
        &self,
        items: Vec<BulkBlockItem>,
        by_identity: &Identity,
    ) -> BulkActionResult {
        let mut result = BulkActionResult {
            succeeded: vec![],
            failed: vec![],
        };
        for item in items {
            match self
                .block_package(&item.package_id, item.reason, by_identity)
                .await
            {
                Ok(()) => result.succeeded.push(item.package_id),
                Err(e) => result.failed.push((item.package_id, e.to_string())),
            }
        }
        result
    }

    pub async fn bulk_unblock_packages(
        &self,
        items: Vec<PackageId>,
        by_identity: &Identity,
    ) -> BulkActionResult {
        let mut result = BulkActionResult {
            succeeded: vec![],
            failed: vec![],
        };
        for pkg in items {
            match self.unblock_package(&pkg, by_identity).await {
                Ok(()) => result.succeeded.push(pkg),
                Err(e) => result.failed.push((pkg, e.to_string())),
            }
        }
        result
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

    pub async fn get_package_status(&self, pkg: &PackageId) -> Result<PackageStatus, CoreError> {
        self.repo.get_status(pkg).await
    }

    /// Returns `(entries, upstream_unavailable)`.
    ///
    /// `upstream_unavailable` is `true` only when the DB is unreachable **and** no
    /// stale cache entry exists. When stale data is served the flag stays `false` so
    /// callers can distinguish "stale but usable" from "nothing at all".
    pub async fn explore_packages(
        &self,
        filter: ExploreFilter,
    ) -> Result<(Vec<ExploreEntry>, bool), CoreError> {
        let key = packages_cache_key(&filter);
        if let Some((items, _count)) = self.explore_cache.get_packages(&key).await {
            return Ok((items, false));
        }
        match self.repo.explore_packages(filter.clone()).await {
            Ok(items) => {
                let regs = packages_entry_registries(&filter);
                self.explore_cache
                    .set_packages(&key, items.clone(), 0, regs)
                    .await;
                Ok((items, false))
            }
            Err(_) => {
                if let Some((items, _)) = self.explore_cache.get_stale_packages(&key).await {
                    Ok((items, false))
                } else {
                    Ok((vec![], true))
                }
            }
        }
    }

    /// Returns `(count, upstream_unavailable)`.
    pub async fn count_explore_packages(
        &self,
        filter: ExploreFilter,
    ) -> Result<(u64, bool), CoreError> {
        let key = packages_cache_key(&filter);
        if let Some((_items, count)) = self.explore_cache.get_packages(&key).await {
            return Ok((count, false));
        }
        match self.repo.count_explore_packages(filter.clone()).await {
            Ok(count) => {
                let regs = packages_entry_registries(&filter);
                self.explore_cache
                    .set_packages(&key, vec![], count, regs)
                    .await;
                Ok((count, false))
            }
            Err(_) => {
                if let Some((_, count)) = self.explore_cache.get_stale_packages(&key).await {
                    Ok((count, false))
                } else {
                    Ok((0, true))
                }
            }
        }
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
        match self.repo.registry_explore_stats(accessible_registries).await {
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
            Arc::new(Self { entries, count, stats })
        }
    }

    #[async_trait]
    impl PackageRepository for StubExploreRepo {
        async fn record_access(&self, _: AccessEvent) -> Result<(), CoreError> { Ok(()) }
        async fn get_status(&self, _: &PackageId) -> Result<PackageStatus, CoreError> {
            Ok(PackageStatus::Available)
        }
        async fn set_status(&self, _: &PackageId, _: PackageStatus) -> Result<(), CoreError> {
            Ok(())
        }
        async fn list_packages(&self, _: PackageFilter) -> Result<Vec<PackageSummary>, CoreError> {
            Ok(vec![])
        }
        async fn count_packages(&self, _: PackageFilter) -> Result<u64, CoreError> { Ok(0) }
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
    }

    /// A repo whose explore methods always return an error.
    struct FailingExploreRepo;

    impl FailingExploreRepo {
        fn arc() -> Arc<Self> { Arc::new(Self) }
    }

    #[async_trait]
    impl PackageRepository for FailingExploreRepo {
        async fn record_access(&self, _: AccessEvent) -> Result<(), CoreError> { Ok(()) }
        async fn get_status(&self, _: &PackageId) -> Result<PackageStatus, CoreError> {
            Ok(PackageStatus::Available)
        }
        async fn set_status(&self, _: &PackageId, _: PackageStatus) -> Result<(), CoreError> {
            Ok(())
        }
        async fn list_packages(&self, _: PackageFilter) -> Result<Vec<PackageSummary>, CoreError> {
            Ok(vec![])
        }
        async fn count_packages(&self, _: PackageFilter) -> Result<u64, CoreError> { Ok(0) }
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
        RegistryStat { registry: "npm".into(), package_count: 1, total_downloads: 10 }
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
        AdminService { repo, explore_cache: cache }
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
        let key = crate::services::explore_cache::packages_cache_key(&filter);
        cache.set_packages(&key, vec![sample_entry()], 1, vec!["npm".into()]).await;

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
        let key = crate::services::explore_cache::packages_cache_key(&filter);
        cache.set_packages(&key, vec![sample_entry()], 1, vec!["npm".into()]).await;

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
        let key = crate::services::explore_cache::packages_cache_key(&filter);
        // set_packages stores the count too
        cache.set_packages(&key, vec![], 42, vec!["npm".into()]).await;

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
        let key = crate::services::explore_cache::packages_cache_key(&filter);
        cache.set_packages(&key, vec![], 7, vec!["npm".into()]).await;

        let svc = make_svc_with_cache(FailingExploreRepo::arc(), Arc::clone(&cache));
        let (count, unavailable) = svc.count_explore_packages(filter).await.unwrap();
        assert!(!unavailable);
        assert_eq!(count, 7);
    }

    // ── registry_explore_stats ────────────────────────────────────────────────

    #[tokio::test]
    async fn registry_explore_stats_cache_hit_skips_repo() {
        let cache = Arc::new(ExploreCache::new());
        let regs = vec!["npm".to_string()];
        let key = crate::services::explore_cache::stats_cache_key(&regs);
        cache.set_stats(&key, vec![sample_stat()], regs.clone()).await;

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
        cache.set_stats(&key, vec![sample_stat()], regs.clone()).await;

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
        let key = crate::services::explore_cache::packages_cache_key(&filter);
        cache.set_packages(&key, vec![sample_entry()], 1, vec!["npm".into()]).await;

        // Invalidate npm
        cache.invalidate(Some("npm")).await;

        // Now the failing repo controls the response → unavailable
        let svc = make_svc_with_cache(FailingExploreRepo::arc(), Arc::clone(&cache));
        let (_, unavailable) = svc.explore_packages(filter).await.unwrap();
        assert!(unavailable);
    }
}
