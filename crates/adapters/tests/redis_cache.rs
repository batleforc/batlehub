#![cfg(feature = "cache-redis")]
//! Integration tests for `RedisCacheStore`.
//!
//! Requires a running Redis instance. Set `REDIS_URL` to opt in:
//!
//!   task test:redis-cache                                     # starts Redis via Podman automatically
//!   REDIS_URL=redis://localhost:6379 \
//!     cargo test -p batlehub-adapters --test redis_cache --features cache-redis
//!
//! Tests are skipped (not failed) when `REDIS_URL` is unset.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use chrono::Utc;
use redis::aio::ConnectionManager;
use redis::AsyncCommands as _;

use batlehub_adapters::cache::RedisCacheStore;
use batlehub_core::{
    entities::{PackageId, PackageMetadata},
    ports::{CacheEntry, CacheStore},
};

// ── Test helpers ──────────────────────────────────────────────────────────────

fn redis_url() -> Option<String> {
    std::env::var("REDIS_URL").ok()
}

// Each test gets a unique numeric prefix so tests can run in parallel without
// interfering with each other's keys.
static TEST_ID: AtomicU64 = AtomicU64::new(0);

struct TestStore {
    store: RedisCacheStore,
    /// Raw connection for inspecting / manipulating Redis state directly.
    conn: ConnectionManager,
    prefix: String,
}

impl TestStore {
    /// Return a namespaced key for this test run.
    fn key(&self, name: &str) -> String {
        format!("{}:{}", self.prefix, name)
    }

    /// Simulate live-key expiry without waiting for the TTL by deleting it
    /// directly. The stale shadow is left intact, matching what happens when
    /// Redis evicts a key whose TTL fired.
    async fn expire_live_key(&mut self, logical_key: &str) {
        let redis_key = format!("batlehub:cache:{}", self.key(logical_key));
        self.conn.del::<_, ()>(redis_key).await.unwrap();
    }

    /// Return the remaining TTL in seconds for the live key, or `None` if the
    /// key has no expiry (`-1`) or does not exist (`-2`).
    async fn live_key_ttl_secs(&mut self, logical_key: &str) -> Option<i64> {
        let redis_key = format!("batlehub:cache:{}", self.key(logical_key));
        let ttl: i64 = redis::cmd("TTL")
            .arg(&redis_key)
            .query_async(&mut self.conn)
            .await
            .unwrap();
        if ttl >= 0 {
            Some(ttl)
        } else {
            None
        }
    }

    /// Return `true` if the stale-shadow key exists in Redis.
    async fn stale_key_exists(&mut self, logical_key: &str) -> bool {
        let redis_key = format!("batlehub:cache:{}:stale", self.key(logical_key));
        let exists: bool = self.conn.exists(redis_key).await.unwrap();
        exists
    }
}

async fn make_store(url: &str) -> TestStore {
    let id = TEST_ID.fetch_add(1, Ordering::Relaxed);
    let prefix = format!("redis-cache-test-{id}");
    let store = RedisCacheStore::new(url).await.expect("connect to Redis");
    let client = redis::Client::open(url).expect("open Redis client");
    let conn = ConnectionManager::new(client)
        .await
        .expect("open Redis connection manager for test manipulation");
    TestStore {
        store,
        conn,
        prefix,
    }
}

fn dummy_meta(name: &str) -> PackageMetadata {
    PackageMetadata {
        id: PackageId::new("npm", name, "1.0.0"),
        published_at: Some(Utc::now()),
        download_url: None,
        checksum: None,
        is_signed: None,
        extra: serde_json::json!({}),
        cache_control: None,
    }
}

fn fresh_entry(name: &str) -> CacheEntry {
    CacheEntry {
        metadata: dummy_meta(name),
        cached_at: Utc::now(),
        expires_at: None,
    }
}

// ── CacheStore contract tests (mirror pg_cache.rs) ────────────────────────────

#[tokio::test]
async fn get_returns_none_for_missing_key() {
    let Some(url) = redis_url() else { return };
    let s = make_store(&url).await;
    assert!(s.store.get(&s.key("never-set")).await.unwrap().is_none());
}

#[tokio::test]
async fn set_and_get_round_trip() {
    let Some(url) = redis_url() else { return };
    let s = make_store(&url).await;
    s.store
        .set(&s.key("k1"), fresh_entry("pkg-a"), None)
        .await
        .unwrap();
    let got = s
        .store
        .get(&s.key("k1"))
        .await
        .unwrap()
        .expect("entry should be present");
    assert_eq!(got.metadata.id.name, "pkg-a");
    assert!(got.expires_at.is_none());
}

