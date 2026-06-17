use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use bytes::Bytes;
use chrono::Utc;
use futures::stream;

use std::time::Duration;

use super::*;
use crate::entities::{
    AccessEvent, AccessResult, EventFilter, Identity, PackageFilter, PackageId, PackageMetadata,
    PackageStatus, PackageSummary,
};
use crate::ports::ByteStream;
use crate::ports::{
    ArtifactMeta, ArtifactMetaRepository, CacheStore, FetchedArtifact, PackageRepository,
    RegistryClient, StorageBackend, StorageMeta, StoredArtifact,
};
use crate::services::hot_config::{new_hot_lock, HotConfig, RegistryPolicy};
use crate::services::metrics::ProxyMetrics;

fn make_hot(
    registry_name: &str,
    client: Arc<dyn RegistryClient>,
    policy: RegistryPolicy,
    max_bytes: Option<u64>,
) -> crate::services::hot_config::HotConfigLock {
    let mut registries = HashMap::new();
    registries.insert(registry_name.to_owned(), client);
    let mut policies = HashMap::new();
    policies.insert(registry_name.to_owned(), Arc::new(policy));
    new_hot_lock(HotConfig {
        registries,
        policies,
        max_artifact_size_bytes: max_bytes,
        ..Default::default()
    })
}

fn empty_hot(
    registry_name: &str,
    client: Arc<dyn RegistryClient>,
) -> crate::services::hot_config::HotConfigLock {
    let mut registries = HashMap::new();
    registries.insert(registry_name.to_owned(), client);
    new_hot_lock(HotConfig {
        registries,
        policies: HashMap::new(),
        ..Default::default()
    })
}

// ── Minimal in-memory mocks ───────────────────────────────────────────────

struct NoopArtifactMeta;
impl NoopArtifactMeta {
    fn arc() -> Arc<dyn ArtifactMetaRepository> {
        Arc::new(Self)
    }
}
#[async_trait]
impl ArtifactMetaRepository for NoopArtifactMeta {
    async fn record_artifact(
        &self,
        _key: &str,
        _registry: &str,
        _name: &str,
        _ver: &str,
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
        _older_than: chrono::DateTime<chrono::Utc>,
    ) -> Result<bool, CoreError> {
        Ok(false)
    }
    async fn list_expired_by_ttl(
        &self,
        _registry: &str,
        _older_than: chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<ArtifactMeta>, CoreError> {
        Ok(vec![])
    }
    async fn list_idle(
        &self,
        _registry: &str,
        _idle_since: chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<ArtifactMeta>, CoreError> {
        Ok(vec![])
    }
    async fn total_size_bytes(&self, _registry: &str) -> Result<u64, CoreError> {
        Ok(0)
    }
    async fn list_lru(&self, _registry: &str, _limit: i64) -> Result<Vec<ArtifactMeta>, CoreError> {
        Ok(vec![])
    }
}

/// Records `record_artifact` and `touch_artifact` calls; returns configurable
/// expired-artifact list from `list_expired_by_ttl`.
struct SpyArtifactMeta {
    recorded: Mutex<Vec<String>>,
    touched: Mutex<Vec<String>>,
    expired: Mutex<Vec<ArtifactMeta>>,
}
impl SpyArtifactMeta {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            recorded: Mutex::new(vec![]),
            touched: Mutex::new(vec![]),
            expired: Mutex::new(vec![]),
        })
    }
    fn with_expired(expired: Vec<ArtifactMeta>) -> Arc<Self> {
        Arc::new(Self {
            recorded: Mutex::new(vec![]),
            touched: Mutex::new(vec![]),
            expired: Mutex::new(expired),
        })
    }
    fn recorded_keys(&self) -> Vec<String> {
        self.recorded.lock().unwrap().clone()
    }
    fn touched_keys(&self) -> Vec<String> {
        self.touched.lock().unwrap().clone()
    }
}
#[async_trait]
impl ArtifactMetaRepository for SpyArtifactMeta {
    async fn record_artifact(
        &self,
        key: &str,
        _: &str,
        _: &str,
        _: &str,
        _: Option<u64>,
    ) -> Result<(), CoreError> {
        self.recorded.lock().unwrap().push(key.to_owned());
        Ok(())
    }
    async fn touch_artifact(&self, key: &str) -> Result<(), CoreError> {
        self.touched.lock().unwrap().push(key.to_owned());
        Ok(())
    }
    async fn list_artifacts(&self, _: &str) -> Result<Vec<ArtifactMeta>, CoreError> {
        Ok(vec![])
    }
    async fn list_artifacts_by_package(&self) -> Result<Vec<ArtifactMeta>, CoreError> {
        Ok(vec![])
    }
    async fn delete_artifact_meta(&self, _: &str) -> Result<(), CoreError> {
        Ok(())
    }
    async fn is_artifact_expired(
        &self,
        key: &str,
        older_than: chrono::DateTime<chrono::Utc>,
    ) -> Result<bool, CoreError> {
        // If the key has been "recorded" (has metadata) and is NOT explicitly in the
        // expired list, treat it as fresh.  If no metadata has been recorded, treat
        // it as expired (matches PgArtifactMetaRepository semantics: missing row → expired).
        let recorded = self.recorded.lock().unwrap();
        let expired = self.expired.lock().unwrap();
        let has_meta =
            recorded.contains(&key.to_owned()) || expired.iter().any(|m| m.artifact_key == key);
        if !has_meta {
            return Ok(true);
        }
        let is_expired = expired
            .iter()
            .any(|m| m.artifact_key == key && m.cached_at < older_than);
        Ok(is_expired)
    }
    async fn list_expired_by_ttl(
        &self,
        _: &str,
        _: chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<ArtifactMeta>, CoreError> {
        Ok(self.expired.lock().unwrap().clone())
    }
    async fn list_idle(
        &self,
        _: &str,
        _: chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<ArtifactMeta>, CoreError> {
        Ok(vec![])
    }
    async fn total_size_bytes(&self, _: &str) -> Result<u64, CoreError> {
        Ok(0)
    }
    async fn list_lru(&self, _: &str, _: i64) -> Result<Vec<ArtifactMeta>, CoreError> {
        Ok(vec![])
    }
}

struct TestCacheStore {
    data: Mutex<HashMap<String, crate::ports::CacheEntry>>,
}

impl TestCacheStore {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            data: Mutex::new(HashMap::new()),
        })
    }

    fn seed_expired(&self, key: &str, metadata: PackageMetadata) {
        let entry = crate::ports::CacheEntry {
            metadata,
            cached_at: Utc::now() - chrono::Duration::hours(2),
            expires_at: Some(Utc::now() - chrono::Duration::hours(1)),
        };
        self.data.lock().unwrap().insert(key.to_owned(), entry);
    }
}

