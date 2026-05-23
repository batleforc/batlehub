//! Integration tests for `PgCacheStore`.
//!
//! Requires a running PostgreSQL instance. Set `DATABASE_URL` to opt in:
//!
//!   task test:pg-cache                              # starts Postgres via Podman automatically
//!   DATABASE_URL=postgresql://batlehub:changeme@localhost/batlehub \
//!     cargo test -p batlehub-adapters --test pg_cache

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use chrono::Utc;
use sqlx::PgPool;

use batlehub_adapters::cache::PgCacheStore;
use batlehub_core::{
    entities::{PackageId, PackageMetadata},
    ports::{CacheEntry, CacheStore},
};

fn db_url() -> Option<String> {
    std::env::var("DATABASE_URL").ok()
}

// Each test gets an isolated key prefix so tests can run in parallel without
// interfering with each other's rows.
static TEST_ID: AtomicU64 = AtomicU64::new(0);

struct TestStore {
    store: PgCacheStore,
    pool: PgPool,
    prefix: String,
}

impl TestStore {
    fn key(&self, name: &str) -> String {
        format!("{}:{}", self.prefix, name)
    }
}

async fn make_store(url: &str) -> TestStore {
    let id = TEST_ID.fetch_add(1, Ordering::Relaxed);
    let prefix = format!("t{id}");
    let pool = PgPool::connect(url).await.expect("connect to postgres");
    batlehub_adapters::migrations::embedded_migrator().run(&pool).await.expect("run migrations");
    TestStore { store: PgCacheStore::new(pool.clone()), pool, prefix }
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
    CacheEntry { metadata: dummy_meta(name), cached_at: Utc::now(), expires_at: None }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn get_returns_none_for_missing_key() {
    let Some(url) = db_url() else { return };
    let s = make_store(&url).await;
    assert!(s.store.get(&s.key("never-set")).await.unwrap().is_none());
}

#[tokio::test]
async fn set_and_get_round_trip() {
    let Some(url) = db_url() else { return };
    let s = make_store(&url).await;
    s.store.set(&s.key("k1"), fresh_entry("pkg-a"), None).await.unwrap();
    let got = s.store.get(&s.key("k1")).await.unwrap().expect("entry should be present");
    assert_eq!(got.metadata.id.name, "pkg-a");
    assert!(got.expires_at.is_none());
}

#[tokio::test]
async fn set_overwrites_existing_entry() {
    let Some(url) = db_url() else { return };
    let s = make_store(&url).await;
    s.store.set(&s.key("k1"), fresh_entry("first"), None).await.unwrap();
    s.store.set(&s.key("k1"), fresh_entry("second"), None).await.unwrap();
    let got = s.store.get(&s.key("k1")).await.unwrap().unwrap();
    assert_eq!(got.metadata.id.name, "second");
}

#[tokio::test]
async fn set_with_ttl_stores_expires_at() {
    let Some(url) = db_url() else { return };
    let s = make_store(&url).await;
    s.store.set(&s.key("k1"), fresh_entry("pkg-ttl"), Some(Duration::from_secs(300))).await.unwrap();
    let got = s.store.get(&s.key("k1")).await.unwrap().unwrap();
    assert!(got.expires_at.is_some(), "expires_at should be set when a TTL is provided");
    assert!(got.expires_at.unwrap() > Utc::now(), "expires_at should be in the future");
}

#[tokio::test]
async fn expired_entry_treated_as_miss_by_get() {
    let Some(url) = db_url() else { return };
    let s = make_store(&url).await;
    let k = s.key("k1");
    s.store.set(&k, fresh_entry("stale-pkg"), None).await.unwrap();
    sqlx::query(
        "UPDATE metadata_cache SET expires_at = NOW() - INTERVAL '1 hour' WHERE cache_key = $1",
    )
    .bind(&k)
    .execute(&s.pool)
    .await
    .unwrap();
    assert!(s.store.get(&k).await.unwrap().is_none(), "expired entry must be a cache miss");
}

#[tokio::test]
async fn get_stale_returns_expired_entry_that_get_skips() {
    let Some(url) = db_url() else { return };
    let s = make_store(&url).await;
    let k = s.key("k1");
    s.store.set(&k, fresh_entry("stale"), None).await.unwrap();
    sqlx::query(
        "UPDATE metadata_cache SET expires_at = NOW() - INTERVAL '1 hour' WHERE cache_key = $1",
    )
    .bind(&k)
    .execute(&s.pool)
    .await
    .unwrap();

    assert!(s.store.get(&k).await.unwrap().is_none(), "get should skip expired");
    assert!(s.store.get_stale(&k).await.unwrap().is_some(), "get_stale should return expired entry");
}

#[tokio::test]
async fn get_stale_returns_none_for_missing_key() {
    let Some(url) = db_url() else { return };
    let s = make_store(&url).await;
    assert!(s.store.get_stale(&s.key("never-set")).await.unwrap().is_none());
}

#[tokio::test]
async fn get_stale_returns_non_expired_entry() {
    let Some(url) = db_url() else { return };
    let s = make_store(&url).await;
    s.store.set(&s.key("k1"), fresh_entry("live"), None).await.unwrap();
    assert!(s.store.get_stale(&s.key("k1")).await.unwrap().is_some());
}

#[tokio::test]
async fn invalidate_removes_entry() {
    let Some(url) = db_url() else { return };
    let s = make_store(&url).await;
    let k = s.key("k1");
    s.store.set(&k, fresh_entry("pkg"), None).await.unwrap();
    s.store.invalidate(&k).await.unwrap();
    assert!(s.store.get(&k).await.unwrap().is_none());
}

#[tokio::test]
async fn invalidate_removes_expired_entry_from_get_stale() {
    let Some(url) = db_url() else { return };
    let s = make_store(&url).await;
    let k = s.key("k1");
    s.store.set(&k, fresh_entry("pkg"), None).await.unwrap();
    sqlx::query(
        "UPDATE metadata_cache SET expires_at = NOW() - INTERVAL '1 hour' WHERE cache_key = $1",
    )
    .bind(&k)
    .execute(&s.pool)
    .await
    .unwrap();
    s.store.invalidate(&k).await.unwrap();
    assert!(s.store.get_stale(&k).await.unwrap().is_none());
}

#[tokio::test]
async fn invalidate_missing_key_is_ok() {
    let Some(url) = db_url() else { return };
    let s = make_store(&url).await;
    s.store.invalidate(&s.key("ghost")).await.unwrap();
}
