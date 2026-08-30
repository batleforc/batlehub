//! A whole-registry document, cached per grant set.
//!
//! RFC 0015 §11.7 arm 3. Phase 0b measured the filtered-uncached document at
//! **806× the cached one** on the M corpus and concluded the grant-set key is
//! *load-bearing* rather than an optimisation (§13.2). This is that key, doing
//! its job.
//!
//! # Why the key is a grant set and not an identity
//!
//! §4.4 rule 3 forbids caching a filtered listing under an identity-blind key —
//! finding 11's lesson, where the search cache replayed one caller's private
//! results to the next. The obvious fix is a per-caller cache, and it is the
//! wrong one: an estate with ten thousand users would hold ten thousand copies
//! of the same bytes.
//!
//! The resolved **grant set** is the smallest thing the document actually
//! depends on. Callers who resolve to the same permissions are entitled to the
//! same bytes, so they share one entry, and §11.7 measures "number of distinct
//! grant sets exercised" precisely because arm 3's viability rests on that
//! number being small — a property of real configurations rather than of code.
//!
//! # Generation, not just TTL
//!
//! A TTL alone is wrong here and the tree has already been bitten by it: conda's
//! `repodata.json.zst` was keyed on the *blocked-set* fingerprint, which a
//! publish does not change, so a client that had probed the channel once kept
//! being served pre-publish bytes while the uncompressed document showed the new
//! package. The two encodings described different channels
//! (`publish_traversal_guards.rs`).
//!
//! So an entry carries the registry's **generation**, and any write to that
//! registry bumps it. A publish is visible on the next request, not on the next
//! expiry. The TTL is a memory bound on top of that, not the correctness
//! mechanism.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::RwLock;

/// How long an entry may live even if nothing writes to the registry.
const TTL: Duration = Duration::from_secs(300);

/// How many documents to hold. A whole-registry index can be tens of megabytes,
/// so this is small on purpose: the win is one entry shared by every caller with
/// the same grants, not many entries.
const MAX_ENTRIES: usize = 64;

struct Entry {
    body: Arc<String>,
    generation: u64,
    stored_at: Instant,
}

/// Documents keyed by `(document, grant set)`, invalidated by generation.
#[derive(Default)]
pub struct DocumentCache {
    entries: RwLock<HashMap<String, Entry>>,
    /// Per-registry write counter. Bumped by every publish and lifecycle change.
    generations: RwLock<HashMap<String, Arc<AtomicU64>>>,
}

impl DocumentCache {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    async fn generation_handle(&self, registry: &str) -> Arc<AtomicU64> {
        if let Some(g) = self.generations.read().await.get(registry) {
            return Arc::clone(g);
        }
        let mut w = self.generations.write().await;
        Arc::clone(
            w.entry(registry.to_owned())
                .or_insert_with(|| Arc::new(AtomicU64::new(0))),
        )
    }

    /// Note that `registry` changed. Every cached document for it is now stale.
    ///
    /// Cheap and unconditional: a counter bump, not a scan. The stale entries are
    /// left to be overwritten or to expire — evicting them eagerly would mean
    /// walking the map on every publish, and a publish is the one operation that
    /// should not pay for the read path's cache.
    pub async fn invalidate_registry(&self, registry: &str) {
        self.generation_handle(registry)
            .await
            .fetch_add(1, Ordering::Relaxed);
    }

    /// The cached body, if one is current.
    pub async fn get(&self, registry: &str, key: &str) -> Option<Arc<String>> {
        let current = self
            .generation_handle(registry)
            .await
            .load(Ordering::Relaxed);
        let entries = self.entries.read().await;
        let entry = entries.get(key)?;
        if entry.generation != current || entry.stored_at.elapsed() > TTL {
            return None;
        }
        Some(Arc::clone(&entry.body))
    }