#[async_trait]
impl CacheStore for TestCacheStore {
    async fn get(&self, key: &str) -> Result<Option<crate::ports::CacheEntry>, CoreError> {
        let map = self.data.lock().unwrap();
        Ok(map.get(key).filter(|e| !e.is_expired()).cloned())
    }
    async fn set(
        &self,
        key: &str,
        mut entry: crate::ports::CacheEntry,
        ttl: Option<std::time::Duration>,
    ) -> Result<(), CoreError> {
        if let Some(ttl) = ttl {
            entry.expires_at =
                Some(Utc::now() + chrono::Duration::from_std(ttl).unwrap_or_default());
        }
        self.data.lock().unwrap().insert(key.to_owned(), entry);
        Ok(())
    }
    async fn invalidate(&self, key: &str) -> Result<(), CoreError> {
        self.data.lock().unwrap().remove(key);
        Ok(())
    }
    async fn get_stale(&self, key: &str) -> Result<Option<crate::ports::CacheEntry>, CoreError> {
        Ok(self.data.lock().unwrap().get(key).cloned())
    }
}

struct SpyRepo {
    events: Mutex<Vec<AccessEvent>>,
}

impl SpyRepo {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            events: Mutex::new(vec![]),
        })
    }

    fn events(&self) -> Vec<AccessEvent> {
        self.events.lock().unwrap().clone()
    }
}

#[async_trait]
impl PackageRepository for SpyRepo {
    async fn record_access(&self, event: AccessEvent) -> Result<(), CoreError> {
        self.events.lock().unwrap().push(event);
        Ok(())
    }
    async fn get_status(&self, _pkg: &PackageId) -> Result<PackageStatus, CoreError> {
        Ok(PackageStatus::Available)
    }
    async fn set_status(&self, _pkg: &PackageId, _status: PackageStatus) -> Result<(), CoreError> {
        Ok(())
    }
    async fn list_packages(
        &self,
        _filter: PackageFilter,
    ) -> Result<Vec<PackageSummary>, CoreError> {
        Ok(vec![])
    }
    async fn count_packages(&self, _filter: PackageFilter) -> Result<u64, CoreError> {
        Ok(0)
    }
    async fn list_events(&self, _filter: EventFilter) -> Result<Vec<AccessEvent>, CoreError> {
        Ok(self.events.lock().unwrap().clone())
    }
}

struct MemStorage {
    data: Mutex<HashMap<String, Bytes>>,
}

impl MemStorage {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            data: Mutex::new(HashMap::new()),
        })
    }
}

#[async_trait]
impl StorageBackend for MemStorage {
    async fn store(&self, key: &str, data: Bytes, _meta: StorageMeta) -> Result<(), CoreError> {
        self.data.lock().unwrap().insert(key.to_owned(), data);
        Ok(())
    }
    async fn retrieve(&self, key: &str) -> Result<Option<StoredArtifact>, CoreError> {
        let lock = self.data.lock().unwrap();
        Ok(lock.get(key).map(|bytes| {
            let b = bytes.clone();
            let s: ByteStream = Box::pin(stream::once(async move { Ok::<Bytes, CoreError>(b) }));
            StoredArtifact {
                stream: s,
                meta: StorageMeta::default(),
            }
        }))
    }
    async fn exists(&self, key: &str) -> Result<bool, CoreError> {
        Ok(self.data.lock().unwrap().contains_key(key))
    }
    async fn delete(&self, key: &str) -> Result<(), CoreError> {
        self.data.lock().unwrap().remove(key);
        Ok(())
    }
    async fn delete_by_prefix(&self, prefix: &str) -> Result<usize, CoreError> {
        let mut map = self.data.lock().unwrap();
        let keys: Vec<String> = map
            .keys()
            .filter(|k| k.starts_with(prefix))
            .cloned()
            .collect();
        let count = keys.len();
        for k in keys {
            map.remove(&k);
        }
        Ok(count)
    }
    async fn stat_by_prefix(&self, prefix: &str) -> Result<(u64, u64), CoreError> {
        let map = self.data.lock().unwrap();
        let (count, bytes) = map
            .iter()
            .filter(|(k, _)| k.starts_with(prefix))
            .fold((0u64, 0u64), |(c, b), (_, v)| (c + 1, b + v.len() as u64));
        Ok((count, bytes))
    }
    async fn list_keys(&self, prefix: &str) -> Result<Vec<String>, CoreError> {
        let map = self.data.lock().unwrap();
        Ok(map
            .keys()
            .filter(|k| k.starts_with(prefix))
            .cloned()
            .collect())
    }
}

struct FixedRegistry;

#[async_trait]
impl RegistryClient for FixedRegistry {
    fn registry_type(&self) -> &str {
        "test"
    }

    async fn resolve_metadata(&self, pkg: &PackageId) -> Result<PackageMetadata, CoreError> {
        Ok(PackageMetadata {
            id: pkg.clone(),
            published_at: Some(Utc::now() - chrono::Duration::days(30)),
            download_url: None,
            checksum: None,
            is_signed: None,
            extra: serde_json::json!({}),
            cache_control: None,
        })
    }

