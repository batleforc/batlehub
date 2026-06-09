mod report;
pub use report::{CoherenceReport, EvictionReport};

use std::sync::Arc;

use chrono::{Duration, Utc};

use crate::error::CoreError;
use crate::ports::{ArtifactMetaRepository, StorageBackend};

/// Configuration for the eviction service. All fields are optional; omitting a
/// field disables that eviction strategy.
#[derive(Debug, Clone, Default)]
pub struct EvictionConfig {
    /// Evict artifacts whose `cached_at` is older than this many seconds.
    pub artifact_ttl_secs: Option<u64>,
    /// Evict artifacts not accessed for this many days.
    pub idle_days: Option<u64>,
    /// When total storage for a registry exceeds this byte count, evict the
    /// least-recently-used artifacts until usage falls below the threshold.
    pub max_size_bytes: Option<u64>,
    /// Keep only the N most-recently-cached versions per (registry, package).
    pub keep_latest_n: Option<usize>,
    /// Registry name to scope eviction to. Pass `""` to run across all registries.
    pub registry: String,
}

/// Drives artifact eviction across storage and artifact-meta.
pub struct EvictionService {
    pub artifact_meta: Arc<dyn ArtifactMetaRepository>,
    pub storage: Arc<dyn StorageBackend>,
    pub config: EvictionConfig,
}

impl EvictionService {
    pub fn new(
        artifact_meta: Arc<dyn ArtifactMetaRepository>,
        storage: Arc<dyn StorageBackend>,
        config: EvictionConfig,
    ) -> Self {
        Self {
            artifact_meta,
            storage,
            config,
        }
    }

    /// Run all configured eviction strategies in sequence.
    pub async fn run_all(&self) -> Result<EvictionReport, CoreError> {
        let mut report = EvictionReport::default();

        if self.config.artifact_ttl_secs.is_some() {
            let n = self.run_ttl().await?;
            report.evicted_ttl = n;
        }
        if self.config.idle_days.is_some() {
            let n = self.run_idle().await?;
            report.evicted_idle = n;
        }
        if self.config.keep_latest_n.is_some() {
            let n = self.run_keep_latest_n().await?;
            report.evicted_old_versions = n;
        }
        if self.config.max_size_bytes.is_some() {
            let n = self.run_lru_size_cap().await?;
            report.evicted_lru = n;
        }

        report.total = report.evicted_ttl
            + report.evicted_idle
            + report.evicted_old_versions
            + report.evicted_lru;
        Ok(report)
    }

    /// Evict artifacts whose `cached_at` is older than `artifact_ttl_secs`.
    pub async fn run_ttl(&self) -> Result<usize, CoreError> {
        let ttl_secs = match self.config.artifact_ttl_secs {
            Some(s) => s,
            None => return Ok(0),
        };
        let cutoff = Utc::now() - Duration::seconds(ttl_secs as i64);
        let expired = self
            .artifact_meta
            .list_expired_by_ttl(&self.config.registry, cutoff)
            .await?;
        let mut count = 0;
        for meta in expired {
            if let Err(e) = self.storage.delete(&meta.artifact_key).await {
                tracing::warn!(key = %meta.artifact_key, error = %e, "eviction(ttl): storage delete failed");
                continue;
            }
            if let Err(e) = self
                .artifact_meta
                .delete_artifact_meta(&meta.artifact_key)
                .await
            {
                tracing::warn!(key = %meta.artifact_key, error = %e, "eviction(ttl): meta delete failed");
            }
            count += 1;
        }
        if count > 0 {
            tracing::info!(count, registry = %self.config.registry, "eviction(ttl): evicted artifacts");
        }
        Ok(count)
    }

    /// Evict artifacts not accessed for `idle_days` days.
    pub async fn run_idle(&self) -> Result<usize, CoreError> {
        let days = match self.config.idle_days {
            Some(d) => d,
            None => return Ok(0),
        };
        let cutoff = Utc::now() - Duration::days(days as i64);
        let idle = self
            .artifact_meta
            .list_idle(&self.config.registry, cutoff)
            .await?;
        let mut count = 0;
        for meta in idle {
            if let Err(e) = self.storage.delete(&meta.artifact_key).await {
                tracing::warn!(key = %meta.artifact_key, error = %e, "eviction(idle): storage delete failed");
                continue;
            }
            if let Err(e) = self
                .artifact_meta
                .delete_artifact_meta(&meta.artifact_key)
                .await
            {
                tracing::warn!(key = %meta.artifact_key, error = %e, "eviction(idle): meta delete failed");
            }
            count += 1;
        }
        if count > 0 {
            tracing::info!(count, registry = %self.config.registry, "eviction(idle): evicted artifacts");
        }
        Ok(count)
    }

