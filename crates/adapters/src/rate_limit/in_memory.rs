//! In-process, in-memory rate-limit counter store.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;

use batlehub_core::error::CoreError;
use batlehub_core::ports::RateLimitStore;

struct WindowEntry {
    window_start: u64,
    count: u64,
}

/// In-process rate-limit counter store backed by a `Mutex<HashMap>`.
///
/// State is **not** persisted across restarts and **not** shared across multiple server
/// instances. Suitable for development, single-node deployments, or unit tests.
/// Use [`PgRateLimitStore`] or [`RedisRateLimitStore`] for multi-instance deployments.
#[derive(Default)]
pub struct InMemoryRateLimitStore {
    inner: Mutex<HashMap<String, WindowEntry>>,
}

impl InMemoryRateLimitStore {
    /// Create a new, empty store.
    pub fn new() -> Self {
        Self::default()
    }

    fn now_unix() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }
}

#[async_trait]
impl RateLimitStore for InMemoryRateLimitStore {
    async fn increment(&self, key: &str, window_secs: u32) -> Result<(u64, u64), CoreError> {
        if window_secs == 0 {
            return Err(CoreError::Cache("window_secs must be > 0".into()));
        }
        let now = Self::now_unix();
        let ws = window_secs as u64;
        let window_start = (now / ws) * ws;
        let window_reset = window_start + ws;

        // Include window_secs in the map key so two callers using the same logical
        // key with different window sizes get independent counters.
        let map_key = format!("{key}@{window_secs}");
        let mut map = self
            .inner
            .lock()
            .map_err(|_| CoreError::Cache("rate_limit lock poisoned".into()))?;
        let entry = map.entry(map_key).or_insert(WindowEntry {
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
}

#[cfg(test)]
impl InMemoryRateLimitStore {
    /// Back-date an entry so the next increment sees it as a stale window.
    fn force_old_window(&self, key: &str, window_secs: u32, old_start: u64, count: u64) {
        let map_key = format!("{key}@{window_secs}");
        let mut map = self.inner.lock().unwrap();
        map.insert(
            map_key,
            WindowEntry {
                window_start: old_start,
                count,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[tokio::test]
    async fn increments_within_window() {
        let store = InMemoryRateLimitStore::new();
        let (c1, _) = store.increment("k", 60).await.unwrap();
        let (c2, _) = store.increment("k", 60).await.unwrap();
        assert_eq!(c1, 1);
        assert_eq!(c2, 2);
    }

    #[tokio::test]
    async fn independent_keys() {
        let store = InMemoryRateLimitStore::new();
        let (a, _) = store.increment("a", 60).await.unwrap();
        let (b, _) = store.increment("b", 60).await.unwrap();
        assert_eq!(a, 1);
        assert_eq!(b, 1);
    }

    #[tokio::test]
    async fn many_increments_accumulate() {
        let store = InMemoryRateLimitStore::new();
        for expected in 1u64..=50 {
            let (count, _) = store.increment("k", 3600).await.unwrap();
            assert_eq!(count, expected);
        }
    }

    #[tokio::test]
    async fn reset_timestamp_is_in_the_future() {
        let store = InMemoryRateLimitStore::new();
        let (_, reset) = store.increment("k", 60).await.unwrap();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        assert!(reset > now, "reset_unix {reset} should be after now {now}");
    }

    #[tokio::test]
    async fn reset_timestamp_does_not_exceed_one_window_from_now() {
        let store = InMemoryRateLimitStore::new();
        let window_secs: u64 = 60;
        let (_, reset) = store.increment("k", window_secs as u32).await.unwrap();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        assert!(
            reset <= now + window_secs,
            "reset_unix {reset} should not be more than {window_secs}s from now {now}"
        );
    }

    #[tokio::test]
    async fn window_rollover_resets_counter_to_one() {
        let store = InMemoryRateLimitStore::new();
        // Seed the store with a counter from the Unix epoch (definitely a past window).
        store.force_old_window("k", 60, 0, 999);
        let (count, reset) = store.increment("k", 60).await.unwrap();
        assert_eq!(
            count, 1,
            "stale window should be discarded and counter reset to 1"
        );
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        assert!(
            reset > now,
            "reset after rollover should still be in the future"
        );
    }

    #[tokio::test]
    async fn same_key_different_window_sizes_are_independent() {
        // window_secs is encoded in the map key, so the two buckets are fully independent.
        let store = InMemoryRateLimitStore::new();
        let (c60, _) = store.increment("k", 60).await.unwrap();
        let (c3600, _) = store.increment("k", 3600).await.unwrap();
        assert_eq!(c60, 1);
        assert_eq!(
            c3600, 1,
            "different window_secs must give independent counters"
        );
    }

    #[tokio::test]
    async fn concurrent_increments_all_succeed() {
        let store = Arc::new(InMemoryRateLimitStore::new());
        let handles: Vec<_> = (0..20)
            .map(|_| {
                let s = store.clone();
                tokio::spawn(async move { s.increment("concurrent", 60).await.unwrap() })
            })
            .collect();
        let results: Vec<(u64, u64)> = futures::future::join_all(handles)
            .await
            .into_iter()
            .map(|r| r.unwrap())
            .collect();
        let counts: std::collections::HashSet<u64> = results.iter().map(|(c, _)| *c).collect();
        assert_eq!(
            counts.len(),
            20,
            "concurrent increments must produce 20 unique counts 1..=20"
        );
        assert_eq!(*counts.iter().max().unwrap(), 20);
    }
}
