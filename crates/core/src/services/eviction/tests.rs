use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use bytes::Bytes;
use chrono::{DateTime, Duration, Utc};

use super::*;
use crate::error::CoreError;
use crate::ports::ByteStream;
use crate::ports::{
    ArtifactCacheMeta, ArtifactInventory, ArtifactMeta, ArtifactMetaRecord, StorageBackend,
    StorageMeta, StoredArtifact,
};

// ── In-memory ArtifactMetaRepository ─────────────────────────────────────

#[derive(Default)]
struct InMemArtifactMeta {
    rows: Mutex<Vec<ArtifactMeta>>,
}

impl InMemArtifactMeta {
    fn arc() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn seed(&self, meta: ArtifactMeta) {
        self.rows.lock().unwrap().push(meta);
    }

    fn all(&self) -> Vec<ArtifactMeta> {
        self.rows.lock().unwrap().clone()
    }
}

#[async_trait]
impl ArtifactCacheMeta for InMemArtifactMeta {
    async fn record_artifact(&self, rec: ArtifactMetaRecord<'_>) -> Result<(), CoreError> {
        let now = Utc::now();
        let mut rows = self.rows.lock().unwrap();
        if let Some(r) = rows.iter_mut().find(|r| r.artifact_key == rec.key) {
            r.size_bytes = rec.size;
            r.cached_at = now;
            r.last_accessed_at = now;
        } else {
            rows.push(ArtifactMeta {
                artifact_key: rec.key.to_owned(),
                registry: rec.registry.to_owned(),
                package_name: rec.package_name.to_owned(),
                version: rec.version.to_owned(),
                size_bytes: rec.size,
                cached_at: now,
                last_accessed_at: now,
            });
        }
        Ok(())
    }

    async fn get_artifact_checksum(&self, _key: &str) -> Result<Option<String>, CoreError> {
        Ok(None)
    }

    async fn touch_artifact(&self, key: &str) -> Result<(), CoreError> {
        let now = Utc::now();
        let mut rows = self.rows.lock().unwrap();
        if let Some(r) = rows.iter_mut().find(|r| r.artifact_key == key) {
            r.last_accessed_at = now;
        }
        Ok(())
    }

    async fn is_artifact_expired(
        &self,
        key: &str,
        older_than: DateTime<Utc>,
    ) -> Result<bool, CoreError> {
        let rows = self.rows.lock().unwrap();
        // No row → treat as expired (matches PgArtifactMetaRepository semantics).
        let fresh = rows
            .iter()
            .any(|r| r.artifact_key == key && r.cached_at >= older_than);
        Ok(!fresh)
    }

    async fn delete_artifact_meta(&self, key: &str) -> Result<(), CoreError> {
        self.rows.lock().unwrap().retain(|r| r.artifact_key != key);
        Ok(())
    }
}

#[async_trait]
impl ArtifactInventory for InMemArtifactMeta {
    async fn list_artifacts(&self, registry: &str) -> Result<Vec<ArtifactMeta>, CoreError> {
        let rows = self.rows.lock().unwrap();
        Ok(rows
            .iter()
            .filter(|r| registry.is_empty() || r.registry == registry)
            .cloned()
            .collect())
    }

    async fn list_artifacts_by_package(&self) -> Result<Vec<ArtifactMeta>, CoreError> {
        let mut rows = self.rows.lock().unwrap().clone();
        rows.sort_by(|a, b| {
            a.registry
                .cmp(&b.registry)
                .then(a.package_name.cmp(&b.package_name))
                .then(b.cached_at.cmp(&a.cached_at)) // DESC
        });
        Ok(rows)
    }

    async fn list_expired_by_ttl(
        &self,
        registry: &str,
        older_than: DateTime<Utc>,
    ) -> Result<Vec<ArtifactMeta>, CoreError> {
        let rows = self.rows.lock().unwrap();
        Ok(rows
            .iter()
            .filter(|r| (registry.is_empty() || r.registry == registry) && r.cached_at < older_than)
            .cloned()
            .collect())
    }

