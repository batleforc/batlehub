/// In-memory implementations of all core port traits.
///
/// These are suitable for tests, integration harnesses, and any scenario
/// that does not need persistence. All types are always compiled (no feature
/// gates) and are thread-safe via `tokio::sync::RwLock`.
///
/// Re-exported at the crate root as `batlehub_adapters::in_memory::*`.
use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use chrono::{DateTime, Utc};
use futures::stream;
use tokio::sync::RwLock;
use uuid::Uuid;

use batlehub_core::{
    entities::{
        AccessEvent, EventFilter, PackageFilter, PackageId, PackageStatus, PackageSummary, Role,
    },
    error::CoreError,
    ports::{
        ArtifactMeta, ArtifactMetaRepository, ByteStream, PackageRepository, StorageBackend,
        StoredArtifact, StorageMeta, UserToken, UserTokenRepository,
    },
};

// ── InMemoryPackageRepository ─────────────────────────────────────────────────

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
        let key = event.package_id.cache_key();
        {
            let mut sums = self.summaries.write().await;
            let entry = sums.entry(key).or_insert_with(|| PackageSummary {
                id: Uuid::new_v4(),
                package_id: event.package_id.clone(),
                status: PackageStatus::Available,
                last_accessed: None,
                last_accessed_by: None,
                access_count: 0,
            });
            entry.access_count += 1;
            entry.last_accessed = Some(event.timestamp);
            entry.last_accessed_by = event.user_id.clone();
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
        let entry = sums.entry(pkg.cache_key()).or_insert_with(|| PackageSummary {
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

    async fn list_packages(
        &self,
        filter: PackageFilter,
    ) -> Result<Vec<PackageSummary>, CoreError> {
        let sums = self.summaries.read().await;
        let mut result: Vec<PackageSummary> = sums
            .values()
            .filter(|s| {
                if let Some(ref reg) = filter.registry {
                    if s.package_id.registry != *reg {
                        return false;
                    }
                }
                if !filter.registries.is_empty()
                    && !filter.registries.contains(&s.package_id.registry)
                {
                    return false;
                }
                if let Some(ref needle) = filter.name_contains {
                    if !s.package_id.name.contains(needle.as_str()) {
                        return false;
                    }
                }
                if let Some(ref name) = filter.name_exact {
                    if s.package_id.name != *name {
                        return false;
                    }
                }
                if filter.blocked_only && !s.status.is_blocked() {
                    return false;
                }
                true
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
        let no_page = PackageFilter { limit: 0, offset: 0, ..filter };
        Ok(self.list_packages(no_page).await?.len() as u64)
    }

    async fn list_events(&self, filter: EventFilter) -> Result<Vec<AccessEvent>, CoreError> {
        let events = self.events.read().await;
        let mut result: Vec<AccessEvent> = events
            .iter()
            .filter(|e| {
                if let Some(ref reg) = filter.registry {
                    if e.package_id.registry != *reg {
                        return false;
                    }
                }
                if let Some(ref name) = filter.package_name {
                    if e.package_id.name != *name {
                        return false;
                    }
                }
                if let Some(ref uid) = filter.user_id {
                    if e.user_id.as_deref() != Some(uid.as_str()) {
                        return false;
                    }
                }
                if filter.denied_only && !e.result.is_denied() {
                    return false;
                }
                if let Some(from) = filter.from {
                    if e.timestamp < from {
                        return false;
                    }
                }
                if let Some(to) = filter.to {
                    if e.timestamp > to {
                        return false;
                    }
                }
                true
            })
            .cloned()
            .collect();

        result.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

        let offset = filter.offset as usize;
        if offset > 0 {
            result = result.into_iter().skip(offset).collect();
        }
        if filter.limit > 0 {
            result.truncate(filter.limit as usize);
        }

        Ok(result)
    }
}

// ── InMemoryStorageBackend ────────────────────────────────────────────────────

/// In-memory [`StorageBackend`].
///
/// Stores artifact bytes and [`StorageMeta`] in a `RwLock`-protected hash map.
/// All keys, including those with colons or slashes, are accepted as-is.
#[derive(Debug, Default)]
pub struct InMemoryStorageBackend {
    data: Arc<RwLock<HashMap<String, (Bytes, StorageMeta)>>>,
}

impl InMemoryStorageBackend {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
}

#[async_trait]
impl StorageBackend for InMemoryStorageBackend {
    async fn store(&self, key: &str, data: Bytes, meta: StorageMeta) -> Result<(), CoreError> {
        self.data.write().await.insert(key.to_owned(), (data, meta));
        Ok(())
    }

    async fn retrieve(&self, key: &str) -> Result<Option<StoredArtifact>, CoreError> {
        let map = self.data.read().await;
        Ok(map.get(key).map(|(data, meta)| {
            let bytes = data.clone();
            let s: ByteStream =
                Box::pin(stream::once(async move { Ok::<Bytes, CoreError>(bytes) }));
            StoredArtifact { stream: s, meta: meta.clone() }
        }))
    }

    async fn exists(&self, key: &str) -> Result<bool, CoreError> {
        Ok(self.data.read().await.contains_key(key))
    }

    async fn delete(&self, key: &str) -> Result<(), CoreError> {
        self.data.write().await.remove(key);
        Ok(())
    }

    async fn delete_by_prefix(&self, prefix: &str) -> Result<usize, CoreError> {
        let mut map = self.data.write().await;
        let keys: Vec<String> =
            map.keys().filter(|k| k.starts_with(prefix)).cloned().collect();
        let count = keys.len();
        for k in keys {
            map.remove(&k);
        }
        Ok(count)
    }

    async fn stat_by_prefix(&self, prefix: &str) -> Result<(u64, u64), CoreError> {
        let map = self.data.read().await;
        Ok(map
            .iter()
            .filter(|(k, _)| k.starts_with(prefix))
            .fold((0u64, 0u64), |(count, bytes), (_, (data, meta))| {
                (count + 1, bytes + meta.size.unwrap_or(data.len() as u64))
            }))
    }

    async fn list_keys(&self, prefix: &str) -> Result<Vec<String>, CoreError> {
        Ok(self
            .data
            .read()
            .await
            .keys()
            .filter(|k| k.starts_with(prefix))
            .cloned()
            .collect())
    }
}

// ── NoopArtifactMetaRepository ────────────────────────────────────────────────

/// A no-op [`ArtifactMetaRepository`] that discards all writes and returns
/// empty / non-expired results for all reads.
///
/// Appropriate for tests that exercise proxy or publish paths but do not
/// need eviction or cache-coherence checks.
#[derive(Debug, Default)]
pub struct NoopArtifactMetaRepository;

impl NoopArtifactMetaRepository {
    pub fn arc() -> Arc<dyn ArtifactMetaRepository> {
        Arc::new(Self)
    }
}

#[async_trait]
impl ArtifactMetaRepository for NoopArtifactMetaRepository {
    async fn record_artifact(
        &self,
        _key: &str,
        _registry: &str,
        _package_name: &str,
        _version: &str,
        _size: Option<u64>,
    ) -> Result<(), CoreError> {
        Ok(())
    }

    async fn touch_artifact(&self, _key: &str) -> Result<(), CoreError> {
        Ok(())
    }

    async fn list_artifacts(&self, _registry: &str) -> Result<Vec<ArtifactMeta>, CoreError> {
        Ok(vec![])
    }

    async fn list_artifacts_by_package(&self) -> Result<Vec<ArtifactMeta>, CoreError> {
        Ok(vec![])
    }

    async fn delete_artifact_meta(&self, _key: &str) -> Result<(), CoreError> {
        Ok(())
    }

    async fn is_artifact_expired(
        &self,
        _key: &str,
        _older_than: DateTime<Utc>,
    ) -> Result<bool, CoreError> {
        Ok(false)
    }

    async fn list_expired_by_ttl(
        &self,
        _registry: &str,
        _older_than: DateTime<Utc>,
    ) -> Result<Vec<ArtifactMeta>, CoreError> {
        Ok(vec![])
    }

    async fn list_idle(
        &self,
        _registry: &str,
        _idle_since: DateTime<Utc>,
    ) -> Result<Vec<ArtifactMeta>, CoreError> {
        Ok(vec![])
    }

    async fn total_size_bytes(&self, _registry: &str) -> Result<u64, CoreError> {
        Ok(0)
    }

    async fn list_lru(
        &self,
        _registry: &str,
        _limit: i64,
    ) -> Result<Vec<ArtifactMeta>, CoreError> {
        Ok(vec![])
    }
}

// ── NullUserTokenRepository ───────────────────────────────────────────────────

/// A [`UserTokenRepository`] that rejects token creation and returns empty
/// results for all lookups.
///
/// Appropriate for tests that authenticate via [`StaticTokenAuthProvider`]
/// (static tokens in config) and do not exercise the user-generated-token flow.
///
/// [`StaticTokenAuthProvider`]: crate::auth::StaticTokenAuthProvider
#[derive(Debug, Default)]
pub struct NullUserTokenRepository;

impl NullUserTokenRepository {
    pub fn arc() -> Arc<dyn UserTokenRepository> {
        Arc::new(Self)
    }
}

#[async_trait]
impl UserTokenRepository for NullUserTokenRepository {
    async fn create_token(
        &self,
        _id: Uuid,
        _user_id: &str,
        _name: &str,
        _token_hash: &str,
        _role: Role,
        _expires_at: DateTime<Utc>,
    ) -> Result<UserToken, CoreError> {
        Err(CoreError::Database(
            "NullUserTokenRepository does not support token creation".into(),
        ))
    }

    async fn find_by_hash(&self, _token_hash: &str) -> Result<Option<UserToken>, CoreError> {
        Ok(None)
    }

    async fn list_for_user(&self, _user_id: &str) -> Result<Vec<UserToken>, CoreError> {
        Ok(vec![])
    }

    async fn revoke(&self, _id: Uuid, _user_id: &str) -> Result<bool, CoreError> {
        Ok(false)
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use chrono::Utc;

    use batlehub_core::{
        entities::{
            AccessEvent, EventFilter, PackageFilter, PackageId, PackageStatus, Role,
        },
        ports::{PackageRepository, StorageBackend, StorageMeta},
    };

    use super::{InMemoryPackageRepository, InMemoryStorageBackend};

    // ── helpers ────────────────────────────────────────────────────────────────

    fn pkg_id(registry: &str, name: &str) -> PackageId {
        PackageId::new(registry, name, "1.0.0")
    }

    fn allow_event(registry: &str, name: &str) -> AccessEvent {
        AccessEvent::allowed_download(pkg_id(registry, name), Some("user".to_owned()), Role::User)
    }

    fn meta(size: u64) -> StorageMeta {
        StorageMeta { size: Some(size), content_type: None, checksum: None }
    }

    // ── PackageRepository ──────────────────────────────────────────────────────

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
        repo.set_status(&pkg_id("reg", "foo"), blocked).await.unwrap();
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
        repo.record_access(allow_event("reg-a", "foo")).await.unwrap();
        repo.record_access(allow_event("reg-b", "bar")).await.unwrap();

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
            .list_packages(PackageFilter { limit: 2, offset: 1, ..Default::default() })
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
        assert!(events.iter().all(|e| e.package_id.name == "foo"));
    }

    #[tokio::test]
    async fn list_events_paginates() {
        let repo = InMemoryPackageRepository::new();
        for _ in 0..5 {
            repo.record_access(allow_event("reg", "foo")).await.unwrap();
        }

        let page = repo
            .list_events(EventFilter { limit: 2, offset: 1, ..Default::default() })
            .await
            .unwrap();
        assert_eq!(page.len(), 2);
    }

    // ── StorageBackend ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn store_then_retrieve_round_trips() {
        let s = InMemoryStorageBackend::new();
        s.store("k", Bytes::from("hello"), meta(5)).await.unwrap();
        let artifact = s.retrieve("k").await.unwrap().expect("should exist");
        assert_eq!(artifact.meta.size, Some(5));
    }

    #[tokio::test]
    async fn retrieve_missing_returns_none() {
        let s = InMemoryStorageBackend::new();
        assert!(s.retrieve("nope").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn exists_before_and_after_store() {
        let s = InMemoryStorageBackend::new();
        assert!(!s.exists("k").await.unwrap());
        s.store("k", Bytes::from("x"), meta(1)).await.unwrap();
        assert!(s.exists("k").await.unwrap());
    }

    #[tokio::test]
    async fn delete_removes_key() {
        let s = InMemoryStorageBackend::new();
        s.store("k", Bytes::from("x"), meta(1)).await.unwrap();
        s.delete("k").await.unwrap();
        assert!(!s.exists("k").await.unwrap());
    }

    #[tokio::test]
    async fn delete_by_prefix_removes_matching_only() {
        let s = InMemoryStorageBackend::new();
        for key in ["p/a", "p/b", "q/c"] {
            s.store(key, Bytes::from("x"), meta(1)).await.unwrap();
        }
        assert_eq!(s.delete_by_prefix("p/").await.unwrap(), 2);
        assert!(!s.exists("p/a").await.unwrap());
        assert!(s.exists("q/c").await.unwrap());
    }

    #[tokio::test]
    async fn stat_by_prefix_sums_sizes() {
        let s = InMemoryStorageBackend::new();
        s.store("p/a", Bytes::from("abc"), meta(3)).await.unwrap();
        s.store("p/b", Bytes::from("de"), meta(2)).await.unwrap();
        s.store("q/c", Bytes::from("f"), meta(1)).await.unwrap();
        let (count, bytes) = s.stat_by_prefix("p/").await.unwrap();
        assert_eq!((count, bytes), (2, 5));
    }

    #[tokio::test]
    async fn list_keys_returns_prefix_matches() {
        let s = InMemoryStorageBackend::new();
        for key in ["ns/x", "ns/y", "other/z"] {
            s.store(key, Bytes::from("v"), meta(1)).await.unwrap();
        }
        let mut keys = s.list_keys("ns/").await.unwrap();
        keys.sort();
        assert_eq!(keys, vec!["ns/x", "ns/y"]);
    }
}