    async fn fetch_artifact(&self, pkg: &PackageId) -> Result<FetchedArtifact, CoreError> {
        let data = Bytes::from(format!("artifact:{}", pkg.cache_key()));
        Ok(FetchedArtifact {
            stream: Box::pin(stream::once(async move { Ok::<Bytes, CoreError>(data) })),
            cache_control: None,
        })
    }
}

struct DenyRegistry;

#[async_trait]
impl RegistryClient for DenyRegistry {
    fn registry_type(&self) -> &str {
        "test"
    }
    async fn resolve_metadata(&self, pkg: &PackageId) -> Result<PackageMetadata, CoreError> {
        Ok(PackageMetadata {
            id: pkg.clone(),
            published_at: Some(Utc::now() - chrono::Duration::days(30)),
            download_url: None,
            checksum: None,
            is_signed: None,
            extra: serde_json::json!({}),
            cache_control: None,
        })
    }
    async fn fetch_artifact(&self, _pkg: &PackageId) -> Result<FetchedArtifact, CoreError> {
        Err(CoreError::Registry("should not be called".into()))
    }
}

struct AlwaysDenyRule;

#[async_trait]
impl crate::rules::Rule for AlwaysDenyRule {
    fn name(&self) -> &str {
        "always_deny"
    }
    async fn evaluate(&self, _ctx: &crate::rules::RuleContext<'_>) -> crate::rules::RuleDecision {
        crate::rules::RuleDecision::Deny {
            reason: "test denial".to_owned(),
        }
    }
}

fn req(registry: &str) -> ProxyRequest {
    ProxyRequest {
        package_id: PackageId::new(registry, "test-pkg", "1.0.0"),
        identity: Identity::anonymous(),
        resource_type: "releases:read".to_owned(),
    }
}

