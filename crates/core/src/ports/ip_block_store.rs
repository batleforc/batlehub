use async_trait::async_trait;

use crate::error::CoreError;

/// Information about a currently blocked IP address.
pub struct BlockedIpInfo {
    pub ip: String,
    pub blocked_at: u64, // Unix seconds
    pub unblock_at: u64, // Unix seconds
    pub reason: String,
}

/// Port for tracking per-IP violation counts and active blocks (fail2ban pattern).
#[async_trait]
pub trait IpBlockStore: Send + Sync {
    /// Record one violation for `ip` within a sliding fixed window of `window_secs`.
    /// Returns `(violation_count_in_window, window_reset_unix_secs)`.
    async fn record_violation(&self, ip: &str, window_secs: u32) -> Result<(u64, u64), CoreError>;

    /// Returns `Some(unblock_at_unix)` if the IP is currently blocked, `None` if allowed.
    async fn is_blocked(&self, ip: &str) -> Result<Option<u64>, CoreError>;

    /// Block `ip` until `unblock_at` (Unix seconds). `reason` is stored for auditing.
    async fn block_ip(&self, ip: &str, unblock_at: u64, reason: &str) -> Result<(), CoreError>;

    /// Remove `ip` from the block list immediately.
    async fn unblock_ip(&self, ip: &str) -> Result<(), CoreError>;

    /// List all currently active blocks (expired blocks may be excluded).
    async fn list_blocked(&self) -> Result<Vec<BlockedIpInfo>, CoreError>;
}
