#![cfg(feature = "cache-redis")]
//! Integration tests for `RedisRateLimitStore`.
//!
//! Requires a running Redis instance. Set `REDIS_URL` to opt in:
//!
//!   REDIS_URL=redis://localhost:6379 \
//!     cargo test -p batlehub-adapters --test redis_rate_limit --features cache-redis
//!
//! Tests are skipped (not failed) when `REDIS_URL` is unset.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use redis::AsyncCommands as _;
use redis::aio::ConnectionManager;

use batlehub_adapters::rate_limit::RedisRateLimitStore;
use batlehub_core::ports::RateLimitStore;

// ── Helpers ───────────────────────────────────────────────────────────────────

fn redis_url() -> Option<String> {
    std::env::var("REDIS_URL").ok()
}

static TEST_ID: AtomicU64 = AtomicU64::new(0);

struct TestStore {
    store: RedisRateLimitStore,
    conn: ConnectionManager,
    prefix: String,
}

impl TestStore {
    fn key(&self, name: &str) -> String {
        format!("redis-rl-test-{}:{}", self.prefix, name)
    }

    fn redis_key_for_window(&self, logical_key: &str, window_secs: u32) -> String {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        let ws = window_secs as u64;
        let window_start = (now / ws) * ws;
        format!("batlehub:rl:{}:{}", self.key(logical_key), window_start)
    }

    async fn ttl(&mut self, redis_key: &str) -> i64 {
        redis::cmd("TTL").arg(redis_key).query_async(&mut self.conn).await.unwrap()
    }

    async fn raw_count(&mut self, redis_key: &str) -> Option<u64> {
        self.conn.get(redis_key).await.unwrap_or(None)
    }
}

async fn make_store(url: &str) -> TestStore {
    let id = TEST_ID.fetch_add(1, Ordering::Relaxed);
    let prefix = format!("{id}");
    let store = RedisRateLimitStore::new(url).await.expect("connect to Redis");
    let client = redis::Client::open(url).expect("open Redis client");
    let conn = ConnectionManager::new(client).await.expect("connection manager");
    TestStore { store, conn, prefix }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn increment_starts_at_one() {
    let Some(url) = redis_url() else { return };
    let s = make_store(&url).await;
    let (count, _) = s.store.increment(&s.key("k"), 60).await.unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn repeated_increments_accumulate() {
    let Some(url) = redis_url() else { return };
    let s = make_store(&url).await;
    let key = s.key("k");
    for expected in 1u64..=10 {
        let (count, _) = s.store.increment(&key, 60).await.unwrap();
        assert_eq!(count, expected);
    }
}

#[tokio::test]
async fn independent_keys_do_not_interfere() {
    let Some(url) = redis_url() else { return };
    let s = make_store(&url).await;
    let (a, _) = s.store.increment(&s.key("a"), 60).await.unwrap();
    let (b, _) = s.store.increment(&s.key("b"), 60).await.unwrap();
    let (a2, _) = s.store.increment(&s.key("a"), 60).await.unwrap();
    assert_eq!(a, 1);
    assert_eq!(b, 1);
    assert_eq!(a2, 2);
}

#[tokio::test]
async fn ttl_is_set_on_first_write() {
    let Some(url) = redis_url() else { return };
    let mut s = make_store(&url).await;
    let rk = s.redis_key_for_window("k", 60);
    s.store.increment(&s.key("k"), 60).await.unwrap();
    let ttl = s.ttl(&rk).await;
    assert!(ttl > 0, "TTL should be positive after first write, got {ttl}");
    assert!(ttl <= 61, "TTL should not exceed window_secs+1 (61), got {ttl}");
}

#[tokio::test]
async fn ttl_is_not_refreshed_on_subsequent_writes() {
    let Some(url) = redis_url() else { return };
    let mut s = make_store(&url).await;
    let key = s.key("k");
    let rk = s.redis_key_for_window("k", 60);

    s.store.increment(&key, 60).await.unwrap();
    let ttl_after_first = s.ttl(&rk).await;

    // A second increment must not call EXPIRE again, so the TTL can only stay
    // the same or decrease — never increase.
    s.store.increment(&key, 60).await.unwrap();
    let ttl_after_second = s.ttl(&rk).await;

    assert!(
        ttl_after_second <= ttl_after_first,
        "TTL should not increase on subsequent writes: first={ttl_after_first} second={ttl_after_second}"
    );
    assert!(ttl_after_second > 0, "TTL should still be alive after second write");
}

#[tokio::test]
async fn reset_unix_is_within_one_window_of_now() {
    let Some(url) = redis_url() else { return };
    let s = make_store(&url).await;
    let window_secs: u64 = 60;
    let (_, reset) = s.store.increment(&s.key("k"), window_secs as u32).await.unwrap();
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
    assert!(reset > now, "reset {reset} must be after now {now}");
    assert!(reset <= now + window_secs, "reset {reset} must be ≤ {}", now + window_secs);
}

#[tokio::test]
async fn concurrent_increments_are_atomic() {
    let Some(url) = redis_url() else { return };
    let s = make_store(&url).await;
    let key = s.key("concurrent");
    let store = Arc::new(s.store);

    let handles: Vec<_> = (0..20)
        .map(|_| {
            let store = store.clone();
            let key = key.clone();
            tokio::spawn(async move { store.increment(&key, 60).await.unwrap() })
        })
        .collect();

    let results: Vec<(u64, u64)> = futures::future::join_all(handles)
        .await
        .into_iter()
        .map(|r| r.unwrap())
        .collect();

    let counts: std::collections::HashSet<u64> = results.iter().map(|(c, _)| *c).collect();
    assert_eq!(counts.len(), 20, "concurrent Redis INCRs must be atomic and unique; got: {counts:?}");
    assert_eq!(*counts.iter().max().unwrap(), 20);
}

#[tokio::test]
async fn raw_redis_key_matches_expected_pattern() {
    let Some(url) = redis_url() else { return };
    let mut s = make_store(&url).await;
    let logical = s.key("raw");
    let expected_rk = s.redis_key_for_window("raw", 60);

    s.store.increment(&logical, 60).await.unwrap();

    let count = s.raw_count(&expected_rk).await;
    assert_eq!(count, Some(1), "raw Redis key {expected_rk} should hold count 1");
}