    /// For each (registry, package), keep only the N most-recently-cached versions;
    /// evict the rest.
    pub async fn run_keep_latest_n(&self) -> Result<usize, CoreError> {
        let n = match self.config.keep_latest_n {
            Some(n) if n > 0 => n,
            _ => return Ok(0),
        };

        let all = self.artifact_meta.list_artifacts_by_package().await?;

        // list_artifacts_by_package returns rows ordered by (registry, package_name, cached_at DESC)
        // Group and pick the tail beyond the first N per group.
        let mut count = 0;
        let mut current_group: Option<(String, String)> = None;
        let mut group_pos: usize = 0;

        for meta in all {
            let group = (meta.registry.clone(), meta.package_name.clone());
            if current_group.as_ref() != Some(&group) {
                current_group = Some(group);
                group_pos = 0;
            }
            group_pos += 1;
            if group_pos <= n {
                continue; // within keep window
            }
            if let Err(e) = self.storage.delete(&meta.artifact_key).await {
                tracing::warn!(key = %meta.artifact_key, error = %e, "eviction(keep_latest_n): storage delete failed");
                continue;
            }
            if let Err(e) = self
                .artifact_meta
                .delete_artifact_meta(&meta.artifact_key)
                .await
            {
                tracing::warn!(key = %meta.artifact_key, error = %e, "eviction(keep_latest_n): meta delete failed");
            }
            count += 1;
        }
        if count > 0 {
            tracing::info!(count, registry = %self.config.registry, "eviction(keep_latest_n): evicted old versions");
        }
        Ok(count)
    }

    /// Evict one batch of LRU candidates. Returns `(evicted_count, new_total)`.
    async fn evict_lru_batch(
        &self,
        candidates: Vec<crate::ports::ArtifactMeta>,
        mut total: u64,
        cap: u64,
    ) -> (usize, u64) {
        let mut count = 0;
        for meta in candidates {
            if total <= cap {
                break;
            }
            let size = meta.size_bytes.unwrap_or(0);
            if let Err(e) = self.storage.delete(&meta.artifact_key).await {
                tracing::warn!(key = %meta.artifact_key, error = %e, "eviction(lru): storage delete failed");
                continue;
            }
            if let Err(e) = self
                .artifact_meta
                .delete_artifact_meta(&meta.artifact_key)
                .await
            {
                tracing::warn!(key = %meta.artifact_key, error = %e, "eviction(lru): meta delete failed");
            }
            total = total.saturating_sub(size);
            count += 1;
        }
        (count, total)
    }

    /// Evict the LRU artifacts until total storage for the registry is under `max_size_bytes`.
    pub async fn run_lru_size_cap(&self) -> Result<usize, CoreError> {
        let cap = match self.config.max_size_bytes {
            Some(c) => c,
            None => return Ok(0),
        };
        let mut total = self
            .artifact_meta
            .total_size_bytes(&self.config.registry)
            .await?;
        if total <= cap {
            return Ok(0);
        }

        let mut count = 0;
        // Fetch up to 256 LRU candidates at a time to avoid huge result sets.
        loop {
            if total.saturating_sub(cap) == 0 {
                break;
            }
            let candidates = self
                .artifact_meta
                .list_lru(&self.config.registry, 256)
                .await?;
            if candidates.is_empty() {
                break;
            }
            let (batch, new_total) = self.evict_lru_batch(candidates, total, cap).await;
            count += batch;
            total = new_total;
            // If we didn't reduce below cap and ran out of candidates, stop.
            if total > cap {
                break;
            }
        }
        if count > 0 {
            tracing::info!(count, registry = %self.config.registry, "eviction(lru): evicted artifacts");
        }
        Ok(count)
    }

    /// Compare artifact keys in storage against the artifact_meta table. Delete
    /// storage entries that have no corresponding meta row (orphaned blobs from
    /// crashed writes or manual deletions from the DB).
    pub async fn run_coherence_check(&self) -> Result<CoherenceReport, CoreError> {
        // Artifact keys are stored as "artifact:{registry}/{name}:{version}".
        // We need the prefix that matches all artifact keys for this registry.
        let key_prefix = if self.config.registry.is_empty() {
            "artifact:".to_owned()
        } else {
            format!("artifact:{}/", self.config.registry)
        };
        let storage_keys = self.storage.list_keys(&key_prefix).await?;
        let meta_rows = self
            .artifact_meta
            .list_artifacts(&self.config.registry)
            .await?;
        let meta_keys: std::collections::HashSet<String> =
            meta_rows.into_iter().map(|m| m.artifact_key).collect();

        let mut orphaned = 0usize;
        for key in &storage_keys {
            if !meta_keys.contains(key) {
                tracing::warn!(key, "coherence: orphaned storage object, deleting");
                if let Err(e) = self.storage.delete(key).await {
                    tracing::warn!(key, error = %e, "coherence: failed to delete orphaned object");
                } else {
                    orphaned += 1;
                }
            }
        }

        Ok(CoherenceReport {
            storage_keys: storage_keys.len(),
            meta_rows: meta_keys.len(),
            orphaned_deleted: orphaned,
        })
    }
}

#[cfg(test)]
mod tests;
