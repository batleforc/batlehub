use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use tokio::sync::RwLock;
use uuid::Uuid;

use batlehub_core::{
    entities::{
        AccessAction, AccessEvent, AccessResult, EventFilter, PackageFilter, PackageId,
        PackageStatus, PackageSummary,
    },
    error::CoreError,
    ports::{PackageRepository, RecentErrorRecord},
};

/// In-memory [`PackageRepository`].
///
/// Stores package summaries keyed by [`PackageId::cache_key`] and access
/// events in an append-only `Vec`. `list_packages` and `list_events` honour
/// all filter fields including pagination (`limit` / `offset`).
/// A `limit` of `0` is treated as "no limit".
#[derive(Debug, Default)]
pub struct InMemoryPackageRepository {
    summaries: Arc<RwLock<HashMap<String, PackageSummary>>>,
    events: Arc<RwLock<Vec<AccessEvent>>>,
}

impl InMemoryPackageRepository {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
}

#[async_trait]
impl PackageRepository for InMemoryPackageRepository {
    async fn record_access(&self, event: AccessEvent) -> Result<(), CoreError> {
        // Only actions that always carry a real, version-specific package
        // coordinate should create/update a `PackageSummary` row. Ownership,
        // visibility, and account-wide actions (package_id: None, or Delete
        // which just removed the row) must not spuriously create one — this
        // mirrors the Postgres adapter's `creates_status_row` guard.
        let updates_summary = matches!(
            event.action,
            AccessAction::Download
                | AccessAction::ViewMetadata
                | AccessAction::Block
                | AccessAction::Unblock
        );
        if updates_summary {
            if let Some(pkg) = &event.package_id {
                let mut sums = self.summaries.write().await;
                let entry = sums
                    .entry(pkg.cache_key())
                    .or_insert_with(|| PackageSummary {
                        id: Uuid::new_v4(),
                        package_id: pkg.clone(),
                        status: PackageStatus::Available,
                        last_accessed: None,
                        last_accessed_by: None,
                        access_count: 0,
                    });
                entry.access_count += 1;
                entry.last_accessed = Some(event.timestamp);
                entry.last_accessed_by = event.user_id.clone();
            }
        }
        self.events.write().await.push(event);
        Ok(())
    }

    async fn get_status(&self, pkg: &PackageId) -> Result<PackageStatus, CoreError> {
        Ok(self
            .summaries
            .read()
            .await
            .get(&pkg.cache_key())
            .map(|s| s.status.clone())
            .unwrap_or(PackageStatus::Available))
    }

    async fn set_status(&self, pkg: &PackageId, status: PackageStatus) -> Result<(), CoreError> {
        let mut sums = self.summaries.write().await;
        let entry = sums
            .entry(pkg.cache_key())
            .or_insert_with(|| PackageSummary {
                id: Uuid::new_v4(),
                package_id: pkg.clone(),
                status: PackageStatus::Available,
                last_accessed: None,
                last_accessed_by: None,
                access_count: 0,
            });
        entry.status = status;
        Ok(())
    }

    async fn delete_package(&self, pkg: &PackageId) -> Result<bool, CoreError> {
        let removed = self.summaries.write().await.remove(&pkg.cache_key());
        Ok(removed.is_some())
    }

    async fn list_packages(&self, filter: PackageFilter) -> Result<Vec<PackageSummary>, CoreError> {
        let sums = self.summaries.read().await;
        let mut result: Vec<PackageSummary> = sums
            .values()
            .filter(|s| {
                filter
                    .registry
                    .as_ref()
                    .is_none_or(|r| s.package_id.registry == *r)
                    && (filter.registries.is_empty()
                        || filter.registries.contains(&s.package_id.registry))
                    && filter
                        .name_contains
                        .as_ref()
                        .is_none_or(|n| s.package_id.name.contains(n.as_str()))
                    && filter
                        .name_exact
                        .as_ref()
                        .is_none_or(|n| s.package_id.name == *n)
                    && (!filter.blocked_only || s.status.is_blocked())
            })
            .cloned()
            .collect();

        result.sort_by(|a, b| {
            b.last_accessed
                .unwrap_or(DateTime::<Utc>::MIN_UTC)
                .cmp(&a.last_accessed.unwrap_or(DateTime::<Utc>::MIN_UTC))
        });

        let offset = filter.offset as usize;
        if offset > 0 {
            result = result.into_iter().skip(offset).collect();
        }
        if filter.limit > 0 {
            result.truncate(filter.limit as usize);
        }

        Ok(result)
    }