fn proxy(
    registry_name: &str,
    client: Arc<dyn RegistryClient>,
    repo: Arc<dyn PackageRepository>,
    rules: Vec<Box<dyn crate::rules::Rule>>,
) -> ProxyService {
    let policy = RegistryPolicy {
        metadata_ttl: None,
        firewall_only: false,
        serve_stale_metadata: false,
        artifact_ttl: None,
        rules,
    };
    ProxyService {
        hot: make_hot(registry_name, client, policy, None),
        storage: MemStorage::new(),
        cache: TestCacheStore::new(),
        repo,
        artifact_meta: NoopArtifactMeta::arc(),
        metrics: Arc::new(ProxyMetrics::new(&[])),
        sbom: None,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn unknown_registry_returns_error() {
    let svc = proxy("npm", Arc::new(FixedRegistry), SpyRepo::new(), vec![]);
    let result = svc.handle(req("unknown")).await;
    assert!(matches!(result, Err(CoreError::UnknownRegistry(_))));
}

#[tokio::test]
async fn rejects_path_traversal_in_coordinate() {
    let svc = proxy("npm", Arc::new(FixedRegistry), SpyRepo::new(), vec![]);

    // `..` in the name escapes the storage root once interpolated into the cache
    // key — the edge chokepoint must reject it before any cache/storage access.
    let bad_name = ProxyRequest {
        package_id: PackageId::new("npm", "../../../../etc/passwd", "1.0.0"),
        identity: Identity::anonymous(),
        resource_type: "releases:read".to_owned(),
    };
    assert!(
        matches!(svc.handle(bad_name).await, Err(CoreError::InvalidInput(_))),
        "traversal in name must be rejected"
    );

    // ...and in the version segment...
    let bad_version = ProxyRequest {
        package_id: PackageId::new("npm", "test-pkg", "../../etc"),
        identity: Identity::anonymous(),
        resource_type: "releases:read".to_owned(),
    };
    assert!(
        matches!(
            svc.handle(bad_version).await,
            Err(CoreError::InvalidInput(_))
        ),
        "traversal in version must be rejected"
    );

    // ...and in the sub-artifact.
    let bad_artifact = ProxyRequest {
        package_id: PackageId::new("npm", "test-pkg", "1.0.0").with_artifact("../evil"),
        identity: Identity::anonymous(),
        resource_type: "source:read".to_owned(),
    };
    assert!(
        matches!(
            svc.handle(bad_artifact).await,
            Err(CoreError::InvalidInput(_))
        ),
        "traversal in artifact must be rejected"
    );
}

#[tokio::test]
async fn metadata_cache_miss_then_hit() {
    let repo = SpyRepo::new();
    let cache = TestCacheStore::new();
    let svc = ProxyService {
        hot: make_hot(
            "npm",
            Arc::new(FixedRegistry),
            RegistryPolicy {
                metadata_ttl: Some(Duration::from_secs(300)),
                firewall_only: false,
                serve_stale_metadata: false,
                artifact_ttl: None,
                rules: vec![],
            },
            None,
        ),
        storage: MemStorage::new(),
        cache: cache.clone(),
        repo: repo.clone(),
        artifact_meta: NoopArtifactMeta::arc(),
        metrics: Arc::new(ProxyMetrics::new(&[])),
        sbom: None,
    };

    let cache_key = format!("meta:{}", req("npm").package_id.cache_key());

    // First call: cache miss — metadata is fetched and stored
    assert!(cache.get(&cache_key).await.unwrap().is_none());
    let resp = svc.handle(req("npm")).await.unwrap();
    assert!(matches!(resp, ProxyResponse::Stream(_)));
    assert!(
        cache.get(&cache_key).await.unwrap().is_some(),
        "metadata should be cached after first call"
    );

    // Second call: cache hit — lines 86-87 are exercised
    let resp2 = svc.handle(req("npm")).await.unwrap();
    assert!(
        matches!(resp2, ProxyResponse::Stream(_)),
        "second call must still return Stream"
    );
}

#[tokio::test]
async fn rule_denial_returns_denied_and_records_event() {
    let repo = SpyRepo::new();
    let svc = proxy(
        "npm",
        Arc::new(DenyRegistry),
        repo.clone(),
        vec![Box::new(AlwaysDenyRule)],
    );

    let resp = svc.handle(req("npm")).await.unwrap();
    assert!(
        matches!(resp, ProxyResponse::Denied { reason } if reason == "test denial"),
        "expected Denied response"
    );
    let events = repo.events();
    assert_eq!(events.len(), 1, "one denied event should be recorded");
    assert!(matches!(events[0].result, AccessResult::Denied { .. }));
}

#[tokio::test]
async fn artifact_cache_hit_returns_stored_bytes() {
    let storage = MemStorage::new();
    let pkg = PackageId::new("npm", "test-pkg", "1.0.0");
    let artifact_key = format!("artifact:{}", pkg.cache_key());
    // Pre-populate storage
    storage
        .store(
            &artifact_key,
            Bytes::from("cached!"),
            StorageMeta::default(),
        )
        .await
        .unwrap();

    let repo = SpyRepo::new();
    let svc = ProxyService {
        hot: make_hot(
            "npm",
            Arc::new(FixedRegistry),
            RegistryPolicy {
                metadata_ttl: None,
                firewall_only: false,
                serve_stale_metadata: false,
                artifact_ttl: None,
                rules: vec![],
            },
            None,
        ),
        storage: storage.clone(),
        cache: TestCacheStore::new(),
        repo: repo.clone(),
        artifact_meta: NoopArtifactMeta::arc(),
        metrics: Arc::new(ProxyMetrics::new(&[])),
        sbom: None,
    };

    let resp = svc.handle(req("npm")).await.unwrap();
    assert!(matches!(resp, ProxyResponse::Stream(_)));
    // Access event should be recorded for the cache hit
    assert!(!repo.events().is_empty(), "access event should be recorded");
}

#[tokio::test]
async fn artifact_cache_miss_fetches_from_upstream() {
    let repo = SpyRepo::new();
    let svc = proxy("npm", Arc::new(FixedRegistry), repo.clone(), vec![]);

    let pkg = PackageId::new("npm", "test-pkg", "1.0.0");
    let artifact_key = format!("artifact:{}", pkg.cache_key());

    // Storage is empty — must fetch from upstream
    assert!(!svc.storage.exists(&artifact_key).await.unwrap());

    let resp = svc.handle(req("npm")).await.unwrap();
    assert!(matches!(resp, ProxyResponse::Stream(_)));

    // Artifact should now be stored
    assert!(
        svc.storage.exists(&artifact_key).await.unwrap(),
        "artifact should be stored after fetch"
    );
    assert!(!repo.events().is_empty(), "access event should be recorded");
}

#[tokio::test]
async fn payload_too_large_returns_error() {
    let repo = SpyRepo::new();
    let svc = ProxyService {
        hot: make_hot(
            "npm",
            Arc::new(FixedRegistry),
            RegistryPolicy {
                metadata_ttl: None,
                firewall_only: false,
                serve_stale_metadata: false,
                artifact_ttl: None,
                rules: vec![],
            },
            Some(5), // FixedRegistry sends >5 bytes
        ),
        storage: MemStorage::new(),
        cache: TestCacheStore::new(),
        repo: repo.clone(),
        artifact_meta: NoopArtifactMeta::arc(),
        metrics: Arc::new(ProxyMetrics::new(&[])),
        sbom: None,
    };

    let result = svc.handle(req("npm")).await;
    assert!(matches!(result, Err(CoreError::PayloadTooLarge(_))));
}

#[tokio::test]
async fn unused_registry_id_in_policies_does_not_panic() {
    let repo = SpyRepo::new();
    // no policy for "npm" — should use empty rule set
    let svc = ProxyService {
        hot: empty_hot("npm", Arc::new(FixedRegistry)),
        storage: MemStorage::new(),
        cache: TestCacheStore::new(),
        repo: repo.clone(),
        artifact_meta: NoopArtifactMeta::arc(),
        metrics: Arc::new(ProxyMetrics::new(&[])),
        sbom: None,
    };

    let resp = svc.handle(req("npm")).await.unwrap();
    assert!(matches!(resp, ProxyResponse::Stream(_)));
}

#[tokio::test]
async fn firewall_only_streams_without_storing() {
    let storage = MemStorage::new();
    let repo = SpyRepo::new();
    let svc = ProxyService {
        hot: make_hot(
            "npm",
            Arc::new(FixedRegistry),
            RegistryPolicy {
                metadata_ttl: None,
                firewall_only: true,
                serve_stale_metadata: false,
                artifact_ttl: None,
                rules: vec![],
            },
            None,
        ),
        storage: storage.clone(),
        cache: TestCacheStore::new(),
        repo: repo.clone(),
        artifact_meta: NoopArtifactMeta::arc(),
        metrics: Arc::new(ProxyMetrics::new(&[])),
        sbom: None,
    };

    let pkg = PackageId::new("npm", "test-pkg", "1.0.0");
    let artifact_key = format!("artifact:{}", pkg.cache_key());

    let resp = svc.handle(req("npm")).await.unwrap();
    assert!(matches!(resp, ProxyResponse::Stream(_)));
    assert!(
        !storage.exists(&artifact_key).await.unwrap(),
        "firewall-only: artifact must not be stored"
    );
    assert!(!repo.events().is_empty(), "access event should be recorded");
}

struct UnavailableRegistry;

#[async_trait]
impl RegistryClient for UnavailableRegistry {
    fn registry_type(&self) -> &str {
        "test"
    }
    async fn resolve_metadata(&self, _pkg: &PackageId) -> Result<PackageMetadata, CoreError> {
        Err(CoreError::Registry("upstream down".into()))
    }
    async fn fetch_artifact(&self, pkg: &PackageId) -> Result<FetchedArtifact, CoreError> {
        let data = Bytes::from(format!("artifact:{}", pkg.cache_key()));
        Ok(FetchedArtifact {
            stream: Box::pin(stream::once(async move { Ok::<Bytes, CoreError>(data) })),
            cache_control: None,
        })
    }
}

fn proxy_with_stale(
    client: Arc<dyn RegistryClient>,
    repo: Arc<dyn PackageRepository>,
    cache: Arc<dyn CacheStore>,
    serve_stale: bool,
) -> ProxyService {
    ProxyService {
        hot: make_hot(
            "npm",
            client,
            RegistryPolicy {
                metadata_ttl: Some(Duration::from_secs(300)),
                firewall_only: false,
                serve_stale_metadata: serve_stale,
                artifact_ttl: None,
                rules: vec![],
            },
            None,
        ),
        storage: MemStorage::new(),
        cache,
        repo,
        artifact_meta: NoopArtifactMeta::arc(),
        metrics: Arc::new(ProxyMetrics::new(&[])),
        sbom: None,
    }
}

#[tokio::test]
async fn stale_metadata_served_when_upstream_unavailable() {
    let repo = SpyRepo::new();
    let cache = TestCacheStore::new();
    let pkg = PackageId::new("npm", "test-pkg", "1.0.0");
    let cache_key = format!("meta:{}", pkg.cache_key());
    let stale_meta = PackageMetadata {
        id: pkg.clone(),
        published_at: Some(Utc::now() - chrono::Duration::days(10)),
        download_url: None,
        checksum: None,
        is_signed: None,
        extra: serde_json::json!({}),
        cache_control: None,
    };
    cache.seed_expired(&cache_key, stale_meta);

    let svc = proxy_with_stale(Arc::new(UnavailableRegistry), repo.clone(), cache, true);
    let resp = svc.handle(req("npm")).await.unwrap();
    assert!(
        matches!(resp, ProxyResponse::Stream(_)),
        "stale fallback should succeed"
    );
    assert!(
        repo.events()
            .iter()
            .all(|e| !matches!(e.result, AccessResult::ProxyError { .. })),
        "no proxy_error should be recorded when stale metadata is served"
    );
}

#[tokio::test]
async fn stale_not_used_when_serve_stale_false() {
    let repo = SpyRepo::new();
    let cache = TestCacheStore::new();
    let pkg = PackageId::new("npm", "test-pkg", "1.0.0");
    let cache_key = format!("meta:{}", pkg.cache_key());
    let stale_meta = PackageMetadata {
        id: pkg.clone(),
        published_at: Some(Utc::now() - chrono::Duration::days(10)),
        download_url: None,
        checksum: None,
        is_signed: None,
        extra: serde_json::json!({}),
        cache_control: None,
    };
    cache.seed_expired(&cache_key, stale_meta);

    let svc = proxy_with_stale(Arc::new(UnavailableRegistry), repo.clone(), cache, false);
    let result = svc.handle(req("npm")).await;
    assert!(
        matches!(result, Err(CoreError::Registry(_))),
        "should propagate the upstream error"
    );
    assert!(
        repo.events()
            .iter()
            .any(|e| matches!(e.result, AccessResult::ProxyError { .. })),
        "proxy_error must be recorded"
    );
}

#[tokio::test]
async fn cold_start_with_upstream_down_returns_error() {
    let repo = SpyRepo::new();
    let cache = TestCacheStore::new(); // empty — no stale entry

    let svc = proxy_with_stale(Arc::new(UnavailableRegistry), repo.clone(), cache, true);
    let result = svc.handle(req("npm")).await;
    assert!(
        matches!(result, Err(CoreError::Registry(_))),
        "no stale entry + upstream down must return error"
    );
}

#[tokio::test]
async fn not_found_from_upstream_is_not_stale_eligible() {
    struct NotFoundRegistry;
    #[async_trait]
    impl RegistryClient for NotFoundRegistry {
        fn registry_type(&self) -> &str {
            "test"
        }
        async fn resolve_metadata(&self, pkg: &PackageId) -> Result<PackageMetadata, CoreError> {
            Err(CoreError::NotFound(pkg.name.clone()))
        }
        async fn fetch_artifact(&self, _pkg: &PackageId) -> Result<FetchedArtifact, CoreError> {
            Err(CoreError::NotFound("no artifact".into()))
        }
    }

    let repo = SpyRepo::new();
    let cache = TestCacheStore::new();
    let pkg = PackageId::new("npm", "test-pkg", "1.0.0");
    let cache_key = format!("meta:{}", pkg.cache_key());
    let stale_meta = PackageMetadata {
        id: pkg.clone(),
        published_at: Some(Utc::now() - chrono::Duration::days(10)),
        download_url: None,
        checksum: None,
        is_signed: None,
        extra: serde_json::json!({}),
        cache_control: None,
    };
    cache.seed_expired(&cache_key, stale_meta);

    let svc = proxy_with_stale(Arc::new(NotFoundRegistry), repo.clone(), cache, true);
    let result = svc.handle(req("npm")).await;
    assert!(
        matches!(result, Err(CoreError::NotFound(_))),
        "NotFound must not fall back to stale"
    );
}

// ── Cache-Control and ArtifactMeta integration tests ─────────────────────

struct NoStoreMetaRegistry;

#[async_trait]
impl RegistryClient for NoStoreMetaRegistry {
    fn registry_type(&self) -> &str {
        "test"
    }
    async fn resolve_metadata(&self, pkg: &PackageId) -> Result<PackageMetadata, CoreError> {
        Ok(PackageMetadata {
            id: pkg.clone(),
            published_at: Some(Utc::now() - chrono::Duration::days(1)),
            download_url: None,
            checksum: None,
            is_signed: None,
            extra: serde_json::json!({}),
            cache_control: Some("no-store".to_owned()),
        })
    }
    async fn fetch_artifact(&self, pkg: &PackageId) -> Result<FetchedArtifact, CoreError> {
        let data = Bytes::from(format!("artifact:{}", pkg.cache_key()));
        Ok(FetchedArtifact {
            stream: Box::pin(stream::once(async move { Ok::<Bytes, CoreError>(data) })),
            cache_control: None,
        })
    }
}

struct NoStoreArtifactRegistry;

#[async_trait]
impl RegistryClient for NoStoreArtifactRegistry {
    fn registry_type(&self) -> &str {
        "test"
    }
    async fn resolve_metadata(&self, pkg: &PackageId) -> Result<PackageMetadata, CoreError> {
        Ok(PackageMetadata {
            id: pkg.clone(),
            published_at: Some(Utc::now() - chrono::Duration::days(1)),
            download_url: None,
            checksum: None,
            is_signed: None,
            extra: serde_json::json!({}),
            cache_control: None,
        })
    }
    async fn fetch_artifact(&self, pkg: &PackageId) -> Result<FetchedArtifact, CoreError> {
        let data = Bytes::from(format!("artifact:{}", pkg.cache_key()));
        Ok(FetchedArtifact {
            stream: Box::pin(stream::once(async move { Ok::<Bytes, CoreError>(data) })),
            cache_control: Some("no-store".to_owned()),
        })
    }
}

#[tokio::test]
async fn metadata_no_store_skips_cache() {
    let repo = SpyRepo::new();
    let cache = TestCacheStore::new();
    let svc = ProxyService {
        hot: make_hot(
            "npm",
            Arc::new(NoStoreMetaRegistry),
            RegistryPolicy {
                metadata_ttl: Some(Duration::from_secs(300)),
                firewall_only: false,
                serve_stale_metadata: false,
                artifact_ttl: None,
                rules: vec![],
            },
            None,
        ),
        storage: MemStorage::new(),
        cache: cache.clone(),
        repo,
        artifact_meta: NoopArtifactMeta::arc(),
        metrics: Arc::new(ProxyMetrics::new(&[])),
        sbom: None,
    };

    let cache_key = format!("meta:{}", req("npm").package_id.cache_key());
    let resp = svc.handle(req("npm")).await.unwrap();
    assert!(
        matches!(resp, ProxyResponse::Stream(_)),
        "response must still be a stream"
    );
    assert!(
        cache.get(&cache_key).await.unwrap().is_none(),
        "metadata must NOT be cached when upstream returns Cache-Control: no-store"
    );
}

#[tokio::test]
async fn artifact_no_store_skips_storage() {
    let repo = SpyRepo::new();
    let storage = MemStorage::new();
    let svc = ProxyService {
        hot: empty_hot("npm", Arc::new(NoStoreArtifactRegistry)),
        storage: storage.clone(),
        cache: TestCacheStore::new(),
        repo,
        artifact_meta: NoopArtifactMeta::arc(),
        metrics: Arc::new(ProxyMetrics::new(&[])),
        sbom: None,
    };

    let pkg = PackageId::new("npm", "test-pkg", "1.0.0");
    let artifact_key = format!("artifact:{}", pkg.cache_key());

    let resp = svc.handle(req("npm")).await.unwrap();
    assert!(
        matches!(resp, ProxyResponse::Stream(_)),
        "response must still be a stream"
    );
    assert!(
        !storage.exists(&artifact_key).await.unwrap(),
        "artifact must NOT be stored when upstream returns Cache-Control: no-store"
    );
}

#[tokio::test]
async fn artifact_ttl_expired_refetches_from_upstream() {
    let storage = MemStorage::new();
    let pkg = PackageId::new("npm", "test-pkg", "1.0.0");
    let artifact_key = format!("artifact:{}", pkg.cache_key());
    storage
        .store(
            &artifact_key,
            Bytes::from("stale-bytes"),
            StorageMeta::default(),
        )
        .await
        .unwrap();

    // Spy meta says artifact is expired
    let expired_meta = ArtifactMeta {
        artifact_key: artifact_key.clone(),
        registry: "npm".to_owned(),
        package_name: "test-pkg".to_owned(),
        version: "1.0.0".to_owned(),
        size_bytes: Some(11),
        cached_at: Utc::now() - chrono::Duration::hours(2),
        last_accessed_at: Utc::now() - chrono::Duration::hours(2),
    };
    let spy_meta = SpyArtifactMeta::with_expired(vec![expired_meta]);

    let repo = SpyRepo::new();
    let svc = ProxyService {
        hot: make_hot(
            "npm",
            Arc::new(FixedRegistry),
            RegistryPolicy {
                metadata_ttl: None,
                firewall_only: false,
                serve_stale_metadata: false,
                artifact_ttl: Some(Duration::from_secs(3600)), // 1h TTL
                rules: vec![],
            },
            None,
        ),
        storage: storage.clone(),
        cache: TestCacheStore::new(),
        repo,
        artifact_meta: spy_meta.clone(),
        metrics: Arc::new(ProxyMetrics::new(&[])),
        sbom: None,
    };

    let resp = svc.handle(req("npm")).await.unwrap();
    assert!(matches!(resp, ProxyResponse::Stream(_)));
    // After re-fetch, record_artifact should have been called
    assert!(
        !spy_meta.recorded_keys().is_empty(),
        "record_artifact must be called after re-fetch"
    );
    // Storage should now contain the freshly fetched artifact
    assert!(storage.exists(&artifact_key).await.unwrap());
}

#[tokio::test]
async fn artifact_cache_hit_records_touch() {
    let storage = MemStorage::new();
    let pkg = PackageId::new("npm", "test-pkg", "1.0.0");
    let artifact_key = format!("artifact:{}", pkg.cache_key());
    storage
        .store(
            &artifact_key,
            Bytes::from("cached!"),
            StorageMeta::default(),
        )
        .await
        .unwrap();

    // Pre-seed the artifact metadata to simulate a previous record_artifact call.
    // Without this, is_artifact_expired treats the missing row as expired (correct
    // production behavior) and the proxy re-fetches instead of serving from cache.
    let spy_meta = SpyArtifactMeta::new();
    spy_meta
        .record_artifact(&artifact_key, "npm", "test-pkg", "1.0.0", None)
        .await
        .unwrap();
    let svc = ProxyService {
        hot: make_hot(
            "npm",
            Arc::new(FixedRegistry),
            RegistryPolicy {
                metadata_ttl: None,
                firewall_only: false,
                serve_stale_metadata: false,
                artifact_ttl: Some(Duration::from_secs(3600)),
                rules: vec![],
            },
            None,
        ),
        storage: storage.clone(),
        cache: TestCacheStore::new(),
        repo: SpyRepo::new(),
        artifact_meta: spy_meta.clone(),
        metrics: Arc::new(ProxyMetrics::new(&[])),
        sbom: None,
    };

    let resp = svc.handle(req("npm")).await.unwrap();
    assert!(matches!(resp, ProxyResponse::Stream(_)));
    // touch_artifact is called from tokio::spawn — yield to let it complete
    tokio::task::yield_now().await;
    assert!(
        spy_meta.touched_keys().contains(&artifact_key),
        "touch_artifact must be called on cache hit"
    );
}

#[tokio::test]
async fn artifact_cache_miss_records_meta() {
    let spy_meta = SpyArtifactMeta::new();
    let repo = SpyRepo::new();
    let svc = ProxyService {
        hot: empty_hot("npm", Arc::new(FixedRegistry)),
        storage: MemStorage::new(),
        cache: TestCacheStore::new(),
        repo,
        artifact_meta: spy_meta.clone(),
        metrics: Arc::new(ProxyMetrics::new(&[])),
        sbom: None,
    };

    let pkg = PackageId::new("npm", "test-pkg", "1.0.0");
    let artifact_key = format!("artifact:{}", pkg.cache_key());

    let resp = svc.handle(req("npm")).await.unwrap();
    assert!(matches!(resp, ProxyResponse::Stream(_)));
    assert!(
        spy_meta.recorded_keys().contains(&artifact_key),
        "record_artifact must be called after a cache miss and successful upstream fetch"
    );
}

#[tokio::test]
async fn metrics_artifact_miss_then_hit() {
    let proxy_metrics = Arc::new(ProxyMetrics::new(&["npm".to_owned()]));
    let storage = MemStorage::new();
    let svc = ProxyService {
        hot: make_hot(
            "npm",
            Arc::new(FixedRegistry),
            RegistryPolicy {
                metadata_ttl: Some(Duration::from_secs(300)),
                firewall_only: false,
                serve_stale_metadata: false,
                artifact_ttl: None,
                rules: vec![],
            },
            None,
        ),
        storage: storage.clone(),
        cache: TestCacheStore::new(),
        repo: SpyRepo::new(),
        artifact_meta: NoopArtifactMeta::arc(),
        metrics: proxy_metrics.clone(),
        sbom: None,
    };

    let npm = proxy_metrics.all().get("npm").unwrap();

    // First call: artifact not in storage → miss counter incremented
    svc.handle(req("npm")).await.unwrap();
    assert_eq!(npm.misses(), 1, "first call must register a miss");
    assert_eq!(npm.hits(), 0);

    // Second call: artifact now in storage → hit counter incremented
    svc.handle(req("npm")).await.unwrap();
    assert_eq!(npm.misses(), 1, "miss count must not change on second call");
    assert_eq!(npm.hits(), 1, "second call must register a hit");
}

// ── Integrity verification ─────────────────────────────────────────────────

/// Registry whose metadata advertises a configurable checksum and whose
/// artifact body is fixed, so a test can force verified / mismatch / missing.
struct ChecksumRegistry {
    checksum: Option<String>,
    body: &'static [u8],
}

#[async_trait]
impl RegistryClient for ChecksumRegistry {
    fn registry_type(&self) -> &str {
        "test"
    }
    async fn resolve_metadata(&self, pkg: &PackageId) -> Result<PackageMetadata, CoreError> {
        Ok(PackageMetadata {
            id: pkg.clone(),
            published_at: Some(Utc::now() - chrono::Duration::days(30)),
            download_url: None,
            checksum: self.checksum.clone(),
            is_signed: None,
            extra: serde_json::json!({}),
            cache_control: None,
        })
    }
    async fn fetch_artifact(&self, _pkg: &PackageId) -> Result<FetchedArtifact, CoreError> {
        let data = Bytes::from_static(self.body);
        Ok(FetchedArtifact {
            stream: Box::pin(stream::once(async move { Ok::<Bytes, CoreError>(data) })),
            cache_control: None,
        })
    }
}

/// Build a proxy whose single registry has the given integrity policy (when
/// `None`, the registry has no explicit block so the default policy applies).
/// Returns the service together with its storage so caching can be asserted.
fn proxy_with_integrity(
    registry_name: &str,
    client: Arc<dyn RegistryClient>,
    integrity: Option<crate::services::IntegrityPolicy>,
) -> (ProxyService, Arc<MemStorage>) {
    let mut registries: HashMap<String, Arc<dyn RegistryClient>> = HashMap::new();
    registries.insert(registry_name.to_owned(), client);
    let mut policies = HashMap::new();
    policies.insert(
        registry_name.to_owned(),
        Arc::new(RegistryPolicy {
            metadata_ttl: None,
            firewall_only: false,
            serve_stale_metadata: false,
            artifact_ttl: None,
            rules: vec![],
        }),
    );
    let mut integrity_map = HashMap::new();
    if let Some(i) = integrity {
        integrity_map.insert(registry_name.to_owned(), i);
    }
    let hot = new_hot_lock(HotConfig {
        registries,
        policies,
        integrity: integrity_map,
        ..Default::default()
    });
    let storage = MemStorage::new();
    let svc = ProxyService {
        hot,
        storage: storage.clone(),
        cache: TestCacheStore::new(),
        repo: SpyRepo::new(),
        artifact_meta: NoopArtifactMeta::arc(),
        metrics: Arc::new(ProxyMetrics::new(&[])),
        sbom: None,
    };
    (svc, storage)
}

fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(data))
}

