//! In-process, in-memory IP block store.

use std::collections::HashMap;

use async_trait::async_trait;
use tokio::sync::Mutex;

use batlehub_core::error::CoreError;
use batlehub_core::ports::{BlockedIpInfo, IpBlockStore};

use super::now_unix;

struct ViolationWindow {
    window_start: u64,
    count: u64,
}

struct BlockEntry {
    blocked_at: u64,
    unblock_at: u64,
    reason: String,
}

/// In-process IP block store backed by `Mutex<HashMap>` entries.
///
/// State is **not** persisted across restarts. Use `PgIpBlockStore` or
/// `RedisIpBlockStore` for multi-instance deployments.
#[derive(Default)]
pub struct InMemoryIpBlockStore {
    violations: Mutex<HashMap<String, ViolationWindow>>,
    blocks: Mutex<HashMap<String, BlockEntry>>,
}

impl InMemoryIpBlockStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl IpBlockStore for InMemoryIpBlockStore {
    async fn record_violation(&self, ip: &str, window_secs: u32) -> Result<(u64, u64), CoreError> {
        if window_secs == 0 {
            return Err(CoreError::Cache("window_secs must be > 0".into()));
        }
        let now = now_unix();
        let ws = window_secs as u64;
        let window_start = (now / ws) * ws;
        let window_reset = window_start + ws;

        let mut guard = self.violations.lock().await;

        // Opportunistic eviction: when the map exceeds 10 000 entries, drop all
        // entries from windows older than 2 periods to bound memory growth.
        if guard.len() > 10_000 {
            let cutoff = window_start.saturating_sub(ws * 2);
            guard.retain(|_, e| e.window_start >= cutoff);
        }

        let entry = guard.entry(ip.to_owned()).or_insert(ViolationWindow {
            window_start,
            count: 0,
        });
        if entry.window_start != window_start {
            entry.window_start = window_start;
            entry.count = 0;
        }
        entry.count += 1;
        Ok((entry.count, window_reset))
    }

    async fn blocked_until(&self, ip: &str) -> Result<Option<u64>, CoreError> {
        let now = now_unix();
        let guard = self.blocks.lock().await;
        Ok(guard.get(ip).and_then(|e| {
            if e.unblock_at > now {
                Some(e.unblock_at)
            } else {
                None
            }
        }))
    }

    async fn block_ip(&self, ip: &str, unblock_at: u64, reason: &str) -> Result<(), CoreError> {
        let now = now_unix();
        let mut guard = self.blocks.lock().await;
        guard.insert(
            ip.to_owned(),
            BlockEntry {
                blocked_at: now,
                unblock_at,
                reason: reason.to_owned(),
            },
        );
        Ok(())
    }

    async fn unblock_ip(&self, ip: &str) -> Result<(), CoreError> {
        let mut guard = self.blocks.lock().await;
        guard.remove(ip);
        Ok(())
    }

    async fn list_blocked(&self) -> Result<Vec<BlockedIpInfo>, CoreError> {
        let now = now_unix();
        let mut guard = self.blocks.lock().await;
        // Evict expired entries eagerly on every list call.
        guard.retain(|_, e| e.unblock_at > now);
        Ok(guard
            .iter()
            .map(|(ip, e)| BlockedIpInfo {
                ip: ip.clone(),
                blocked_at: e.blocked_at,
                unblock_at: e.unblock_at,
                reason: e.reason.clone(),
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> u64 {
        now_unix()
    }

    #[tokio::test]
    async fn violation_counter_increments() {
        let store = InMemoryIpBlockStore::new();
        let (c1, _) = store.record_violation("1.2.3.4", 60).await.unwrap();
        let (c2, _) = store.record_violation("1.2.3.4", 60).await.unwrap();
        assert_eq!(c1, 1);
        assert_eq!(c2, 2);
    }

    #[tokio::test]
    async fn violation_counters_are_per_ip() {
        let store = InMemoryIpBlockStore::new();
        let (a, _) = store.record_violation("1.1.1.1", 60).await.unwrap();
        let (b, _) = store.record_violation("2.2.2.2", 60).await.unwrap();
        assert_eq!(a, 1);
        assert_eq!(b, 1);
    }

    #[tokio::test]
    async fn not_blocked_initially() {
        let store = InMemoryIpBlockStore::new();
        assert!(store.blocked_until("1.2.3.4").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn block_and_detect() {
        let store = InMemoryIpBlockStore::new();
        let unblock_at = now() + 3600;
        store.block_ip("1.2.3.4", unblock_at, "test").await.unwrap();
        let result = store.blocked_until("1.2.3.4").await.unwrap();
        assert_eq!(result, Some(unblock_at));
    }

    #[tokio::test]
    async fn expired_block_returns_none() {
        let store = InMemoryIpBlockStore::new();
        // unblock_at in the past
        store
            .block_ip("1.2.3.4", now().saturating_sub(1), "old")
            .await
            .unwrap();
        assert!(store.blocked_until("1.2.3.4").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn unblock_removes_block() {
        let store = InMemoryIpBlockStore::new();
        store
            .block_ip("1.2.3.4", now() + 3600, "test")
            .await
            .unwrap();
        store.unblock_ip("1.2.3.4").await.unwrap();
        assert!(store.blocked_until("1.2.3.4").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn list_blocked_shows_active_only() {
        let store = InMemoryIpBlockStore::new();
        store
            .block_ip("1.1.1.1", now() + 3600, "active")
            .await
            .unwrap();
        store
            .block_ip("2.2.2.2", now().saturating_sub(1), "expired")
            .await
            .unwrap();
        let list = store.list_blocked().await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].ip, "1.1.1.1");
    }

    #[tokio::test]
    async fn list_blocked_includes_reason_and_timestamps() {
        let store = InMemoryIpBlockStore::new();
        let unblock_at = now() + 100;
        store
            .block_ip("9.9.9.9", unblock_at, "manual")
            .await
            .unwrap();
        let list = store.list_blocked().await.unwrap();
        assert_eq!(list[0].reason, "manual");
        assert_eq!(list[0].unblock_at, unblock_at);
        assert!(list[0].blocked_at <= now());
    }

    #[tokio::test]
    async fn window_secs_zero_returns_error() {
        let store = InMemoryIpBlockStore::new();
        assert!(store.record_violation("1.2.3.4", 0).await.is_err());
    }
}