    async fn count_packages(&self, filter: PackageFilter) -> Result<u64, CoreError> {
        let no_page = PackageFilter {
            limit: 0,
            offset: 0,
            ..filter
        };
        Ok(self.list_packages(no_page).await?.len() as u64)
    }

    async fn list_events(&self, filter: EventFilter) -> Result<Vec<AccessEvent>, CoreError> {
        let events = self.events.read().await;
        let mut result: Vec<AccessEvent> = events
            .iter()
            .filter(|e| {
                filter
                    .registry
                    .as_ref()
                    .is_none_or(|r| e.package_id.as_ref().is_some_and(|p| p.registry == *r))
                    && filter
                        .package_name
                        .as_ref()
                        .is_none_or(|n| e.package_id.as_ref().is_some_and(|p| p.name == *n))
                    && filter
                        .user_id
                        .as_ref()
                        .is_none_or(|u| e.user_id.as_deref() == Some(u.as_str()))
                    && (!filter.denied_only || e.result.is_denied())
                    && filter.from.is_none_or(|from| e.timestamp >= from)
                    && filter.to.is_none_or(|to| e.timestamp <= to)
            })
            .cloned()
            .collect();

        result.sort_by_key(|b| std::cmp::Reverse(b.timestamp));

        let offset = filter.offset as usize;
        if offset > 0 {
            result = result.into_iter().skip(offset).collect();
        }
        if filter.limit > 0 {
            result.truncate(filter.limit as usize);
        }

        Ok(result)
    }

    async fn registry_package_counts(
        &self,
        registries: &[String],
    ) -> Result<HashMap<String, i64>, CoreError> {
        let sums = self.summaries.read().await;
        let mut counts: HashMap<String, i64> = HashMap::new();
        for s in sums.values() {
            if registries.contains(&s.package_id.registry) {
                *counts.entry(s.package_id.registry.clone()).or_insert(0) += 1;
            }
        }
        Ok(counts)
    }

    async fn registry_event_stats(
        &self,
        registries: &[String],
    ) -> Result<HashMap<String, (Option<DateTime<Utc>>, i64, i64)>, CoreError> {
        let events = self.events.read().await;
        let now = Utc::now();
        let mut stats: HashMap<String, (Option<DateTime<Utc>>, i64, i64)> = HashMap::new();
        for e in events.iter() {
            if !matches!(e.action, AccessAction::Download)
                || !matches!(e.result, AccessResult::Allowed)
            {
                continue;
            }
            let Some(registry) = e.package_id.as_ref().map(|p| p.registry.clone()) else {
                continue;
            };
            if !registries.contains(&registry) {
                continue;
            }
            let entry = stats.entry(registry).or_insert((None, 0, 0));
            entry.0 = Some(entry.0.map_or(e.timestamp, |cur| cur.max(e.timestamp)));
            if now - e.timestamp <= chrono::Duration::hours(1) {
                entry.1 += 1;
            }
            if now - e.timestamp <= chrono::Duration::days(1) {
                entry.2 += 1;
            }
        }
        Ok(stats)
    }

