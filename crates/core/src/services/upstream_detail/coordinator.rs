//! What stops the console becoming a request amplifier (RFC 0007 §5.5, §7.7).
//!
//! Two pieces of per-process state, both deliberately *not* in `ExploreCache`:
//! that cache is keyed by query and invalidated per registry by
//! `invalidate_explore_cache`, so a per-package absence marker keyed into it
//! would be cleared by an unrelated catalogue write.
//!
//! - **Single-flight.** Ten operators opening the same new package must produce
//!   one upstream request. Without it the console amplifies requests under
//!   exactly the conditions that make a package interesting: several people
//!   looking at it at once.
//! - **A negative cache.** A `404` is a *fact* — RFC 0009's distinction between
//!   *failed* and *answered something other than success* — so it is remembered,
//!   and a bad URL, a typo or a crawler cannot turn every reload into an
//!   upstream request. A connection failure is not a fact about the package and
//!   is not cached at all.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::Notify;

/// Per-process coordination for the discovery read.
#[derive(Default)]
pub struct UpstreamDetailCoordinator {
    /// Keys with a read in progress, and what to wake when it finishes.
    in_flight: Mutex<HashMap<String, Arc<Notify>>>,
    /// Coordinates upstream has said it does not have, and when to forget that.
    absent: Mutex<HashMap<String, Instant>>,
}

/// Held for the duration of a read; releasing it wakes everyone waiting on the
/// same key.
///
/// A guard rather than a `finish(key)` call, so an early return or a `?` cannot
/// leave a key marked in-flight forever — which would wedge that package's page
/// for the life of the process.
pub struct FlightGuard {
    coordinator: Arc<UpstreamDetailCoordinator>,
    key: String,
}

impl Drop for FlightGuard {
    fn drop(&mut self) {
        let notify = self.coordinator.in_flight.lock().unwrap().remove(&self.key);
        if let Some(notify) = notify {
            notify.notify_waiters();
        }
    }
}

impl UpstreamDetailCoordinator {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Claim `key`, or wait for whoever already holds it.
    ///
    /// `Some(guard)` means this caller does the work. `None` means someone else
    /// just did it and the caller should re-read the cache — which is where the
    /// answer now is, because the holder writes it before releasing.
    ///
    /// The wait is bounded: a holder that is wedged (a hung upstream against a
    /// client with no timeout) must not hold every other reader with it. On
    /// timeout the caller proceeds on its own, which costs a duplicate request
    /// in a situation that is already broken.
    pub async fn claim(self: &Arc<Self>, key: &str, wait: Duration) -> Option<FlightGuard> {
        loop {
            let waiter = {
                let mut in_flight = self.in_flight.lock().unwrap();
                match in_flight.get(key) {
                    Some(notify) => Arc::clone(notify),
                    None => {
                        in_flight.insert(key.to_owned(), Arc::new(Notify::new()));
                        return Some(FlightGuard {
                            coordinator: Arc::clone(self),
                            key: key.to_owned(),
                        });
                    }
                }
            };
            // `notified()` is registered before the lock is dropped above only
            // in the sense that the `Arc` was cloned under it; a holder that
            // finishes in between wakes nothing, so the loop re-checks the map
            // rather than waiting forever.
            if tokio::time::timeout(wait, waiter.notified()).await.is_err() {
                tracing::debug!(key, "upstream detail: waited out the in-flight read");
                return None;
            }
            if !self.in_flight.lock().unwrap().contains_key(key) {
                return None;
            }
        }
    }

    /// Remember that upstream does not have this coordinate.
    pub fn record_absent(&self, key: &str, ttl: Duration) {
        if ttl.is_zero() {
            return;
        }
        self.absent
            .lock()
            .unwrap()
            .insert(key.to_owned(), Instant::now() + ttl);
    }

