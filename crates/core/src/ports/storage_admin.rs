use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::error::CoreError;

/// A row from the `artifact_storage` tracking table: which backend holds a
/// given cached artifact key, and when it was first stored.
pub struct ArtifactStorageRecord {
    pub backend_name: String,
    pub stored_at: DateTime<Utc>,
}

/// Admin-facing port over the `artifact_storage` tracking table — used by the
/// back-office cache-clear and package-detail endpoints.
///
/// Distinct from the adapter-internal lookups in
/// `crates/adapters/src/storage/router/tracking.rs` (`StorageRouter`'s
/// per-request backend resolution): this port is for admin/API read and
/// bulk-delete access, not the hot request path.
#[async_trait]
pub trait StorageAdminRepository: Send + Sync {
    /// Look up the recorded backend + stored_at for a single storage key.
    async fn find_by_key(
        &self,
        storage_key: &str,
    ) -> Result<Option<ArtifactStorageRecord>, CoreError>;

    /// Delete all `artifact_storage` rows whose storage_key starts with `prefix`.
    /// Returns the number of rows deleted.
    async fn delete_by_prefix(&self, prefix: &str) -> Result<u64, CoreError>;
}
