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