const BODY: &[u8] = b"the-real-artifact-bytes";

#[tokio::test]
async fn integrity_verified_artifact_is_cached_and_served() {
    let reg = Arc::new(ChecksumRegistry {
        checksum: Some(sha256_hex(BODY)),
        body: BODY,
    });
    // No explicit policy → default (enabled, block-on-mismatch).
    let (svc, storage) = proxy_with_integrity("npm", reg, None);

    let resp = svc.handle(req("npm")).await.unwrap();
    assert!(matches!(resp, ProxyResponse::Stream(_)));

    let artifact_key = format!("artifact:{}", req("npm").package_id.cache_key());
    assert!(
        storage.exists(&artifact_key).await.unwrap(),
        "verified artifact must be cached"
    );
}

#[tokio::test]
async fn integrity_mismatch_blocks_and_is_not_cached() {
    let reg = Arc::new(ChecksumRegistry {
        // Advertise the checksum of *different* bytes → mismatch.
        checksum: Some(sha256_hex(b"some-other-bytes")),
        body: BODY,
    });
    let (svc, storage) = proxy_with_integrity("npm", reg, None);

    let result = svc.handle(req("npm")).await;
    assert!(
        matches!(result, Err(CoreError::IntegrityFailure(_))),
        "mismatch must fail the download"
    );

    let artifact_key = format!("artifact:{}", req("npm").package_id.cache_key());
    assert!(
        !storage.exists(&artifact_key).await.unwrap(),
        "bytes that fail verification must never be cached"
    );
}