#[tokio::test]
async fn set_overwrites_existing_entry() {
    let Some(url) = redis_url() else { return };
    let s = make_store(&url).await;
    s.store
        .set(&s.key("k1"), fresh_entry("first"), None)
        .await
        .unwrap();
    s.store
        .set(&s.key("k1"), fresh_entry("second"), None)
        .await
        .unwrap();
    let got = s.store.get(&s.key("k1")).await.unwrap().unwrap();
    assert_eq!(got.metadata.id.name, "second");
}

#[tokio::test]
async fn set_with_ttl_stores_expires_at_in_payload() {
    let Some(url) = redis_url() else { return };
    let s = make_store(&url).await;
    s.store
        .set(
            &s.key("k1"),
            fresh_entry("pkg-ttl"),
            Some(Duration::from_secs(300)),
        )
        .await
        .unwrap();
    let got = s.store.get(&s.key("k1")).await.unwrap().unwrap();
    assert!(
        got.expires_at.is_some(),
        "expires_at should be serialised in the payload"
    );
    assert!(
        got.expires_at.unwrap() > Utc::now(),
        "expires_at should be in the future"
    );
}

#[tokio::test]
async fn expired_live_key_treated_as_miss_by_get() {
    let Some(url) = redis_url() else { return };
    let mut s = make_store(&url).await;
    s.store
        .set(&s.key("k1"), fresh_entry("stale-pkg"), None)
        .await
        .unwrap();
    s.expire_live_key("k1").await; // simulate Redis TTL firing
    assert!(
        s.store.get(&s.key("k1")).await.unwrap().is_none(),
        "expired live key must be a cache miss"
    );
}

#[tokio::test]
async fn get_stale_returns_entry_when_live_key_expired() {
    let Some(url) = redis_url() else { return };
    let mut s = make_store(&url).await;
    s.store
        .set(&s.key("k1"), fresh_entry("stale"), None)
        .await
        .unwrap();
    s.expire_live_key("k1").await;
    assert!(
        s.store.get(&s.key("k1")).await.unwrap().is_none(),
        "get should miss after live key expired"
    );
    assert!(
        s.store.get_stale(&s.key("k1")).await.unwrap().is_some(),
        "get_stale should return the stale shadow"
    );
}

