use async_trait::async_trait;

use crate::error::CoreError;

/// Current quota usage for one (user, registry) pair.
#[derive(Debug, Clone)]
pub struct QuotaUsage {
    pub user_id: String,
    pub registry: String,
    pub bytes_published: u64,
    pub packages_count: u32,
}

#[async_trait]
pub trait QuotaRepository: Send + Sync {
    /// Return the current quota usage for a user in a registry.
    /// Returns a zeroed `QuotaUsage` if the user has not published anything yet.
    async fn get_usage(&self, user_id: &str, registry: &str) -> Result<QuotaUsage, CoreError>;

    /// Atomically add `bytes` to the user's published-bytes counter and increment
    /// the packages count by 1.
    async fn record_publish(
        &self,
        user_id: &str,
        registry: &str,
        bytes: u64,
    ) -> Result<(), CoreError>;

    /// Subtract `bytes` from the published-bytes counter (used when a publish is
    /// rolled back or a package is deleted). The counter is floored at 0.
    async fn revoke_publish(
        &self,
        user_id: &str,
        registry: &str,
        bytes: u64,
    ) -> Result<(), CoreError>;

    /// Reset the usage counters for a specific user in a registry to zero.
    async fn reset_usage(&self, user_id: &str, registry: &str) -> Result<(), CoreError>;

    /// List usage rows, optionally filtered to a single registry.
    async fn list_usage(&self, registry: Option<&str>) -> Result<Vec<QuotaUsage>, CoreError>;
}
