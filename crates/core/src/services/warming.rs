use std::sync::Arc;

use bytes::Bytes;
use futures::StreamExt;
use tokio::sync::Semaphore;

use crate::{
    entities::PackageId,
    ports::{ArtifactMetaRepository, RegistryClient, StorageBackend, StorageMeta},
};

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
pub struct WarmingService {
    pub client: Arc<dyn RegistryClient>,
    pub storage: Arc<dyn StorageBackend>,
    pub artifact_meta: Arc<dyn ArtifactMetaRepository>,
    pub registry_name: String,
    /// How many of the most-recent versions to warm per package.
    /// Ignored when the package string includes a pinned version (e.g. `"lodash@4.17.21"`).
    pub latest_n: usize,
    /// Maximum concurrent artifact downloads.
    pub concurrency: usize,
}

/// Fetch and store one artifact version. Returns a single-field `WarmingReport`.
#[allow(clippy::too_many_arguments)]
async fn warm_one_version(
    client: Arc<dyn RegistryClient>,
    storage: Arc<dyn StorageBackend>,
    artifact_meta: Arc<dyn ArtifactMetaRepository>,
    artifact_key: String,
    pkg: PackageId,
    registry_name: String,
    name: String,
    version: String,
    sem: Arc<Semaphore>,
) -> WarmingReport {
    let _permit = sem.acquire_owned().await;

    match storage.exists(&artifact_key).await {
        Ok(true) => {
            return WarmingReport {
                skipped: 1,
                ..Default::default()
            }
        }
        Ok(false) => {}
        Err(e) => {
            tracing::warn!(error = %e, key = %artifact_key, "warming: exists check failed");
            return WarmingReport {
                errors: 1,
                ..Default::default()
            };
        }
    }

    let fetched = match client.fetch_artifact(&pkg).await {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!(
                registry = %registry_name, package = %name,
                version = %version, error = %e,
                "warming: fetch failed"
            );
            return WarmingReport {
                errors: 1,
                ..Default::default()
            };
        }
    };

    let mut buf = Vec::new();
    let mut stream = fetched.stream;
    while let Some(chunk) = stream.next().await {
        match chunk {
            Ok(b) => buf.extend_from_slice(&b),
            Err(e) => {
                tracing::warn!(
                    registry = %registry_name, package = %name,
                    version = %version, error = %e,
                    "warming: stream error"
                );
                return WarmingReport {
                    errors: 1,
                    ..Default::default()
                };
            }
        }
    }
    let data = Bytes::from(buf);
    let size = data.len() as u64;

    if let Err(e) = storage
        .store(
            &artifact_key,
            data,
            StorageMeta {
                size: Some(size),
                ..Default::default()
            },
        )
        .await
    {
        tracing::warn!(error = %e, key = %artifact_key, "warming: store failed");
        return WarmingReport {
            errors: 1,
            ..Default::default()
        };
    }

    if let Err(e) = artifact_meta
        .record_artifact(&artifact_key, &registry_name, &name, &version, Some(size))
        .await
    {
        tracing::warn!(error = %e, key = %artifact_key, "warming: record_artifact failed");
    }

    tracing::info!(
        registry = %registry_name, package = %name,
        version = %version, bytes = size,
        "warming: artifact cached"
    );
    WarmingReport {
        warmed: 1,
        ..Default::default()
    }
}

impl WarmingService {
    /// Return a new `WarmingService` identical to `self` but with a different `latest_n`.
    /// Used by the admin API to honour a per-request version count override.
    pub fn with_latest_n(&self, n: usize) -> Self {
        Self {
            client: Arc::clone(&self.client),
            storage: Arc::clone(&self.storage),
            artifact_meta: Arc::clone(&self.artifact_meta),
            registry_name: self.registry_name.clone(),
            latest_n: n,
            concurrency: self.concurrency,
        }
    }

