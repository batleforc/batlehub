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

/// Result of [`QuotaRepository::try_record_publish`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuotaOutcome {
    /// The publish was recorded; these are the post-publish totals.
    Recorded { bytes_used: u64, packages_used: u32 },
    /// The publish was rejected because it would exceed `max_bytes` and/or
    /// `max_packages`. Usage was left unchanged; these are the totals the
    /// publish would have produced.
    Exceeded { bytes_used: u64, packages_used: u32 },
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

    /// Atomically check the configured limits against the *would-be* post-publish
    /// totals and, only if neither limit is exceeded, record the publish — all as
    /// a single atomic operation so concurrent callers can't both pass the check
    /// and then both record (a classic check-then-act race). `None` in either
    /// limit means "no limit configured for that dimension".
    async fn try_record_publish(
        &self,
        user_id: &str,
        registry: &str,
        bytes: u64,
        max_bytes: Option<u64>,
        max_packages: Option<u32>,
    ) -> Result<QuotaOutcome, CoreError>;

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
