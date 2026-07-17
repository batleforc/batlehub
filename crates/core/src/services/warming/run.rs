use std::sync::Arc;

use tokio::sync::Semaphore;

use super::{WarmingReport, WarmingService, WARM_CLAIM_TTL};
use crate::{
    entities::PackageId,
    ports::{ArtifactMetaRecord, StorageMeta},
};

/// Fetch and store one artifact version. Returns a single-field `WarmingReport`.
///
/// Takes an owned `svc` (a cloned `WarmingService`) rather than its four
/// `Arc` fields plus `registry_name` individually — every caller already has
/// a `WarmingService` in hand, so `svc.clone()` at the spawn site replaces
/// four separate `Arc::clone` calls with one.
async fn warm_one_version(
    svc: WarmingService,
    artifact_key: String,
    pkg: PackageId,
    name: String,
    version: String,
    sem: Arc<Semaphore>,
) -> WarmingReport {
    let _permit = sem.acquire_owned().await;

    if !svc
        .coordinator
        .try_claim(&artifact_key, WARM_CLAIM_TTL)
        .await
    {
        tracing::debug!(
            registry = %svc.registry_name, package = %name,
            version = %version, key = %artifact_key,
            "warming: skipped — another replica is warming this artifact"
        );
        return WarmingReport {
            skipped: 1,
            ..Default::default()
        };
    }

    let report = warm_one_version_inner(&svc, &artifact_key, pkg, &name, &version).await;

    svc.coordinator.release(&artifact_key).await;
    report
}

async fn warm_one_version_inner(
    svc: &WarmingService,
    artifact_key: &str,
    pkg: PackageId,
    name: &str,
    version: &str,
) -> WarmingReport {
    let registry_name = svc.registry_name.as_str();
    match svc.storage.exists(artifact_key).await {
        Ok(true) => {
            tracing::debug!(
                registry = %registry_name, package = %name,
                version = %version, key = %artifact_key,
                "warming: skipped — artifact already in cache"
            );
            return WarmingReport {
                skipped: 1,
                ..Default::default()
            };
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

    let registry_label: Arc<str> = Arc::from(registry_name);
    let upstream_start = std::time::Instant::now();
    let mut fetched = match svc.client.fetch_artifact(&pkg).await {
        Ok(f) => {
            svc.metrics.record_upstream_outcome(registry_name, true);
            f
        }
        Err(e) => {
            svc.metrics.record_upstream_outcome(registry_name, false);
            crate::services::proxy::record_upstream_duration(
                &registry_label,
                "fetch_artifact",
                upstream_start,
                &svc.metrics,
            );
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
    // Times the whole body transfer, not just time-to-headers — warming issues
    // real upstream load, so it must feed the same degradation signal as the
    // proxy read path instead of silently going untimed.
    fetched.stream = crate::services::proxy::time_upstream_stream(
        Arc::clone(&registry_label),
        "fetch_artifact",
        upstream_start,
        Arc::clone(&svc.metrics),
        fetched.stream,
    );

    // Stream directly to storage without buffering the full artifact in memory.
    // `store_streaming` implementations (filesystem, S3) write incrementally so
    // peak memory is bounded to a single chunk regardless of artifact size.
    let outcome = match svc
        .storage
        .store_streaming(artifact_key, fetched.stream, StorageMeta::default())
        .await
    {
        Ok(o) => o,
        Err(e) => {
            tracing::warn!(error = %e, key = %artifact_key, "warming: store failed");
            return WarmingReport {
                errors: 1,
                ..Default::default()
            };
        }
    };
    let size = outcome.size;
    let checksum = outcome.content_hash;

    if let Err(e) = svc
        .artifact_meta
        .record_artifact(ArtifactMetaRecord {
            key: artifact_key,
            registry: registry_name,
            package_name: name,
            version,
            size: Some(size),
            checksum: Some(&checksum),
        })
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
            latest_n: n,
            ..self.clone()
        }
    }

    /// Warm a single package.
    ///
    /// If `package` contains an `@version` suffix (e.g. `"lodash@4.17.21"`), only
    /// that exact version is warmed regardless of `self.latest_n`. Otherwise the
    /// latest `self.latest_n` versions are fetched.
    ///
    /// Scoped npm names (`"@scope/name"` or `"@scope/name@version"`) start with a
    /// leading `@` that is part of the name, not a version separator, so it is
    /// skipped before searching for the real `@version` split point.
    pub async fn warm_package(&self, package: &str) -> WarmingReport {
        if self.concurrency == 0 {
            return WarmingReport::default();
        }

        let (name, pinned_version) = if let Some(rest) = package.strip_prefix('@') {
            match rest.find('@') {
                Some(pos) => (&package[..pos + 1], Some(package[pos + 2..].to_owned())),
                None => (package, None),
            }
        } else if let Some((n, v)) = package.split_once('@') {
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
                self.clone(),
                artifact_key,
                pkg,
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

    /// Warm a single artifact by its upstream **path**, for path-addressed
    /// registries (`deb`/`rpm`/`jetbrains`) that have no per-package version model.
    ///
    /// The path is fetched through the same synthetic `repo` coordinate the proxy
    /// read path uses (`{registry}/repo/_/{path}`), so the artifact lands in the
    /// exact cache slot a later `GET /proxy/{registry}/…/{path}` will read.
    pub async fn warm_path(&self, path: &str) -> WarmingReport {
        self.warm_all_paths(std::slice::from_ref(&path.to_owned()))
            .await
    }

    /// Warm every upstream path in `paths` concurrently (bounded by `concurrency`).
    pub async fn warm_all_paths(&self, paths: &[String]) -> WarmingReport {
        if self.concurrency == 0 {
            return WarmingReport::default();
        }

        let sem = Arc::new(Semaphore::new(self.concurrency));
        let mut handles = Vec::with_capacity(paths.len());

        for path in paths {
            let pkg =
                PackageId::new(self.registry_name.clone(), "repo", "_").with_artifact(path.clone());
            let artifact_key = format!("artifact:{}", pkg.cache_key());
            handles.push(tokio::spawn(warm_one_version(
                self.clone(),
                artifact_key,
                pkg,
                path.clone(),
                "_".to_owned(),
                Arc::clone(&sem),
            )));
        }

        let mut total = WarmingReport::default();
        for handle in handles {
            match handle.await {
                Ok(r) => total += r,
                Err(e) => {
                    tracing::warn!(error = %e, "warming: path task panicked");
                    total.errors += 1;
                }
            }
        }
        total
    }
}
