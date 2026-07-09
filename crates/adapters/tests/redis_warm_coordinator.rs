#![cfg(feature = "cache-redis")]
//! Integration tests for `RedisWarmCoordinator`.
//!
//! Requires a running Redis instance. Set `REDIS_URL` to opt in:
//!
//!   REDIS_URL=redis://localhost:6379 \
//!     cargo test -p batlehub-adapters --test redis_warm_coordinator --features cache-redis
//!
//! Tests are skipped (not failed) when `REDIS_URL` is unset.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use batlehub_adapters::cache::RedisWarmCoordinator;
use batlehub_core::ports::WarmCoordinator;

fn redis_url() -> Option<String> {
    std::env::var("REDIS_URL").ok()
}

static TEST_ID: AtomicU64 = AtomicU64::new(0);

fn unique_key() -> String {
    let id = TEST_ID.fetch_add(1, Ordering::Relaxed);
    format!("warm-coordinator-test-{id}")
}

#[tokio::test]
async fn first_claim_succeeds() {
    let Some(url) = redis_url() else { return };
    let coord = RedisWarmCoordinator::new(&url).await.unwrap();
    let key = unique_key();
    assert!(coord.try_claim(&key, Duration::from_secs(30)).await);
    coord.release(&key).await;
}

#[tokio::test]
async fn second_claim_on_same_key_fails_until_released() {
    let Some(url) = redis_url() else { return };
    let coord = RedisWarmCoordinator::new(&url).await.unwrap();
    let key = unique_key();

    assert!(coord.try_claim(&key, Duration::from_secs(30)).await);
    assert!(
        !coord.try_claim(&key, Duration::from_secs(30)).await,
        "a second claim on the same key must be rejected"
    );

    coord.release(&key).await;
    assert!(
        coord.try_claim(&key, Duration::from_secs(30)).await,
        "claim should succeed again after release"
    );
    coord.release(&key).await;
}

#[tokio::test]
async fn claim_expires_after_ttl() {
    let Some(url) = redis_url() else { return };
    let coord = RedisWarmCoordinator::new(&url).await.unwrap();
    let key = unique_key();

    assert!(coord.try_claim(&key, Duration::from_millis(300)).await);
    tokio::time::sleep(Duration::from_millis(900)).await;
    assert!(
        coord.try_claim(&key, Duration::from_secs(30)).await,
        "claim should succeed again once the TTL has fired"
    );
    coord.release(&key).await;
}

#[tokio::test]
async fn release_of_unclaimed_key_is_a_no_op() {
    let Some(url) = redis_url() else { return };
    let coord = RedisWarmCoordinator::new(&url).await.unwrap();
    coord.release(&unique_key()).await;
}
