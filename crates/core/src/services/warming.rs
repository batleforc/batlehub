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
                    return WarmingReport { errors: 1, ..Default::default() };
                }
            }
        };

        let sem = Arc::new(Semaphore::new(self.concurrency));
        let mut handles = Vec::with_capacity(versions.len());

        for version in versions {
            let artifact_key = format!(
                "artifact:{}/{name}:{version}",
                self.registry_name
            );
            let storage = Arc::clone(&self.storage);
            let artifact_meta = Arc::clone(&self.artifact_meta);
            let client = Arc::clone(&self.client);
            let registry_name = self.registry_name.clone();
            let name = name.to_owned();
            let sem = Arc::clone(&sem);

            handles.push(tokio::spawn(async move {
                let _permit = sem.acquire_owned().await;

                // Skip if already cached.
                match storage.exists(&artifact_key).await {
                    Ok(true) => return WarmingReport { skipped: 1, ..Default::default() },
                    Ok(false) => {}
                    Err(e) => {
                        tracing::warn!(error = %e, key = %artifact_key, "warming: exists check failed");
                        return WarmingReport { errors: 1, ..Default::default() };
                    }
                }

                let pkg = PackageId::new(registry_name.clone(), name.clone(), version.clone());

                // Fetch from upstream.
                let fetched = match client.fetch_artifact(&pkg).await {
                    Ok(f) => f,
                    Err(e) => {
                        tracing::warn!(
                            registry = %registry_name, package = %name,
                            version = %version, error = %e,
                            "warming: fetch failed"
                        );
                        return WarmingReport { errors: 1, ..Default::default() };
                    }
                };

                // Buffer the stream.
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
                            return WarmingReport { errors: 1, ..Default::default() };
                        }
                    }
                }
                let data = Bytes::from(buf);
                let size = data.len() as u64;

                // Store.
                if let Err(e) = storage
                    .store(&artifact_key, data, StorageMeta { size: Some(size), ..Default::default() })
                    .await
                {
                    tracing::warn!(error = %e, key = %artifact_key, "warming: store failed");
                    return WarmingReport { errors: 1, ..Default::default() };
                }

                // Record metadata for eviction tracking.
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
                WarmingReport { warmed: 1, ..Default::default() }
            }));
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
        fn registry_type(&self) -> &str { "test" }
        async fn resolve_metadata(&self, _: &PackageId) -> Result<PackageMetadata, CoreError> { panic!("should not be called") }
        async fn fetch_artifact(&self, _: &PackageId) -> Result<FetchedArtifact, CoreError> { panic!("should not be called") }
        async fn list_versions(&self, _: &str) -> Result<Vec<String>, CoreError> { panic!("should not be called") }
    }

    #[async_trait]
    impl StorageBackend for PanicStorage {
        async fn store(&self, _: &str, _: bytes::Bytes, _: StorageMeta) -> Result<(), CoreError> { panic!("should not be called") }
        async fn retrieve(&self, _: &str) -> Result<Option<StoredArtifact>, CoreError> { panic!("should not be called") }
        async fn exists(&self, _: &str) -> Result<bool, CoreError> { panic!("should not be called") }
        async fn delete(&self, _: &str) -> Result<(), CoreError> { panic!("should not be called") }
        async fn delete_by_prefix(&self, _: &str) -> Result<usize, CoreError> { panic!("should not be called") }
        async fn stat_by_prefix(&self, _: &str) -> Result<(u64, u64), CoreError> { panic!("should not be called") }
        async fn list_keys(&self, _: &str) -> Result<Vec<String>, CoreError> { panic!("should not be called") }
    }

    #[async_trait]
    impl ArtifactMetaRepository for PanicMeta {
        async fn record_artifact(&self, _: &str, _: &str, _: &str, _: &str, _: Option<u64>) -> Result<(), CoreError> { panic!("should not be called") }
        async fn touch_artifact(&self, _: &str) -> Result<(), CoreError> { panic!("should not be called") }
        async fn list_artifacts(&self, _: &str) -> Result<Vec<ArtifactMeta>, CoreError> { panic!("should not be called") }
        async fn list_artifacts_by_package(&self) -> Result<Vec<ArtifactMeta>, CoreError> { panic!("should not be called") }
        async fn delete_artifact_meta(&self, _: &str) -> Result<(), CoreError> { panic!("should not be called") }
        async fn list_expired_by_ttl(&self, _: &str, _: chrono::DateTime<Utc>) -> Result<Vec<ArtifactMeta>, CoreError> { panic!("should not be called") }
        async fn list_idle(&self, _: &str, _: chrono::DateTime<Utc>) -> Result<Vec<ArtifactMeta>, CoreError> { panic!("should not be called") }
        async fn total_size_bytes(&self, _: &str) -> Result<u64, CoreError> { panic!("should not be called") }
        async fn list_lru(&self, _: &str, _: i64) -> Result<Vec<ArtifactMeta>, CoreError> { panic!("should not be called") }
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
}