#[tokio::test]
async fn integrity_missing_metadata_warns_and_serves_by_default() {
    let reg = Arc::new(ChecksumRegistry {
        checksum: None,
        body: BODY,
    });
    // Default policy: require_metadata = false → missing only warns.
    let (svc, storage) = proxy_with_integrity("npm", reg, None);

    let resp = svc.handle(req("npm")).await.unwrap();
    assert!(matches!(resp, ProxyResponse::Stream(_)));
    let artifact_key = format!("artifact:{}", req("npm").package_id.cache_key());
    assert!(storage.exists(&artifact_key).await.unwrap());
}

#[tokio::test]
async fn integrity_require_metadata_blocks_when_absent() {
    let reg = Arc::new(ChecksumRegistry {
        checksum: None,
        body: BODY,
    });
    let policy = crate::services::IntegrityPolicy {
        enabled: true,
        block_on_mismatch: true,
        require_metadata: true,
        bypass_roles: vec![],
    };
    let (svc, storage) = proxy_with_integrity("npm", reg, Some(policy));

    let result = svc.handle(req("npm")).await;
    assert!(
        matches!(result, Err(CoreError::IntegrityFailure(_))),
        "require_metadata must block a checksum-less download"
    );
    let artifact_key = format!("artifact:{}", req("npm").package_id.cache_key());
    assert!(!storage.exists(&artifact_key).await.unwrap());
}

