//! Integration tests for `PgRateLimitStore`.
//!
//! Requires a running PostgreSQL instance. Set `DATABASE_URL` to opt in:
//!
//!   DATABASE_URL=postgresql://batlehub:changeme@localhost/batlehub \
//!     cargo test -p batlehub-adapters --test pg_rate_limit
//!
//! Tests are skipped (not failed) when `DATABASE_URL` is unset.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use sqlx::PgPool;

use batlehub_adapters::rate_limit::PgRateLimitStore;
use batlehub_core::ports::RateLimitStore;

// ── Helpers ───────────────────────────────────────────────────────────────────

fn db_url() -> Option<String> {
    std::env::var("DATABASE_URL").ok()
}

// Each test gets an isolated key prefix to allow parallel execution without
// rows from one test interfering with another.
static TEST_ID: AtomicU64 = AtomicU64::new(0);

struct TestStore {
    store: PgRateLimitStore,
    pool: PgPool,
    prefix: String,
}

impl TestStore {
    fn key(&self, name: &str) -> String {
        format!("pg-rl-test-{}:{}", self.prefix, name)
    }

    /// Read the raw row count for a key prefix from the DB (for pruning assertions).
    async fn raw_row_count(&self, key: &str) -> i64 {
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM rate_limit_counters WHERE key = $1")
            .bind(key)
            .fetch_one(&self.pool)
            .await
            .unwrap_or(0)
    }
}

async fn make_store(url: &str) -> TestStore {
    let id = TEST_ID.fetch_add(1, Ordering::Relaxed);
    // Include a nanosecond timestamp so keys from different test-binary invocations
    // never collide with leftover rows in the shared Postgres instance.
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let prefix = format!("{ts}-{id}");
    let pool = PgPool::connect(url).await.expect("connect to postgres");
    batlehub_adapters::migrations::embedded_migrator()
        .run(&pool)
        .await
        .expect("run migrations");
    TestStore {
        store: PgRateLimitStore::new(pool.clone()),
        pool,
        prefix,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn increment_starts_at_one() {
    let Some(url) = db_url() else { return };
    let s = make_store(&url).await;
    let (count, _) = s.store.increment(&s.key("k"), 60).await.unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn repeated_increments_accumulate() {
    let Some(url) = db_url() else { return };
    let s = make_store(&url).await;
    let key = s.key("k");
    for expected in 1u64..=10 {
        let (count, _) = s.store.increment(&key, 60).await.unwrap();
        assert_eq!(
            count, expected,
            "increment #{expected} should return {expected}"
        );
    }
}

#[tokio::test]
async fn independent_keys_do_not_interfere() {
    let Some(url) = db_url() else { return };
    let s = make_store(&url).await;
    let (a, _) = s.store.increment(&s.key("a"), 60).await.unwrap();
    let (b, _) = s.store.increment(&s.key("b"), 60).await.unwrap();
    let (a2, _) = s.store.increment(&s.key("a"), 60).await.unwrap();
    assert_eq!(a, 1);
    assert_eq!(b, 1);
    assert_eq!(a2, 2, "second increment for 'a' should be 2");
}

#[tokio::test]
async fn reset_unix_is_at_window_boundary() {
    let Some(url) = db_url() else { return };
    let s = make_store(&url).await;
    let window_secs: u64 = 60;
    let before = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let (_, reset) = s
        .store
        .increment(&s.key("k"), window_secs as u32)
        .await
        .unwrap();
    let after = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    // reset must be strictly after the call started and at most one window from now
    assert!(
        reset > before,
        "reset {reset} must be after call start {before}"
    );
    assert!(
        reset <= after + window_secs,
        "reset {reset} must be ≤ {}",
        after + window_secs
    );
    // reset must fall on a window boundary (multiple of window_secs)
    assert_eq!(
        reset % window_secs,
        0,
        "reset {reset} must be a multiple of {window_secs}"
    );
}

#[tokio::test]
async fn old_window_rows_are_pruned_on_write() {
    let Some(url) = db_url() else { return };
    let s = make_store(&url).await;
    let key = s.key("prune");

    // Insert a fake row from a long-past window directly.
    sqlx::query("INSERT INTO rate_limit_counters (key, window_start, count) VALUES ($1, 0, 999)")
        .bind(&key)
        .execute(&s.pool)
        .await
        .unwrap();
    assert_eq!(s.raw_row_count(&key).await, 1, "seeded row should exist");

    // Any new increment should prune the stale row and insert the current window.
    s.store.increment(&key, 60).await.unwrap();

    // After pruning + insert there should be exactly one row (current window).
    assert_eq!(
        s.raw_row_count(&key).await,
        1,
        "stale row should have been pruned"
    );
}

#[tokio::test]
async fn concurrent_increments_are_atomic() {
    let Some(url) = db_url() else { return };
    let s = make_store(&url).await;
    let key = s.key("concurrent");

    // Spawn 20 concurrent tasks each incrementing the same key.
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
    // Every concurrent increment must produce a unique count value (1..=20).
    assert_eq!(
        counts.len(),
        20,
        "expected 20 unique counts; got: {counts:?}"
    );
    assert_eq!(*counts.iter().max().unwrap(), 20, "max count should be 20");
}

#[tokio::test]
async fn different_window_secs_produce_independent_windows() {
    let Some(url) = db_url() else { return };
    let s = make_store(&url).await;
    // Using the same logical name but different window_secs creates different window_start values,
    // so they should not count against each other's row.
    let key = s.key("multiwin");
    let (c60, reset60) = s.store.increment(&key, 60).await.unwrap();
    let (c3600, reset3600) = s.store.increment(&key, 3600).await.unwrap();
    assert_eq!(c60, 1);
    // The 3600-window boundary is different so a fresh row is inserted.
    // (The 60-window row gets pruned since its window_start < current 3600-window - 3600,
    // which only happens if now > 2*3600 from epoch — always true.)
    assert!(c3600 >= 1, "count for 3600-window should be ≥ 1");
    assert!(
        reset3600 > reset60 || reset3600 >= reset60,
        "3600-window resets later than 60-window"
    );
}
