use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::error::CoreError;

/// A row from the `config_changes` audit table — records each hot-reload
/// attempt (applied or failed) so operators can review what changed and when.
pub struct ConfigChangeRecord {
    pub id: Uuid,
    pub triggered_by: String,
    pub triggered_at: DateTime<Utc>,
    pub status: String,
    pub diff: serde_json::Value,
    pub summary: String,
    pub error_msg: Option<String>,
}

/// Port for persisting and querying the hot-reload audit trail
/// (`config_changes` table).
#[async_trait]
pub trait ConfigChangeRepository: Send + Sync {
    /// Insert a new audit row recording a reload attempt.
    async fn insert(&self, record: ConfigChangeRecord) -> Result<(), CoreError>;

    /// List past reload audit rows, newest first, paginated by page/per_page.
    async fn list(&self, page: u64, per_page: u64) -> Result<Vec<ConfigChangeRecord>, CoreError>;

    /// Total count of audit rows, ignoring pagination — backs `ConfigChangesResponse.total`.
    async fn count(&self) -> Result<u64, CoreError>;
}