    /// Warm a single package.
    ///
    /// If `package` contains an `@version` suffix (e.g. `"lodash@4.17.21"`), only
    /// that exact version is warmed regardless of `self.latest_n`. Otherwise the
    /// latest `self.latest_n` versions are fetched.
    pub async fn warm_package(&self, package: &str) -> WarmingReport {
        if self.concurrency == 0 {
            return WarmingReport::default();
        }

        let (name, pinned_version) = if let Some((n, v)) = package.split_once('@') {
            (n, Some(v.to_owned()))
        } else {
            (package, None)
        };

        let versions: Vec<String> = if let Some(v) = pinned_version {
            vec![v]
        } else {
            match self.client.list_versions(name).await {
                Ok(v) => {
                    let n = self.latest_n;
                    v.into_iter().rev().take(n).collect()
                }
                Err(e) => {
                    tracing::warn!(
                        registry = %self.registry_name,
                        package = name,
                        error = %e,
                        "warming: failed to list versions"
                    );
                    return WarmingReport {
                        errors: 1,
                        ..Default::default()
                    };
                }
            }
        };

        let sem = Arc::new(Semaphore::new(self.concurrency));
        let mut handles = Vec::with_capacity(versions.len());

        for version in versions {
            let artifact_key = format!("artifact:{}/{name}:{version}", self.registry_name);
            let pkg = PackageId::new(self.registry_name.clone(), name.to_owned(), version.clone());
            handles.push(tokio::spawn(warm_one_version(
                Arc::clone(&self.client),
                Arc::clone(&self.storage),
                Arc::clone(&self.artifact_meta),
                artifact_key,
                pkg,
                self.registry_name.clone(),
                name.to_owned(),
                version,
                Arc::clone(&sem),
            )));
        }

        let mut total = WarmingReport::default();
        for handle in handles {
            match handle.await {
                Ok(r) => total += r,
                Err(e) => {
                    tracing::warn!(error = %e, "warming: task panicked");
                    total.errors += 1;
                }
            }
        }
        total
    }