#[tokio::test]
async fn get_stale_returns_none_for_missing_key() {
    let Some(url) = redis_url() else { return };
    let s = make_store(&url).await;
    assert!(s
        .store
        .get_stale(&s.key("never-set"))
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn get_stale_returns_non_expired_entry() {
    let Some(url) = redis_url() else { return };
    let s = make_store(&url).await;
    s.store
        .set(&s.key("k1"), fresh_entry("live"), None)
        .await
        .unwrap();
    assert!(s.store.get_stale(&s.key("k1")).await.unwrap().is_some());
}

#[tokio::test]
async fn invalidate_removes_live_entry() {
    let Some(url) = redis_url() else { return };
    let s = make_store(&url).await;
    let k = s.key("k1");
    s.store.set(&k, fresh_entry("pkg"), None).await.unwrap();
    s.store.invalidate(&k).await.unwrap();
    assert!(s.store.get(&k).await.unwrap().is_none());
}

#[tokio::test]
async fn invalidate_removes_stale_shadow_too() {
    let Some(url) = redis_url() else { return };
    let mut s = make_store(&url).await;
    let k = s.key("k1");
    s.store.set(&k, fresh_entry("pkg"), None).await.unwrap();
    s.expire_live_key("k1").await; // stale key still exists
    assert!(
        s.store.get_stale(&k).await.unwrap().is_some(),
        "stale shadow should exist before invalidate"
    );
    s.store.invalidate(&k).await.unwrap();
    assert!(
        s.store.get_stale(&k).await.unwrap().is_none(),
        "get_stale must return None after invalidate, even if live key had already expired"
    );
}

#[tokio::test]
async fn invalidate_missing_key_is_ok() {
    let Some(url) = redis_url() else { return };
    let s = make_store(&url).await;
    s.store.invalidate(&s.key("ghost")).await.unwrap();
}

// ── Redis-specific behaviour tests ────────────────────────────────────────────

#[tokio::test]
async fn set_without_ttl_live_key_has_no_redis_expiry() {
    let Some(url) = redis_url() else { return };
    let mut s = make_store(&url).await;
    s.store
        .set(&s.key("k1"), fresh_entry("persistent"), None)
        .await
        .unwrap();
    assert!(
        s.live_key_ttl_secs("k1").await.is_none(),
        "live key should have no Redis TTL when set() is called without a TTL"
    );
}

#[tokio::test]
async fn set_with_ttl_live_key_has_redis_expiry() {
    let Some(url) = redis_url() else { return };
    let mut s = make_store(&url).await;
    s.store
        .set(
            &s.key("k1"),
            fresh_entry("ephemeral"),
            Some(Duration::from_secs(120)),
        )
        .await
        .unwrap();
    let ttl = s.live_key_ttl_secs("k1").await;
    assert!(
        ttl.is_some(),
        "live key should carry a Redis TTL when set() is called with a TTL"
    );
    assert!(
        ttl.unwrap() <= 120,
        "TTL should not exceed the requested value"
    );
    assert!(ttl.unwrap() > 0, "TTL should be positive");
}

#[tokio::test]
async fn stale_key_always_has_no_redis_expiry() {
    let Some(url) = redis_url() else { return };
    let mut s = make_store(&url).await;
    // Even when set() receives a TTL, the stale shadow must have no Redis TTL.
    s.store
        .set(
            &s.key("k1"),
            fresh_entry("pkg"),
            Some(Duration::from_secs(60)),
        )
        .await
        .unwrap();
    let stale_key = format!("batlehub:cache:{}:stale", s.key("k1"));
    let ttl: i64 = redis::cmd("TTL")
        .arg(&stale_key)
        .query_async(&mut s.conn)
        .await
        .unwrap();
    assert_eq!(ttl, -1, "stale shadow must never have a Redis TTL");
}

#[tokio::test]
async fn redis_native_ttl_evicts_live_key_but_stale_persists() {
    let Some(url) = redis_url() else { return };
    let s = make_store(&url).await;
    // Use a short real TTL so Redis itself evicts the live key; sleep 3x the
    // TTL (rather than +200ms) so the assertion isn't racing wall-clock expiry
    // under load.
    s.store
        .set(
            &s.key("k1"),
            fresh_entry("short-lived"),
            Some(Duration::from_millis(300)),
        )
        .await
        .unwrap();
    assert!(
        s.store.get(&s.key("k1")).await.unwrap().is_some(),
        "entry should be present immediately after set"
    );

    tokio::time::sleep(Duration::from_millis(900)).await;

    assert!(
        s.store.get(&s.key("k1")).await.unwrap().is_none(),
        "get must miss once the Redis TTL has fired"
    );
    assert!(
        s.store.get_stale(&s.key("k1")).await.unwrap().is_some(),
        "get_stale must still return the entry after the live key was evicted by Redis TTL"
    );
}

#[tokio::test]
async fn invalidate_after_ttl_expiry_removes_stale_shadow() {
    let Some(url) = redis_url() else { return };
    let s = make_store(&url).await;
    s.store
        .set(
            &s.key("k1"),
            fresh_entry("pkg"),
            Some(Duration::from_millis(300)),
        )
        .await
        .unwrap();
    // Sleep 3x the TTL (rather than +200ms) so this isn't racing wall-clock
    // expiry under load.
    tokio::time::sleep(Duration::from_millis(900)).await;

    assert!(
        s.store.get(&s.key("k1")).await.unwrap().is_none(),
        "live key should have expired"
    );
    s.store.invalidate(&s.key("k1")).await.unwrap();
    assert!(
        s.store.get_stale(&s.key("k1")).await.unwrap().is_none(),
        "stale shadow must be gone after invalidate even when live key had already expired"
    );
}

#[tokio::test]
async fn stale_shadow_exists_independently_of_live_key() {
    let Some(url) = redis_url() else { return };
    let mut s = make_store(&url).await;
    s.store
        .set(&s.key("k1"), fresh_entry("pkg"), None)
        .await
        .unwrap();
    assert!(
        s.stale_key_exists("k1").await,
        "stale shadow should be created on set"
    );
    s.expire_live_key("k1").await;
    assert!(
        s.stale_key_exists("k1").await,
        "stale shadow should survive live key deletion"
    );
    s.store.invalidate(&s.key("k1")).await.unwrap();
    assert!(
        !s.stale_key_exists("k1").await,
        "stale shadow should be gone after invalidate"
    );
}

#[tokio::test]
async fn set_updates_stale_shadow_on_overwrite() {
    let Some(url) = redis_url() else { return };
    let s = make_store(&url).await;
    s.store
        .set(&s.key("k1"), fresh_entry("v1"), None)
        .await
        .unwrap();
    s.store
        .set(&s.key("k1"), fresh_entry("v2"), None)
        .await
        .unwrap();
    // After overwrite the stale shadow must hold the latest value.
    let stale = s.store.get_stale(&s.key("k1")).await.unwrap().unwrap();
    assert_eq!(
        stale.metadata.id.name, "v2",
        "stale shadow must reflect the latest set() call"
    );
}