    async fn list_idle(
        &self,
        registry: &str,
        idle_since: DateTime<Utc>,
    ) -> Result<Vec<ArtifactMeta>, CoreError> {
        let rows = self.rows.lock().unwrap();
        Ok(rows
            .iter()
            .filter(|r| {
                (registry.is_empty() || r.registry == registry) && r.last_accessed_at < idle_since
            })
            .cloned()
            .collect())
    }

    async fn total_size_bytes(&self, registry: &str) -> Result<u64, CoreError> {
        let rows = self.rows.lock().unwrap();
        Ok(rows
            .iter()
            .filter(|r| registry.is_empty() || r.registry == registry)
            .map(|r| r.size_bytes.unwrap_or(0))
            .sum())
    }

    async fn list_lru(&self, registry: &str, limit: i64) -> Result<Vec<ArtifactMeta>, CoreError> {
        let mut rows = self.rows.lock().unwrap().clone();
        rows.retain(|r| registry.is_empty() || r.registry == registry);
        rows.sort_by_key(|r| r.last_accessed_at);
        rows.truncate(limit as usize);
        Ok(rows)
    }
}

// ── In-memory StorageBackend ──────────────────────────────────────────────

#[derive(Default)]
struct InMemStorage {
    data: Mutex<HashMap<String, Bytes>>,
}

impl InMemStorage {
    fn arc() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn seed(&self, key: &str, data: &[u8]) {
        self.data
            .lock()
            .unwrap()
            .insert(key.to_owned(), Bytes::copy_from_slice(data));
    }

    fn keys(&self) -> Vec<String> {
        self.data.lock().unwrap().keys().cloned().collect()
    }

    fn contains(&self, key: &str) -> bool {
        self.data.lock().unwrap().contains_key(key)
    }
}