    async fn recent_registry_errors(
        &self,
        registry: &str,
        limit: i64,
    ) -> Result<Vec<RecentErrorRecord>, CoreError> {
        let events = self.events.read().await;
        let now = Utc::now();
        let mut errors: Vec<RecentErrorRecord> = events
            .iter()
            .filter(|e| {
                e.package_id
                    .as_ref()
                    .is_some_and(|p| p.registry == registry)
                    && matches!(
                        e.result,
                        AccessResult::Denied { .. } | AccessResult::ProxyError { .. }
                    )
                    && now - e.timestamp <= chrono::Duration::hours(24)
            })
            .map(|e| {
                let (outcome, deny_reason) = match &e.result {
                    AccessResult::Denied { reason } => ("denied".to_owned(), Some(reason.clone())),
                    AccessResult::ProxyError { reason } => {
                        ("error".to_owned(), Some(reason.clone()))
                    }
                    AccessResult::Allowed => unreachable!("filtered out above"),
                };
                RecentErrorRecord {
                    created_at: e.timestamp,
                    user_id: e.user_id.clone(),
                    package_name: e
                        .package_id
                        .as_ref()
                        .map(|p| p.name.clone())
                        .unwrap_or_default(),
                    package_version: e
                        .package_id
                        .as_ref()
                        .map(|p| p.version.clone())
                        .unwrap_or_default(),
                    outcome,
                    deny_reason,
                }
            })
            .collect();

        errors.sort_by_key(|e| std::cmp::Reverse(e.created_at));
        if limit >= 0 {
            errors.truncate(limit as usize);
        }
        Ok(errors)
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use batlehub_core::{
        entities::{AccessEvent, EventFilter, PackageFilter, PackageId, PackageStatus, Role},
        ports::PackageRepository,
    };

    use super::InMemoryPackageRepository;

    fn pkg_id(registry: &str, name: &str) -> PackageId {
        PackageId::new(registry, name, "1.0.0")
    }

    fn allow_event(registry: &str, name: &str) -> AccessEvent {
        AccessEvent::allowed_download(pkg_id(registry, name), Some("user".to_owned()), Role::User)
    }

    #[tokio::test]
    async fn get_status_returns_available_for_unknown_package() {
        let repo = InMemoryPackageRepository::new();
        let status = repo.get_status(&pkg_id("reg", "foo")).await.unwrap();
        assert!(matches!(status, PackageStatus::Available));
    }

    #[tokio::test]
    async fn set_then_get_status_round_trips() {
        let repo = InMemoryPackageRepository::new();
        let blocked = PackageStatus::Blocked {
            reason: "test".to_owned(),
            blocked_by: "admin".to_owned(),
            blocked_at: Utc::now(),
        };
        repo.set_status(&pkg_id("reg", "foo"), blocked)
            .await
            .unwrap();
        let status = repo.get_status(&pkg_id("reg", "foo")).await.unwrap();
        assert!(status.is_blocked());
    }

    #[tokio::test]
    async fn record_access_increments_count() {
        let repo = InMemoryPackageRepository::new();
        repo.record_access(allow_event("reg", "foo")).await.unwrap();
        repo.record_access(allow_event("reg", "foo")).await.unwrap();

        let pkgs = repo
            .list_packages(PackageFilter {
                registry: Some("reg".to_owned()),
                name_exact: Some("foo".to_owned()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(pkgs.len(), 1);
        assert_eq!(pkgs[0].access_count, 2);
        assert!(pkgs[0].last_accessed.is_some());
    }

    #[tokio::test]
    async fn list_packages_filters_by_registry() {
        let repo = InMemoryPackageRepository::new();
        repo.record_access(allow_event("reg-a", "foo"))
            .await
            .unwrap();
        repo.record_access(allow_event("reg-b", "bar"))
            .await
            .unwrap();

        let result = repo
            .list_packages(PackageFilter {
                registry: Some("reg-a".to_owned()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].package_id.registry, "reg-a");
    }

    #[tokio::test]
    async fn list_packages_name_contains_filter() {
        let repo = InMemoryPackageRepository::new();
        for name in ["my-lib", "my-app", "other"] {
            repo.record_access(allow_event("reg", name)).await.unwrap();
        }
        let result = repo
            .list_packages(PackageFilter {
                name_contains: Some("my".to_owned()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(result.len(), 2);
    }

    #[tokio::test]
    async fn list_packages_pagination() {
        let repo = InMemoryPackageRepository::new();
        for name in ["a", "b", "c", "d", "e"] {
            repo.record_access(allow_event("reg", name)).await.unwrap();
        }

        let page = repo
            .list_packages(PackageFilter {
                limit: 2,
                offset: 1,
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(page.len(), 2);
    }

    #[tokio::test]
    async fn count_packages_matches_unfiltered_total() {
        let repo = InMemoryPackageRepository::new();
        for name in ["a", "b", "c"] {
            repo.record_access(allow_event("reg", name)).await.unwrap();
        }
        let count = repo
            .count_packages(PackageFilter {
                registry: Some("reg".to_owned()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(count, 3);
    }

    #[tokio::test]
    async fn list_events_filters_by_package_name() {
        let repo = InMemoryPackageRepository::new();
        for _ in 0..3 {
            repo.record_access(allow_event("reg", "foo")).await.unwrap();
        }
        repo.record_access(allow_event("reg", "bar")).await.unwrap();

        let events = repo
            .list_events(EventFilter {
                registry: Some("reg".to_owned()),
                package_name: Some("foo".to_owned()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(events.len(), 3);
        assert!(events
            .iter()
            .all(|e| e.package_id.as_ref().is_some_and(|p| p.name == "foo")));
    }

    #[tokio::test]
    async fn list_events_paginates() {
        let repo = InMemoryPackageRepository::new();
        for _ in 0..5 {
            repo.record_access(allow_event("reg", "foo")).await.unwrap();
        }

        let page = repo
            .list_events(EventFilter {
                limit: 2,
                offset: 1,
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(page.len(), 2);
    }

    // ── account-wide events (package_id: None) ────────────────────────────────

    use batlehub_core::entities::AccessAction;

    fn account_event(action: AccessAction) -> AccessEvent {
        AccessEvent {
            id: uuid::Uuid::new_v4(),
            user_id: Some("admin".to_owned()),
            user_role: Role::Admin,
            package_id: None,
            action,
            result: batlehub_core::entities::AccessResult::Allowed,
            timestamp: Utc::now(),
            ip_address: None,
            user_agent: None,
        }
    }

    #[tokio::test]
    async fn record_access_accepts_account_wide_event_with_no_package() {
        let repo = InMemoryPackageRepository::new();
        repo.record_access(account_event(AccessAction::BlockUser))
            .await
            .unwrap();

        let events = repo.list_events(EventFilter::new()).await.unwrap();
        assert_eq!(events.len(), 1);
        assert!(events[0].package_id.is_none());
        assert!(matches!(events[0].action, AccessAction::BlockUser));
    }

    #[tokio::test]
    async fn account_wide_event_does_not_create_a_package_summary() {
        let repo = InMemoryPackageRepository::new();
        repo.record_access(account_event(AccessAction::BlockIp))
            .await
            .unwrap();

        let pkgs = repo.list_packages(PackageFilter::new()).await.unwrap();
        assert!(
            pkgs.is_empty(),
            "an account-wide event must not fabricate a package_statuses row"
        );
    }

    #[tokio::test]
    async fn list_events_with_registry_filter_excludes_account_wide_events() {
        let repo = InMemoryPackageRepository::new();
        repo.record_access(allow_event("reg", "foo")).await.unwrap();
        repo.record_access(account_event(AccessAction::UnblockUser))
            .await
            .unwrap();

        let events = repo
            .list_events(EventFilter {
                registry: Some("reg".to_owned()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(
            events.len(),
            1,
            "account-wide event has no registry to match"
        );
        assert!(events[0].package_id.is_some());
    }
}