#[tokio::test]
async fn integrity_require_metadata_bypass_role_is_allowed() {
    let reg = Arc::new(ChecksumRegistry {
        checksum: None,
        body: BODY,
    });
    let policy = crate::services::IntegrityPolicy {
        enabled: true,
        block_on_mismatch: true,
        require_metadata: true,
        bypass_roles: vec![crate::entities::Role::Admin],
    };
    let (svc, _storage) = proxy_with_integrity("npm", reg, Some(policy));

    let admin_req = ProxyRequest {
        package_id: PackageId::new("npm", "test-pkg", "1.0.0"),
        identity: Identity {
            user_id: None,
            role: crate::entities::Role::Admin,
            auth_provider: None,
            groups: vec![],
        },
        resource_type: "releases:read".to_owned(),
    };
    let resp = svc.handle(admin_req).await.unwrap();
    assert!(
        matches!(resp, ProxyResponse::Stream(_)),
        "a bypass-role caller must be served despite missing metadata"
    );
}

#[tokio::test]
async fn integrity_mismatch_warn_only_serves_and_caches() {
    let reg = Arc::new(ChecksumRegistry {
        checksum: Some(sha256_hex(b"some-other-bytes")),
        body: BODY,
    });
    // block_on_mismatch = false → warn but do not block; bytes are still cached.
    let policy = crate::services::IntegrityPolicy {
        enabled: true,
        block_on_mismatch: false,
        require_metadata: false,
        bypass_roles: vec![],
    };
    let (svc, storage) = proxy_with_integrity("npm", reg, Some(policy));

    let resp = svc.handle(req("npm")).await.unwrap();
    assert!(
        matches!(resp, ProxyResponse::Stream(_)),
        "warn-only mismatch must still serve the artifact"
    );
    let artifact_key = format!("artifact:{}", req("npm").package_id.cache_key());
    assert!(storage.exists(&artifact_key).await.unwrap());
}