    /// Store a body against the registry's generation *as it is now*.
    ///
    /// Read before the document was built, not after: a publish that lands while
    /// the document is being rendered must invalidate the result, and taking the
    /// generation afterwards would stamp the new value onto bytes that predate
    /// it. Callers pass the value they read before starting.
    pub async fn put(&self, key: String, body: Arc<String>, generation: u64) {
        let mut entries = self.entries.write().await;
        if entries.len() >= MAX_ENTRIES && !entries.contains_key(&key) {
            // Evict the oldest. A true LRU would need access tracking on the
            // read path, and the win here is that a handful of grant sets share
            // a handful of entries — not that the eviction order is optimal.
            if let Some(oldest) = entries
                .iter()
                .min_by_key(|(_, e)| e.stored_at)
                .map(|(k, _)| k.clone())
            {
                entries.remove(&oldest);
            }
        }
        entries.insert(
            key,
            Entry {
                body,
                generation,
                stored_at: Instant::now(),
            },
        );
    }

    /// The registry's current generation, for a caller about to build a document.
    pub async fn generation(&self, registry: &str) -> u64 {
        self.generation_handle(registry)
            .await
            .load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_stored_document_reads_back() {
        let cache = DocumentCache::new();
        let gen = cache.generation("reg").await;
        cache
            .put(
                "reg/versions:grants=abc".to_owned(),
                Arc::new("body".to_owned()),
                gen,
            )
            .await;
        assert_eq!(
            cache.get("reg", "reg/versions:grants=abc").await.as_deref(),
            Some(&"body".to_owned())
        );
    }

    /// A write to the registry invalidates its documents immediately.
    ///
    /// The conda bug, as a test: `repodata.json.zst` was keyed on a fingerprint a
    /// publish did not change, so a client that had probed the channel once was
    /// served pre-publish bytes indefinitely while the uncompressed document
    /// showed the new package. A TTL would have healed that eventually; a
    /// resolver does not wait.
    #[tokio::test]
    async fn a_publish_invalidates_the_registrys_documents() {
        let cache = DocumentCache::new();
        let gen = cache.generation("reg").await;
        cache
            .put(
                "reg/versions:grants=abc".to_owned(),
                Arc::new("old".to_owned()),
                gen,
            )
            .await;

        cache.invalidate_registry("reg").await;

        assert!(
            cache.get("reg", "reg/versions:grants=abc").await.is_none(),
            "a publish must be visible on the next request, not the next expiry"
        );
    }

    /// One registry's write does not invalidate another's.
    #[tokio::test]
    async fn invalidation_is_scoped_to_its_registry() {
        let cache = DocumentCache::new();
        let gen = cache.generation("other").await;
        cache
            .put(
                "other/versions:grants=abc".to_owned(),
                Arc::new("body".to_owned()),
                gen,
            )
            .await;
        cache.invalidate_registry("reg").await;
        assert!(cache
            .get("other", "other/versions:grants=abc")
            .await
            .is_some());
    }

    /// Two grant sets do not share an entry.
    ///
    /// §4.4 rule 3 in the cache itself: the key carries the grant set, so a
    /// narrow caller cannot be served a broad caller's document however the
    /// entries are stored.
    #[tokio::test]
    async fn two_grant_sets_do_not_share_an_entry() {
        let cache = DocumentCache::new();
        let gen = cache.generation("reg").await;
        cache
            .put(
                "reg/versions:grants=broad".to_owned(),
                Arc::new("everything".to_owned()),
                gen,
            )
            .await;
        assert!(cache
            .get("reg", "reg/versions:grants=narrow")
            .await
            .is_none());
    }

    /// The cache is bounded.
    #[tokio::test]
    async fn the_cache_evicts_rather_than_growing() {
        let cache = DocumentCache::new();
        let gen = cache.generation("reg").await;
        for i in 0..(MAX_ENTRIES + 10) {
            cache
                .put(format!("reg/doc{i}"), Arc::new(i.to_string()), gen)
                .await;
        }
        assert!(cache.entries.read().await.len() <= MAX_ENTRIES);
    }
}