    /// Whether this coordinate is remembered as absent.
    ///
    /// Expired entries are dropped as they are found rather than by a sweep:
    /// the map is only read on the path that writes it, so it cannot grow
    /// without being walked.
    pub fn is_absent(&self, key: &str) -> bool {
        let mut absent = self.absent.lock().unwrap();
        match absent.get(key) {
            Some(until) if *until > Instant::now() => true,
            Some(_) => {
                absent.remove(key);
                false
            }
            None => false,
        }
    }

    /// Forget every remembered absence. For tests, and for a config reload that
    /// changes what "absent" would mean.
    pub fn clear_absent(&self) {
        self.absent.lock().unwrap().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_first_caller_claims_and_the_second_waits_for_it() {
        let coordinator = UpstreamDetailCoordinator::new();
        let guard = coordinator
            .claim("npm1:express", Duration::from_secs(5))
            .await
            .expect("first caller does the work");

        let second = {
            let coordinator = Arc::clone(&coordinator);
            tokio::spawn(async move {
                coordinator
                    .claim("npm1:express", Duration::from_secs(5))
                    .await
                    .is_none()
            })
        };

        // Let the second caller reach the wait before the guard is released,
        // so this exercises the wake rather than the fast path.
        tokio::time::sleep(Duration::from_millis(20)).await;
        drop(guard);

        assert!(
            second.await.unwrap(),
            "the second caller should have waited and then re-read the cache"
        );
    }

    /// A different package is a different key, and must not be serialised
    /// behind an unrelated read.
    #[tokio::test]
    async fn a_different_key_is_not_blocked() {
        let coordinator = UpstreamDetailCoordinator::new();
        let _held = coordinator
            .claim("npm1:express", Duration::from_secs(5))
            .await
            .unwrap();
        assert!(coordinator
            .claim("npm1:lodash", Duration::from_secs(5))
            .await
            .is_some());
    }

    /// A wedged holder must not hold every other reader with it.
    #[tokio::test]
    async fn a_waiter_gives_up_rather_than_hanging_forever() {
        let coordinator = UpstreamDetailCoordinator::new();
        let _held = coordinator
            .claim("npm1:express", Duration::from_secs(5))
            .await
            .unwrap();
        assert!(coordinator
            .claim("npm1:express", Duration::from_millis(10))
            .await
            .is_none());
    }

    /// An early return must not leave a key claimed for the life of the
    /// process, which would wedge that package's page permanently.
    #[tokio::test]
    async fn dropping_the_guard_releases_the_key() {
        let coordinator = UpstreamDetailCoordinator::new();
        drop(
            coordinator
                .claim("npm1:express", Duration::from_secs(5))
                .await
                .unwrap(),
        );
        assert!(coordinator
            .claim("npm1:express", Duration::from_secs(5))
            .await
            .is_some());
    }

    #[tokio::test]
    async fn an_absence_is_remembered_until_it_expires() {
        let coordinator = UpstreamDetailCoordinator::new();
        assert!(!coordinator.is_absent("npm1:nope"));

        coordinator.record_absent("npm1:nope", Duration::from_secs(60));
        assert!(coordinator.is_absent("npm1:nope"));
        assert!(!coordinator.is_absent("npm1:other"));

        coordinator.clear_absent();
        assert!(!coordinator.is_absent("npm1:nope"));
    }

    #[tokio::test]
    async fn an_expired_absence_is_forgotten() {
        let coordinator = UpstreamDetailCoordinator::new();
        coordinator.record_absent("npm1:nope", Duration::from_millis(5));
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!coordinator.is_absent("npm1:nope"));
    }

    /// A zero TTL is "do not remember", not "remember forever".
    #[tokio::test]
    async fn a_zero_negative_ttl_remembers_nothing() {
        let coordinator = UpstreamDetailCoordinator::new();
        coordinator.record_absent("npm1:nope", Duration::ZERO);
        assert!(!coordinator.is_absent("npm1:nope"));
    }
}
