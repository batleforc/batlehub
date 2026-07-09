use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Compute the aligned window `(start, reset)` for a given timestamp.
///
/// `window_start = floor(now / window_secs) * window_secs`
/// `window_reset = window_start + window_secs`
pub(crate) fn violation_window(now: u64, window_secs: u32) -> (u64, u64) {
    let ws = window_secs as u64;
    let window_start = (now / ws) * ws;
    (window_start, window_start + ws)
}

/// A single window-counter entry, shared by the two in-memory stores
/// (`InMemoryRateLimitStore`, `InMemoryIpBlockStore`) that each track a
/// per-key request count within a rolling time window.
pub(crate) struct WindowEntry {
    pub(crate) window_start: u64,
    pub(crate) count: u64,
}

/// Once an in-memory window-counter map exceeds this many entries, opportunistically
/// drop everything from windows older than 2 periods to bound memory growth.
const EVICTION_THRESHOLD: usize = 10_000;

/// Increment `key`'s counter in `map` for the current window, resetting it to 1
/// if the previous entry belongs to a stale window. Shared by
/// `InMemoryRateLimitStore::increment` and `InMemoryIpBlockStore::record_violation`,
/// which differ only in what they call the counted thing (rate-limit key vs. IP).
///
/// Callers must reject `window_secs == 0` themselves before calling this —
/// [`violation_window`] divides by it.
pub(crate) fn windowed_increment(
    map: &mut HashMap<String, WindowEntry>,
    key: String,
    window_secs: u32,
) -> (u64, u64) {
    let (window_start, window_reset) = violation_window(now_unix(), window_secs);

    if map.len() > EVICTION_THRESHOLD {
        let cutoff = window_start.saturating_sub((window_secs as u64) * 2);
        map.retain(|_, e| e.window_start >= cutoff);
    }

    let entry = map.entry(key).or_insert(WindowEntry {
        window_start,
        count: 0,
    });
    if entry.window_start != window_start {
        entry.window_start = window_start;
        entry.count = 0;
    }
    entry.count += 1;
    (entry.count, window_reset)
}

#[cfg(test)]
mod tests {
    use super::{now_unix, violation_window};

    #[test]
    fn now_unix_is_after_2024() {
        assert!(now_unix() > 1_704_067_200);
    }

    #[test]
    fn violation_window_aligns_to_boundary() {
        let (start, reset) = violation_window(1000, 60);
        assert_eq!(start, 960);
        assert_eq!(reset, 1020);
    }

    #[test]
    fn violation_window_exactly_on_boundary() {
        let (start, reset) = violation_window(960, 60);
        assert_eq!(start, 960);
        assert_eq!(reset, 1020);
    }

    #[test]
    fn violation_window_start_is_always_lte_now() {
        for now in [0u64, 1, 59, 60, 61, 1000, 9999] {
            for ws in [1u32, 30, 60, 300] {
                let (start, reset) = violation_window(now, ws);
                assert!(start <= now, "start={start} > now={now} for ws={ws}");
                assert!(reset > now, "reset={reset} <= now={now} for ws={ws}");
            }
        }
    }
}

pub mod in_memory;
pub use in_memory::InMemoryRateLimitStore;

pub mod ip_block_in_memory;
pub use ip_block_in_memory::InMemoryIpBlockStore;

#[cfg(feature = "db-postgres")]
pub mod postgres;
#[cfg(feature = "db-postgres")]
pub use postgres::PgRateLimitStore;

#[cfg(feature = "db-postgres")]
pub mod ip_block_postgres;
#[cfg(feature = "db-postgres")]
pub use ip_block_postgres::PgIpBlockStore;

#[cfg(feature = "cache-redis")]
pub mod redis;
#[cfg(feature = "cache-redis")]
pub use redis::RedisRateLimitStore;

#[cfg(feature = "cache-redis")]
pub mod ip_block_redis;
#[cfg(feature = "cache-redis")]
pub use ip_block_redis::RedisIpBlockStore;