    /// Warm every entry in `packages`. Each entry may be `"name"` or `"name@version"`.
    pub async fn warm_all(&self, packages: &[String]) -> WarmingReport {
        let mut total = WarmingReport::default();
        for package in packages {
            total += self.warm_package(package).await;
        }
        total
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;
    use crate::{
        entities::{PackageId, PackageMetadata},
        error::CoreError,
        ports::{
            ArtifactMeta, ArtifactMetaRepository, FetchedArtifact, RegistryClient, StorageBackend,
            StorageMeta, StoredArtifact,
        },
    };
    use async_trait::async_trait;

    struct PanicClient;
    struct PanicStorage;
    struct PanicMeta;

    #[async_trait]
    impl RegistryClient for PanicClient {
        fn registry_type(&self) -> &str {
            "test"
        }
        async fn resolve_metadata(&self, _: &PackageId) -> Result<PackageMetadata, CoreError> {
            panic!("should not be called")
        }
        async fn fetch_artifact(&self, _: &PackageId) -> Result<FetchedArtifact, CoreError> {
            panic!("should not be called")
        }
        async fn list_versions(&self, _: &str) -> Result<Vec<String>, CoreError> {
            panic!("should not be called")
        }
    }

    #[async_trait]
    impl StorageBackend for PanicStorage {
        async fn store(&self, _: &str, _: bytes::Bytes, _: StorageMeta) -> Result<(), CoreError> {
            panic!("should not be called")
        }
        async fn retrieve(&self, _: &str) -> Result<Option<StoredArtifact>, CoreError> {
            panic!("should not be called")
        }
        async fn exists(&self, _: &str) -> Result<bool, CoreError> {
            panic!("should not be called")
        }
        async fn delete(&self, _: &str) -> Result<(), CoreError> {
            panic!("should not be called")
        }
        async fn delete_by_prefix(&self, _: &str) -> Result<usize, CoreError> {
            panic!("should not be called")
        }
        async fn stat_by_prefix(&self, _: &str) -> Result<(u64, u64), CoreError> {
            panic!("should not be called")
        }
        async fn list_keys(&self, _: &str) -> Result<Vec<String>, CoreError> {
            panic!("should not be called")
        }
    }

    #[async_trait]
    impl ArtifactMetaRepository for PanicMeta {
        async fn record_artifact(
            &self,
            _: &str,
            _: &str,
            _: &str,
            _: &str,
            _: Option<u64>,
        ) -> Result<(), CoreError> {
            panic!("should not be called")
        }
        async fn touch_artifact(&self, _: &str) -> Result<(), CoreError> {
            panic!("should not be called")
        }
        async fn list_artifacts(&self, _: &str) -> Result<Vec<ArtifactMeta>, CoreError> {
            panic!("should not be called")
        }
        async fn list_artifacts_by_package(&self) -> Result<Vec<ArtifactMeta>, CoreError> {
            panic!("should not be called")
        }
        async fn delete_artifact_meta(&self, _: &str) -> Result<(), CoreError> {
            panic!("should not be called")
        }
        async fn is_artifact_expired(
            &self,
            _: &str,
            _: chrono::DateTime<Utc>,
        ) -> Result<bool, CoreError> {
            panic!("should not be called")
        }
        async fn list_expired_by_ttl(
            &self,
            _: &str,
            _: chrono::DateTime<Utc>,
        ) -> Result<Vec<ArtifactMeta>, CoreError> {
            panic!("should not be called")
        }
        async fn list_idle(
            &self,
            _: &str,
            _: chrono::DateTime<Utc>,
        ) -> Result<Vec<ArtifactMeta>, CoreError> {
            panic!("should not be called")
        }
        async fn total_size_bytes(&self, _: &str) -> Result<u64, CoreError> {
            panic!("should not be called")
        }
        async fn list_lru(&self, _: &str, _: i64) -> Result<Vec<ArtifactMeta>, CoreError> {
            panic!("should not be called")
        }
    }

    fn disabled_svc() -> WarmingService {
        WarmingService {
            client: Arc::new(PanicClient),
            storage: Arc::new(PanicStorage),
            artifact_meta: Arc::new(PanicMeta),
            registry_name: "test".into(),
            latest_n: 3,
            concurrency: 0,
        }
    }

    #[tokio::test]
    async fn concurrency_zero_disables_warm_package() {
        let svc = disabled_svc();
        let report = svc.warm_package("lodash").await;
        assert_eq!(report.warmed, 0);
        assert_eq!(report.skipped, 0);
        assert_eq!(report.errors, 0);
    }

    #[tokio::test]
    async fn concurrency_zero_disables_warm_all() {
        let svc = disabled_svc();
        let report = svc.warm_all(&["lodash".into(), "react".into()]).await;
        assert_eq!(report.warmed, 0);
        assert_eq!(report.skipped, 0);
        assert_eq!(report.errors, 0);
    }

    // ── Functional mocks for active warming tests ─────────────────────────────

    use futures::stream;
    use std::collections::HashMap;
    use tokio::sync::Mutex as TokioMutex;

    struct StubClient {
        versions: Vec<String>,
        fail_fetch: bool,
        fail_list: bool,
    }

    impl StubClient {
        fn with_versions(versions: Vec<&str>) -> Arc<Self> {
            Arc::new(Self {
                versions: versions.into_iter().map(str::to_owned).collect(),
                fail_fetch: false,
                fail_list: false,
            })
        }
        fn failing_list() -> Arc<Self> {
            Arc::new(Self {
                versions: vec![],
                fail_fetch: false,
                fail_list: true,
            })
        }
        fn failing_fetch() -> Arc<Self> {
            Arc::new(Self {
                versions: vec!["1.0.0".into()],
                fail_fetch: true,
                fail_list: false,
            })
        }
    }

    #[async_trait]
    impl RegistryClient for StubClient {
        fn registry_type(&self) -> &str {
            "stub"
        }
        async fn resolve_metadata(&self, _: &PackageId) -> Result<PackageMetadata, CoreError> {
            Ok(PackageMetadata {
                id: PackageId::new("stub", "pkg", "0.0.0"),
                published_at: None,
                download_url: None,
                checksum: None,
                is_signed: None,
                extra: serde_json::Value::Null,
                cache_control: None,
            })
        }
        async fn fetch_artifact(&self, _: &PackageId) -> Result<FetchedArtifact, CoreError> {
            if self.fail_fetch {
                return Err(CoreError::Registry("fetch failed".into()));
            }
            let data = Bytes::from("stub-artifact-data");
            Ok(FetchedArtifact {
                stream: Box::pin(stream::once(async move { Ok::<Bytes, CoreError>(data) })),
                cache_control: None,
            })
        }
        async fn list_versions(&self, _: &str) -> Result<Vec<String>, CoreError> {
            if self.fail_list {
                return Err(CoreError::Registry("list failed".into()));
            }
            Ok(self.versions.clone())
        }
    }

    struct StubStorage {
        data: Arc<TokioMutex<HashMap<String, Bytes>>>,
        fail_store: bool,
    }

    impl StubStorage {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                data: Arc::new(TokioMutex::new(HashMap::new())),
                fail_store: false,
            })
        }
        fn failing_store() -> Arc<Self> {
            Arc::new(Self {
                data: Arc::new(TokioMutex::new(HashMap::new())),
                fail_store: true,
            })
        }
    }

    #[async_trait]
    impl StorageBackend for StubStorage {
        async fn store(&self, key: &str, data: Bytes, _: StorageMeta) -> Result<(), CoreError> {
            if self.fail_store {
                return Err(CoreError::Storage("store failed".into()));
            }
            self.data.lock().await.insert(key.to_owned(), data);
            Ok(())
        }
        async fn retrieve(&self, key: &str) -> Result<Option<StoredArtifact>, CoreError> {
            Ok(self.data.lock().await.get(key).map(|d| StoredArtifact {
                stream: Box::pin(stream::once({
                    let b = d.clone();
                    async move { Ok::<Bytes, CoreError>(b) }
                })),
                meta: StorageMeta::default(),
            }))
        }
        async fn exists(&self, key: &str) -> Result<bool, CoreError> {
            Ok(self.data.lock().await.contains_key(key))
        }
        async fn delete(&self, key: &str) -> Result<(), CoreError> {
            self.data.lock().await.remove(key);
            Ok(())
        }
        async fn delete_by_prefix(&self, prefix: &str) -> Result<usize, CoreError> {
            let mut m = self.data.lock().await;
            let before = m.len();
            m.retain(|k, _| !k.starts_with(prefix));
            Ok(before - m.len())
        }
        async fn stat_by_prefix(&self, prefix: &str) -> Result<(u64, u64), CoreError> {
            let m = self.data.lock().await;
            let matching: Vec<_> = m.iter().filter(|(k, _)| k.starts_with(prefix)).collect();
            Ok((
                matching.len() as u64,
                matching.iter().map(|(_, v)| v.len() as u64).sum(),
            ))
        }
        async fn list_keys(&self, prefix: &str) -> Result<Vec<String>, CoreError> {
            Ok(self
                .data
                .lock()
                .await
                .keys()
                .filter(|k| k.starts_with(prefix))
                .cloned()
                .collect())
        }
    }

    struct NoopMeta;
    #[async_trait]
    impl ArtifactMetaRepository for NoopMeta {
        async fn record_artifact(
            &self,
            _: &str,
            _: &str,
            _: &str,
            _: &str,
            _: Option<u64>,
        ) -> Result<(), CoreError> {
            Ok(())
        }
        async fn touch_artifact(&self, _: &str) -> Result<(), CoreError> {
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
            _: &str,
            _: chrono::DateTime<Utc>,
        ) -> Result<bool, CoreError> {
            Ok(false)
        }
        async fn list_expired_by_ttl(
            &self,
            _: &str,
            _: chrono::DateTime<Utc>,
        ) -> Result<Vec<ArtifactMeta>, CoreError> {
            Ok(vec![])
        }
        async fn list_idle(
            &self,
            _: &str,
            _: chrono::DateTime<Utc>,
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

    fn active_svc(
        client: Arc<dyn RegistryClient>,
        storage: Arc<dyn StorageBackend>,
    ) -> WarmingService {
        WarmingService {
            client,
            storage,
            artifact_meta: Arc::new(NoopMeta),
            registry_name: "test-reg".into(),
            latest_n: 3,
            concurrency: 4,
        }
    }

    #[tokio::test]
    async fn warm_package_fetches_and_stores_new_version() {
        let storage = StubStorage::new();
        let svc = active_svc(StubClient::with_versions(vec!["1.0.0"]), storage.clone());
        let report = svc.warm_package("mylib@1.0.0").await;
        assert_eq!(report.warmed, 1);
        assert_eq!(report.skipped, 0);
        assert_eq!(report.errors, 0);
        let key = "artifact:test-reg/mylib:1.0.0";
        assert!(storage.exists(key).await.unwrap());
    }

    #[tokio::test]
    async fn warm_package_skips_already_cached_version() {
        let storage = StubStorage::new();
        storage
            .store(
                "artifact:test-reg/mylib:1.0.0",
                Bytes::from("old"),
                StorageMeta::default(),
            )
            .await
            .unwrap();
        let svc = active_svc(StubClient::with_versions(vec!["1.0.0"]), storage.clone());
        let report = svc.warm_package("mylib@1.0.0").await;
        assert_eq!(report.warmed, 0);
        assert_eq!(report.skipped, 1);
        assert_eq!(report.errors, 0);
    }

    #[tokio::test]
    async fn warm_package_lists_versions_and_warms_latest_n() {
        let storage = StubStorage::new();
        let svc = WarmingService {
            client: StubClient::with_versions(vec!["1.0.0", "1.1.0", "1.2.0", "2.0.0"]),
            storage: storage.clone(),
            artifact_meta: Arc::new(NoopMeta),
            registry_name: "test-reg".into(),
            latest_n: 2,
            concurrency: 4,
        };
        let report = svc.warm_package("mylib").await;
        assert_eq!(report.warmed, 2);
        assert_eq!(report.skipped, 0);
        assert_eq!(report.errors, 0);
    }

    #[tokio::test]
    async fn warm_package_returns_error_when_list_versions_fails() {
        let svc = active_svc(StubClient::failing_list(), StubStorage::new());
        let report = svc.warm_package("mylib").await;
        assert_eq!(report.errors, 1);
        assert_eq!(report.warmed, 0);
    }

    #[tokio::test]
    async fn warm_package_records_error_when_fetch_fails() {
        let svc = active_svc(StubClient::failing_fetch(), StubStorage::new());
        let report = svc.warm_package("mylib@1.0.0").await;
        assert_eq!(report.errors, 1);
        assert_eq!(report.warmed, 0);
    }

    #[tokio::test]
    async fn warm_package_records_error_when_store_fails() {
        let svc = active_svc(
            StubClient::with_versions(vec!["1.0.0"]),
            StubStorage::failing_store(),
        );
        let report = svc.warm_package("mylib@1.0.0").await;
        assert_eq!(report.errors, 1);
        assert_eq!(report.warmed, 0);
    }

    #[tokio::test]
    async fn warm_all_aggregates_multiple_packages() {
        let storage = StubStorage::new();
        let svc = active_svc(StubClient::with_versions(vec!["1.0.0"]), storage.clone());
        let report = svc
            .warm_all(&["pkgA@1.0.0".to_string(), "pkgB@1.0.0".to_string()])
            .await;
        assert_eq!(report.warmed, 2);
        assert_eq!(report.errors, 0);
    }

    #[tokio::test]
    async fn with_latest_n_creates_new_service_with_different_n() {
        let svc = active_svc(StubClient::with_versions(vec![]), StubStorage::new());
        assert_eq!(svc.latest_n, 3);
        let svc2 = svc.with_latest_n(10);
        assert_eq!(svc2.latest_n, 10);
        assert_eq!(svc2.registry_name, "test-reg");
    }

    #[tokio::test]
    async fn warming_report_add_assign_aggregates() {
        let mut a = WarmingReport {
            warmed: 1,
            skipped: 2,
            errors: 3,
        };
        let b = WarmingReport {
            warmed: 10,
            skipped: 20,
            errors: 30,
        };
        a += b;
        assert_eq!(a.warmed, 11);
        assert_eq!(a.skipped, 22);
        assert_eq!(a.errors, 33);
    }
}