#[async_trait]
impl StorageBackend for InMemStorage {
    async fn store(&self, key: &str, data: Bytes, _: StorageMeta) -> Result<(), CoreError> {
        self.data.lock().unwrap().insert(key.to_owned(), data);
        Ok(())
    }
    async fn retrieve(&self, key: &str) -> Result<Option<StoredArtifact>, CoreError> {
        Ok(self.data.lock().unwrap().get(key).map(|b| {
            let b = b.clone();
            let stream: ByteStream =
                Box::pin(futures::stream::once(
                    async move { Ok::<Bytes, CoreError>(b) },
                ));
            StoredArtifact {
                stream,
                meta: StorageMeta::default(),
            }
        }))
    }
    async fn exists(&self, key: &str) -> Result<bool, CoreError> {
        Ok(self.data.lock().unwrap().contains_key(key))
    }
    async fn delete(&self, key: &str) -> Result<bool, CoreError> {
        Ok(self.data.lock().unwrap().remove(key).is_some())
    }
    async fn delete_by_prefix(&self, prefix: &str) -> Result<usize, CoreError> {
        let mut m = self.data.lock().unwrap();
        let keys: Vec<_> = m
            .keys()
            .filter(|k| k.starts_with(prefix))
            .cloned()
            .collect();
        let n = keys.len();
        for k in keys {
            m.remove(&k);
        }
        Ok(n)
    }
    async fn stat_by_prefix(&self, prefix: &str) -> Result<(u64, u64), CoreError> {
        let m = self.data.lock().unwrap();
        let (c, b) = m
            .iter()
            .filter(|(k, _)| k.starts_with(prefix))
            .fold((0u64, 0u64), |(c, b), (_, v)| (c + 1, b + v.len() as u64));
        Ok((c, b))
    }
    async fn list_keys(&self, prefix: &str) -> Result<Vec<String>, CoreError> {
        Ok(self
            .data
            .lock()
            .unwrap()
            .keys()
            .filter(|k| k.starts_with(prefix))
            .cloned()
            .collect())
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────

fn make_meta(
    key: &str,
    registry: &str,
    pkg: &str,
    version: &str,
    size: u64,
    cached_ago: Duration,
    accessed_ago: Duration,
) -> ArtifactMeta {
    let now = Utc::now();
    ArtifactMeta {
        artifact_key: key.to_owned(),
        registry: registry.to_owned(),
        package_name: pkg.to_owned(),
        version: version.to_owned(),
        size_bytes: Some(size),
        cached_at: now - cached_ago,
        last_accessed_at: now - accessed_ago,
    }
}

fn svc(
    meta: Arc<InMemArtifactMeta>,
    storage: Arc<InMemStorage>,
    config: EvictionConfig,
) -> EvictionService {
    EvictionService::new(meta, storage, config)
}

// ── TTL tests ────────────────────────────────────────────────────────────

#[tokio::test]
async fn run_ttl_evicts_expired_artifacts() {
    let meta = InMemArtifactMeta::arc();
    let storage = InMemStorage::arc();

    let old = make_meta(
        "artifact:npm/old:1.0",
        "npm",
        "old",
        "1.0",
        100,
        Duration::hours(2),
        Duration::hours(2),
    );
    let fresh = make_meta(
        "artifact:npm/fresh:1.0",
        "npm",
        "fresh",
        "1.0",
        100,
        Duration::minutes(5),
        Duration::minutes(5),
    );
    meta.seed(old.clone());
    meta.seed(fresh.clone());
    storage.seed(&old.artifact_key, b"old-data");
    storage.seed(&fresh.artifact_key, b"fresh-data");

    let config = EvictionConfig {
        artifact_ttl_secs: Some(3600), // 1 hour TTL → "old" (2h) is expired
        registry: "npm".to_owned(),
        ..Default::default()
    };
    let count = svc(meta.clone(), storage.clone(), config)
        .run_ttl()
        .await
        .unwrap();

    assert_eq!(count, 1);
    assert!(
        !storage.contains(&old.artifact_key),
        "expired artifact must be removed from storage"
    );
    assert!(
        storage.contains(&fresh.artifact_key),
        "fresh artifact must remain"
    );
    assert!(
        !meta
            .all()
            .iter()
            .any(|r| r.artifact_key == old.artifact_key),
        "expired meta must be removed"
    );
}

#[tokio::test]
async fn run_ttl_noop_when_not_configured() {
    let meta = InMemArtifactMeta::arc();
    let storage = InMemStorage::arc();
    meta.seed(make_meta(
        "artifact:npm/x:1.0",
        "npm",
        "x",
        "1.0",
        10,
        Duration::days(365),
        Duration::days(365),
    ));
    storage.seed("artifact:npm/x:1.0", b"data");

    let count = svc(
        meta.clone(),
        storage.clone(),
        EvictionConfig {
            artifact_ttl_secs: None,
            registry: "npm".to_owned(),
            ..Default::default()
        },
    )
    .run_ttl()
    .await
    .unwrap();

    assert_eq!(count, 0);
    assert_eq!(storage.keys().len(), 1, "nothing should be deleted");
}

// ── Idle tests ────────────────────────────────────────────────────────────

#[tokio::test]
async fn run_idle_evicts_unaccessed_artifacts() {
    let meta = InMemArtifactMeta::arc();
    let storage = InMemStorage::arc();

    let idle = make_meta(
        "artifact:npm/idle:1.0",
        "npm",
        "idle",
        "1.0",
        50,
        Duration::days(1),
        Duration::days(10),
    );
    let active = make_meta(
        "artifact:npm/active:1.0",
        "npm",
        "active",
        "1.0",
        50,
        Duration::days(1),
        Duration::hours(1),
    );
    meta.seed(idle.clone());
    meta.seed(active.clone());
    storage.seed(&idle.artifact_key, b"data");
    storage.seed(&active.artifact_key, b"data");

    let count = svc(
        meta.clone(),
        storage.clone(),
        EvictionConfig {
            idle_days: Some(7),
            registry: "npm".to_owned(),
            ..Default::default()
        },
    )
    .run_idle()
    .await
    .unwrap();

    assert_eq!(count, 1);
    assert!(!storage.contains(&idle.artifact_key));
    assert!(storage.contains(&active.artifact_key));
}

#[tokio::test]
async fn run_idle_noop_when_not_configured() {
    let meta = InMemArtifactMeta::arc();
    let storage = InMemStorage::arc();
    meta.seed(make_meta(
        "k",
        "npm",
        "p",
        "1.0",
        10,
        Duration::days(1),
        Duration::days(365),
    ));
    storage.seed("k", b"d");

    let count = svc(
        meta,
        storage.clone(),
        EvictionConfig {
            idle_days: None,
            registry: "npm".to_owned(),
            ..Default::default()
        },
    )
    .run_idle()
    .await
    .unwrap();

    assert_eq!(count, 0);
}

// ── Keep-latest-N tests ───────────────────────────────────────────────────

#[tokio::test]
async fn run_keep_latest_n_removes_oldest_versions() {
    let meta = InMemArtifactMeta::arc();
    let storage = InMemStorage::arc();
    let now = Utc::now();

    // 3 versions of "serde", cached at t-3, t-2, t-1 (newest = t-1)
    for (ver, ago) in [("1.0", 3i64), ("2.0", 2), ("3.0", 1)] {
        let m = ArtifactMeta {
            artifact_key: format!("artifact:cargo/serde:{ver}"),
            registry: "cargo".to_owned(),
            package_name: "serde".to_owned(),
            version: ver.to_owned(),
            size_bytes: Some(50),
            cached_at: now - Duration::hours(ago),
            last_accessed_at: now - Duration::hours(ago),
        };
        meta.seed(m.clone());
        storage.seed(&m.artifact_key, b"data");
    }

    let count = svc(
        meta.clone(),
        storage.clone(),
        EvictionConfig {
            keep_latest_n: Some(2),
            registry: "cargo".to_owned(),
            ..Default::default()
        },
    )
    .run_keep_latest_n()
    .await
    .unwrap();

    assert_eq!(count, 1);
    assert!(
        !storage.contains("artifact:cargo/serde:1.0"),
        "oldest (v1.0) must be evicted"
    );
    assert!(
        storage.contains("artifact:cargo/serde:2.0"),
        "v2.0 must remain"
    );
    assert!(
        storage.contains("artifact:cargo/serde:3.0"),
        "v3.0 must remain"
    );
}

#[tokio::test]
async fn run_keep_latest_n_respects_package_boundaries() {
    let meta = InMemArtifactMeta::arc();
    let storage = InMemStorage::arc();
    let now = Utc::now();

    for pkg in ["serde", "tokio"] {
        for (ver, ago) in [("1.0", 3i64), ("2.0", 2), ("3.0", 1)] {
            let m = ArtifactMeta {
                artifact_key: format!("artifact:cargo/{pkg}:{ver}"),
                registry: "cargo".to_owned(),
                package_name: pkg.to_owned(),
                version: ver.to_owned(),
                size_bytes: Some(50),
                cached_at: now - Duration::hours(ago),
                last_accessed_at: now - Duration::hours(ago),
            };
            meta.seed(m.clone());
            storage.seed(&m.artifact_key, b"data");
        }
    }

    let count = svc(
        meta.clone(),
        storage.clone(),
        EvictionConfig {
            keep_latest_n: Some(2),
            registry: "cargo".to_owned(),
            ..Default::default()
        },
    )
    .run_keep_latest_n()
    .await
    .unwrap();

    assert_eq!(count, 2, "one eviction per package");
    assert!(!storage.contains("artifact:cargo/serde:1.0"));
    assert!(!storage.contains("artifact:cargo/tokio:1.0"));
}

#[tokio::test]
async fn run_keep_latest_n_noop_when_not_configured() {
    let meta = InMemArtifactMeta::arc();
    let storage = InMemStorage::arc();
    for ver in ["1.0", "2.0", "3.0"] {
        let m = make_meta(
            &format!("k:{ver}"),
            "npm",
            "pkg",
            ver,
            10,
            Duration::hours(1),
            Duration::hours(1),
        );
        meta.seed(m.clone());
        storage.seed(&m.artifact_key, b"d");
    }

    let count = svc(
        meta,
        storage.clone(),
        EvictionConfig {
            keep_latest_n: None,
            registry: "npm".to_owned(),
            ..Default::default()
        },
    )
    .run_keep_latest_n()
    .await
    .unwrap();

    assert_eq!(count, 0);
    assert_eq!(storage.keys().len(), 3);
}

// ── LRU size cap tests ────────────────────────────────────────────────────

#[tokio::test]
async fn run_lru_size_cap_evicts_until_under_cap() {
    let meta = InMemArtifactMeta::arc();
    let storage = InMemStorage::arc();
    let now = Utc::now();

    // 3 artifacts: sizes 100+100+100 = 300 bytes, cap = 150
    // LRU order: "a" (oldest), "b", "c" (newest)
    let artifacts = [
        ("artifact:npm/a:1.0", "a", 100i64, 3i64),
        ("artifact:npm/b:1.0", "b", 100, 2),
        ("artifact:npm/c:1.0", "c", 100, 1),
    ];
    for (key, pkg, size, accessed_ago) in artifacts {
        let m = ArtifactMeta {
            artifact_key: key.to_owned(),
            registry: "npm".to_owned(),
            package_name: pkg.to_owned(),
            version: "1.0".to_owned(),
            size_bytes: Some(size as u64),
            cached_at: now - Duration::hours(accessed_ago),
            last_accessed_at: now - Duration::hours(accessed_ago),
        };
        meta.seed(m);
        storage.seed(key, b"x".repeat(size as usize).as_slice());
    }

    let count = svc(
        meta.clone(),
        storage.clone(),
        EvictionConfig {
            max_size_bytes: Some(150),
            registry: "npm".to_owned(),
            ..Default::default()
        },
    )
    .run_lru_size_cap()
    .await
    .unwrap();

    assert!(count >= 1, "at least one artifact must be evicted");
    assert!(
        meta.all()
            .iter()
            .map(|r| r.size_bytes.unwrap_or(0))
            .sum::<u64>()
            <= 150,
        "total remaining size must be within cap"
    );
    // "a" (oldest LRU) must have been evicted first
    assert!(
        !storage.contains("artifact:npm/a:1.0"),
        "LRU artifact must be evicted first"
    );
}

#[tokio::test]
async fn run_lru_size_cap_noop_when_under_cap() {
    let meta = InMemArtifactMeta::arc();
    let storage = InMemStorage::arc();
    meta.seed(make_meta(
        "k",
        "npm",
        "p",
        "1.0",
        50,
        Duration::hours(1),
        Duration::hours(1),
    ));
    storage.seed("k", b"data");

    let count = svc(
        meta,
        storage.clone(),
        EvictionConfig {
            max_size_bytes: Some(1_000_000),
            registry: "npm".to_owned(),
            ..Default::default()
        },
    )
    .run_lru_size_cap()
    .await
    .unwrap();

    assert_eq!(count, 0);
    assert_eq!(storage.keys().len(), 1);
}

#[tokio::test]
async fn run_lru_size_cap_noop_when_not_configured() {
    let meta = InMemArtifactMeta::arc();
    let storage = InMemStorage::arc();
    meta.seed(make_meta(
        "k",
        "npm",
        "p",
        "1.0",
        999_999,
        Duration::hours(1),
        Duration::hours(1),
    ));
    storage.seed("k", b"data");

    let count = svc(
        meta,
        storage.clone(),
        EvictionConfig {
            max_size_bytes: None,
            registry: "npm".to_owned(),
            ..Default::default()
        },
    )
    .run_lru_size_cap()
    .await
    .unwrap();

    assert_eq!(count, 0);
}

// ── run_all() tests ───────────────────────────────────────────────────────

#[tokio::test]
async fn run_all_applies_all_enabled_strategies() {
    let meta = InMemArtifactMeta::arc();
    let storage = InMemStorage::arc();

    // Artifact qualifies for both TTL and idle eviction
    let old = make_meta(
        "artifact:npm/old:1.0",
        "npm",
        "old",
        "1.0",
        100,
        Duration::hours(25),
        Duration::days(10),
    );
    meta.seed(old.clone());
    storage.seed(&old.artifact_key, b"data");

    let config = EvictionConfig {
        artifact_ttl_secs: Some(3600 * 24), // 24h TTL, artifact is 25h old
        idle_days: Some(7),
        keep_latest_n: Some(5), // high enough not to evict anything extra
        max_size_bytes: Some(10_000_000), // high enough not to evict anything extra
        registry: "npm".to_owned(),
    };
    let report = svc(meta, storage.clone(), config).run_all().await.unwrap();

    assert!(report.total >= 1);
    assert!(!storage.contains(&old.artifact_key));
}

#[tokio::test]
async fn run_all_noop_when_all_strategies_disabled() {
    let meta = InMemArtifactMeta::arc();
    let storage = InMemStorage::arc();
    meta.seed(make_meta(
        "k",
        "npm",
        "p",
        "1.0",
        10,
        Duration::days(365),
        Duration::days(365),
    ));
    storage.seed("k", b"data");

    let report = svc(
        meta,
        storage.clone(),
        EvictionConfig {
            registry: "npm".to_owned(),
            ..Default::default()
        },
    )
    .run_all()
    .await
    .unwrap();

    assert_eq!(report.total, 0);
    assert_eq!(storage.keys().len(), 1, "nothing should be deleted");
}

// ── Coherence check tests ─────────────────────────────────────────────────

#[tokio::test]
async fn coherence_check_deletes_orphaned_storage_objects() {
    let meta = InMemArtifactMeta::arc();
    let storage = InMemStorage::arc();

    // 3 storage objects, only 2 in meta → 1 orphan
    storage.seed("artifact:npm/a:1.0", b"data");
    storage.seed("artifact:npm/b:1.0", b"data");
    storage.seed("artifact:npm/orphan:1.0", b"orphan");

    meta.seed(make_meta(
        "artifact:npm/a:1.0",
        "npm",
        "a",
        "1.0",
        10,
        Duration::hours(1),
        Duration::hours(1),
    ));
    meta.seed(make_meta(
        "artifact:npm/b:1.0",
        "npm",
        "b",
        "1.0",
        10,
        Duration::hours(1),
        Duration::hours(1),
    ));

    let report = svc(
        meta,
        storage.clone(),
        EvictionConfig {
            registry: "npm".to_owned(),
            ..Default::default()
        },
    )
    .run_coherence_check()
    .await
    .unwrap();

    assert_eq!(report.orphaned_deleted, 1);
    assert_eq!(report.storage_keys, 3);
    assert_eq!(report.meta_rows, 2);
    assert!(
        !storage.contains("artifact:npm/orphan:1.0"),
        "orphan must be deleted from storage"
    );
    assert!(
        storage.contains("artifact:npm/a:1.0"),
        "tracked artifact must remain"
    );
    assert!(
        storage.contains("artifact:npm/b:1.0"),
        "tracked artifact must remain"
    );
}

#[tokio::test]
async fn coherence_check_clean_when_no_orphans() {
    let meta = InMemArtifactMeta::arc();
    let storage = InMemStorage::arc();

    storage.seed("artifact:npm/a:1.0", b"data");
    meta.seed(make_meta(
        "artifact:npm/a:1.0",
        "npm",
        "a",
        "1.0",
        10,
        Duration::hours(1),
        Duration::hours(1),
    ));

    let report = svc(
        meta,
        storage,
        EvictionConfig {
            registry: "npm".to_owned(),
            ..Default::default()
        },
    )
    .run_coherence_check()
    .await
    .unwrap();

    assert_eq!(report.orphaned_deleted, 0);
}

#[tokio::test]
async fn coherence_check_empty_registry_spans_all_namespaces() {
    let meta = InMemArtifactMeta::arc();
    let storage = InMemStorage::arc();

    storage.seed("artifact:npm/x:1.0", b"data");
    storage.seed("artifact:cargo/y:1.0", b"data");
    // only npm is tracked in meta — cargo artifact is orphaned
    meta.seed(make_meta(
        "artifact:npm/x:1.0",
        "npm",
        "x",
        "1.0",
        10,
        Duration::hours(1),
        Duration::hours(1),
    ));

    let report = svc(
        meta,
        storage.clone(),
        EvictionConfig {
            registry: "".to_owned(), // empty = all namespaces
            ..Default::default()
        },
    )
    .run_coherence_check()
    .await
    .unwrap();

    assert_eq!(
        report.orphaned_deleted, 1,
        "orphan in cargo namespace must be found"
    );
    assert!(!storage.contains("artifact:cargo/y:1.0"));
    assert!(storage.contains("artifact:npm/x:1.0"));
}

// ── Error-path tests ──────────────────────────────────────────────────────

struct FailStorage;

#[async_trait]
impl StorageBackend for FailStorage {
    async fn store(&self, _: &str, _: Bytes, _: StorageMeta) -> Result<(), CoreError> {
        Ok(())
    }
    async fn retrieve(&self, _: &str) -> Result<Option<StoredArtifact>, CoreError> {
        Ok(None)
    }
    async fn exists(&self, _: &str) -> Result<bool, CoreError> {
        Ok(false)
    }
    async fn delete(&self, _: &str) -> Result<bool, CoreError> {
        Err(CoreError::Storage("injected delete failure".into()))
    }
    async fn delete_by_prefix(&self, _: &str) -> Result<usize, CoreError> {
        Ok(0)
    }
    async fn stat_by_prefix(&self, _: &str) -> Result<(u64, u64), CoreError> {
        Ok((0, 0))
    }
    async fn list_keys(&self, _: &str) -> Result<Vec<String>, CoreError> {
        Ok(vec![])
    }
}

#[tokio::test]
async fn run_ttl_storage_error_skips_artifact_but_continues() {
    let meta = InMemArtifactMeta::arc();
    let storage = Arc::new(FailStorage);

    let old = make_meta(
        "artifact:npm/old:1.0",
        "npm",
        "old",
        "1.0",
        100,
        Duration::hours(2),
        Duration::hours(2),
    );
    meta.seed(old);

    // Storage delete will fail → error path is exercised
    let count = EvictionService::new(
        meta,
        storage,
        EvictionConfig {
            artifact_ttl_secs: Some(3600),
            registry: "npm".to_owned(),
            ..Default::default()
        },
    )
    .run_ttl()
    .await
    .unwrap();

    assert_eq!(
        count, 0,
        "failed storage delete means artifact is not counted as evicted"
    );
}

#[tokio::test]
async fn run_idle_storage_error_skips_artifact_but_continues() {
    let meta = InMemArtifactMeta::arc();
    let storage = Arc::new(FailStorage);

    let idle = make_meta(
        "artifact:npm/idle:1.0",
        "npm",
        "idle",
        "1.0",
        50,
        Duration::days(1),
        Duration::days(10),
    );
    meta.seed(idle);

    let count = EvictionService::new(
        meta,
        storage,
        EvictionConfig {
            idle_days: Some(7),
            registry: "npm".to_owned(),
            ..Default::default()
        },
    )
    .run_idle()
    .await
    .unwrap();

    assert_eq!(count, 0);
}

#[tokio::test]
async fn run_keep_latest_n_storage_error_skips_artifact() {
    let meta = InMemArtifactMeta::arc();
    let storage = Arc::new(FailStorage);
    let now = Utc::now();

    for (ver, ago) in [("1.0", 3i64), ("2.0", 2), ("3.0", 1)] {
        meta.seed(ArtifactMeta {
            artifact_key: format!("artifact:cargo/serde:{ver}"),
            registry: "cargo".to_owned(),
            package_name: "serde".to_owned(),
            version: ver.to_owned(),
            size_bytes: Some(50),
            cached_at: now - Duration::hours(ago),
            last_accessed_at: now - Duration::hours(ago),
        });
    }

    let count = EvictionService::new(
        meta,
        storage,
        EvictionConfig {
            keep_latest_n: Some(2),
            registry: "cargo".to_owned(),
            ..Default::default()
        },
    )
    .run_keep_latest_n()
    .await
    .unwrap();

    assert_eq!(count, 0);
}
