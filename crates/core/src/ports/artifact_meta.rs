use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::error::CoreError;

/// Tracks every artifact stored in the cache: when it was first stored, when
/// it was last accessed, its size, and which registry/package/version it belongs to.
/// Used by the eviction service and cache coherence checker.
#[derive(Debug, Clone)]
pub struct ArtifactMeta {
    pub artifact_key: String,
    pub registry: String,
    pub package_name: String,
    pub version: String,
    pub size_bytes: Option<u64>,
    pub cached_at: DateTime<Utc>,
    pub last_accessed_at: DateTime<Utc>,
}

#[async_trait]
pub trait ArtifactMetaRepository: Send + Sync {
    /// Record or refresh the metadata for a newly stored artifact.
    /// Upserts: if the key already exists, updates `size_bytes` and resets `cached_at`.
    async fn record_artifact(
        &self,
        key: &str,
        registry: &str,
        package_name: &str,
        version: &str,
        size: Option<u64>,
    ) -> Result<(), CoreError>;

    /// Update `last_accessed_at` to now for an existing artifact record.
    /// No-ops gracefully if the key does not exist in the meta table.
    async fn touch_artifact(&self, key: &str) -> Result<(), CoreError>;

    /// List all artifact metadata rows for a given registry.
    /// Pass `""` to list across all registries.
    async fn list_artifacts(&self, registry: &str) -> Result<Vec<ArtifactMeta>, CoreError>;

    /// List all artifact metadata rows, grouped by (registry, package_name),
    /// sorted by `cached_at DESC` within each group.
    async fn list_artifacts_by_package(&self) -> Result<Vec<ArtifactMeta>, CoreError>;

    /// Remove the metadata record for a key (called when an artifact is evicted).
    async fn delete_artifact_meta(&self, key: &str) -> Result<(), CoreError>;

    /// Return `true` if the artifact's `cached_at` is older than `older_than`, OR if the
    /// artifact has no metadata row (unknown age → conservatively treat as expired so the
    /// caller re-fetches rather than serving a potentially stale artifact indefinitely).
    /// More efficient than `list_expired_by_ttl` for per-request TTL checks.
    async fn is_artifact_expired(
        &self,
        key: &str,
        older_than: DateTime<Utc>,
    ) -> Result<bool, CoreError>;

    /// List artifact keys whose `cached_at` is older than `older_than`.
    async fn list_expired_by_ttl(
        &self,
        registry: &str,
        older_than: DateTime<Utc>,
    ) -> Result<Vec<ArtifactMeta>, CoreError>;

    /// List artifact keys whose `last_accessed_at` is older than `idle_since`.
    async fn list_idle(
        &self,
        registry: &str,
        idle_since: DateTime<Utc>,
    ) -> Result<Vec<ArtifactMeta>, CoreError>;

    /// Return the total cached size in bytes across all artifacts for a registry.
    async fn total_size_bytes(&self, registry: &str) -> Result<u64, CoreError>;

    /// List artifacts for a registry sorted by `last_accessed_at ASC` (oldest first),
    /// up to `limit` rows. Used for LRU size-cap eviction.
    async fn list_lru(
        &self,
        registry: &str,
        limit: i64,
    ) -> Result<Vec<ArtifactMeta>, CoreError>;
}
