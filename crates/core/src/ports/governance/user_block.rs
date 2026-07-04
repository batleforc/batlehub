use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::error::CoreError;

/// A blocked user entry.
pub struct UserBlock {
    pub user_id: String,
    pub blocked_at: DateTime<Utc>,
    pub blocked_by: String,
    pub reason: Option<String>,
}

/// Port for tracking manually blocked user accounts.
///
/// Blocks are permanent (no expiry) — unlike IP blocks which use a timed window.
/// Use `unblock` to lift a block.
#[async_trait]
pub trait UserBlockRepository: Send + Sync {
    /// List all currently blocked users.
    async fn list(&self) -> Result<Vec<UserBlock>, CoreError>;

    /// Block a user. Idempotent — re-blocking an already-blocked user updates the entry.
    async fn block(
        &self,
        user_id: &str,
        blocked_by: &str,
        reason: Option<&str>,
    ) -> Result<(), CoreError>;

    /// Lift the block on a user. No-op if the user is not blocked.
    async fn unblock(&self, user_id: &str) -> Result<(), CoreError>;

    /// Returns `true` if the user is currently blocked.
    async fn is_blocked(&self, user_id: &str) -> Result<bool, CoreError>;
}