#[tokio::test]
async fn integrity_unparseable_checksum_warns_and_serves() {
    let reg = Arc::new(ChecksumRegistry {
        // Not SRI and not a known-length hex string → Unparseable.
        checksum: Some("not-a-real-checksum".to_owned()),
        body: BODY,
    });
    // Default policy: an unverifiable checksum is treated like missing (serve).
    let (svc, storage) = proxy_with_integrity("npm", reg, None);

    let resp = svc.handle(req("npm")).await.unwrap();
    assert!(matches!(resp, ProxyResponse::Stream(_)));
    let artifact_key = format!("artifact:{}", req("npm").package_id.cache_key());
    assert!(storage.exists(&artifact_key).await.unwrap());
}

#[tokio::test]
async fn integrity_disabled_skips_verification_even_on_mismatch() {
    let reg = Arc::new(ChecksumRegistry {
        checksum: Some(sha256_hex(b"some-other-bytes")),
        body: BODY,
    });
    let policy = crate::services::IntegrityPolicy {
        enabled: false,
        block_on_mismatch: true,
        require_metadata: false,
        bypass_roles: vec![],
    };
    let (svc, storage) = proxy_with_integrity("npm", reg, Some(policy));

    let resp = svc.handle(req("npm")).await.unwrap();
    assert!(
        matches!(resp, ProxyResponse::Stream(_)),
        "disabled integrity must not block a mismatching artifact"
    );
    let artifact_key = format!("artifact:{}", req("npm").package_id.cache_key());
    assert!(storage.exists(&artifact_key).await.unwrap());
}
