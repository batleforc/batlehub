use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::RwLock;

use crate::entities::{ExploreEntry, ExploreFilter, ExploreSortBy, RegistryStat};

const TTL: Duration = Duration::from_secs(600);

struct PackageEntry {
    items: Vec<ExploreEntry>,
    count: u64,
    /// Registries covered by this query (for registry-scoped invalidation).
    registries: Vec<String>,
    cached_at: Instant,
}

struct StatsEntry {
    stats: Vec<RegistryStat>,
    registries: Vec<String>,
    cached_at: Instant,
}

/// In-memory cache for explore query results (package list + registry stats).
///
/// TTL is 10 minutes. Expired entries are kept as stale shadows so they can be
/// served when the upstream DB is unreachable. Entries are removed only by an
/// explicit `invalidate` call.
pub struct ExploreCache {
    packages: Arc<RwLock<HashMap<String, PackageEntry>>>,
    stats: Arc<RwLock<HashMap<String, StatsEntry>>>,
    ttl: Duration,
}

impl Default for ExploreCache {
    fn default() -> Self {
        Self::new()
    }
}

impl ExploreCache {
    pub fn new() -> Self {
        Self::with_ttl(TTL)
    }

    pub fn with_ttl(ttl: Duration) -> Self {
        Self {
            packages: Arc::new(RwLock::new(HashMap::new())),
            stats: Arc::new(RwLock::new(HashMap::new())),
            ttl,
        }
    }

    // ── package list ──────────────────────────────────────────────────────────

    /// Return fresh (within TTL) package list + count, if cached.
    pub async fn get_packages(&self, key: &str) -> Option<(Vec<ExploreEntry>, u64)> {
        let map = self.packages.read().await;
        let e = map.get(key)?;
        if e.cached_at.elapsed() < self.ttl {
            Some((e.items.clone(), e.count))
        } else {
            None
        }
    }

    /// Return stale package data regardless of TTL.
    /// Returns `None` only if the key was never written or was explicitly invalidated.
    pub async fn get_stale_packages(&self, key: &str) -> Option<(Vec<ExploreEntry>, u64)> {
        let map = self.packages.read().await;
        map.get(key).map(|e| (e.items.clone(), e.count))
    }

    pub async fn set_packages(
        &self,
        key: &str,
        items: Vec<ExploreEntry>,
        count: u64,
        registries: Vec<String>,
    ) {
        let mut map = self.packages.write().await;
        map.insert(
            key.to_owned(),
            PackageEntry {
                items,
                count,
                registries,
                cached_at: Instant::now(),
            },
        );
    }

    // ── registry stats ────────────────────────────────────────────────────────

    pub async fn get_stats(&self, key: &str) -> Option<Vec<RegistryStat>> {
        let map = self.stats.read().await;
        let e = map.get(key)?;
        if e.cached_at.elapsed() < self.ttl {
            Some(e.stats.clone())
        } else {
            None
        }
    }

    pub async fn get_stale_stats(&self, key: &str) -> Option<Vec<RegistryStat>> {
        let map = self.stats.read().await;
        map.get(key).map(|e| e.stats.clone())
    }

    pub async fn set_stats(&self, key: &str, stats: Vec<RegistryStat>, registries: Vec<String>) {
        let mut map = self.stats.write().await;
        map.insert(
            key.to_owned(),
            StatsEntry {
                stats,
                registries,
                cached_at: Instant::now(),
            },
        );
    }

    // ── invalidation ──────────────────────────────────────────────────────────

    /// Invalidate cache entries.
    ///
    /// `registry = None` clears the entire cache. `registry = Some("npm")` removes
    /// only entries whose query touched that registry. This is called automatically
    /// on successful local-registry publish.
    pub async fn invalidate(&self, registry: Option<&str>) {
        match registry {
            None => {
                self.packages.write().await.clear();
                self.stats.write().await.clear();
            }
            Some(reg) => {
                self.packages
                    .write()
                    .await
                    .retain(|_, e| !e.registries.iter().any(|r| r == reg));
                self.stats
                    .write()
                    .await
                    .retain(|_, e| !e.registries.iter().any(|r| r == reg));
            }
        }
    }
}

// ── cache key helpers ─────────────────────────────────────────────────────────

pub fn packages_cache_key(filter: &ExploreFilter) -> String {
    let mut regs = filter.registries.clone();
    regs.sort();
    let sort = match filter.sort_by {
        ExploreSortBy::Downloads => "dl",
        ExploreSortBy::Name => "name",
        ExploreSortBy::Recent => "recent",
    };
    format!(
        "exp:pkg:{}:{}:{}:{}:{}:{}",
        filter.registry.as_deref().unwrap_or(""),
        filter.name_contains.as_deref().unwrap_or(""),
        sort,
        filter.limit,
        filter.offset,
        regs.join(","),
    )
}

/// Registries that a package-list query touches (used for invalidation tracking).
pub fn packages_entry_registries(filter: &ExploreFilter) -> Vec<String> {
    let mut regs = filter.registries.clone();
    if let Some(ref r) = filter.registry {
        if !regs.contains(r) {
            regs.push(r.clone());
        }
    }
    regs
}

pub fn stats_cache_key(accessible_registries: &[String]) -> String {
    let mut regs = accessible_registries.to_vec();
    regs.sort();
    format!("exp:stats:{}", regs.join(","))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::{
        ExploreEntry, ExploreFilter, ExploreSortBy, PackageSource, RegistryStat,
    };

    fn sample_entries() -> Vec<ExploreEntry> {
        vec![ExploreEntry {
            registry: "npm".into(),
            name: "lodash".into(),
            version_count: 3,
            total_downloads: 100,
            last_accessed: None,
            source: PackageSource::Proxied,
            has_blocked: false,
        }]
    }

    fn sample_stats() -> Vec<RegistryStat> {
        vec![RegistryStat {
            registry: "npm".into(),
            package_count: 5,
            total_downloads: 200,
        }]
    }

    // ── package list ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn packages_fresh_hit_returns_data() {
        let cache = ExploreCache::new();
        cache
            .set_packages("k", sample_entries(), 1, vec!["npm".into()])
            .await;
        let result = cache.get_packages("k").await;
        assert!(result.is_some());
        let (items, count) = result.unwrap();
        assert_eq!(items[0].name, "lodash");
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn packages_miss_on_unknown_key() {
        let cache = ExploreCache::new();
        assert!(cache.get_packages("missing").await.is_none());
    }

    #[tokio::test]
    async fn packages_expired_returns_none() {
        let cache = ExploreCache::with_ttl(Duration::from_nanos(1));
        cache
            .set_packages("k", sample_entries(), 1, vec!["npm".into()])
            .await;
        // 1 ns has elapsed by the time we get here
        assert!(cache.get_packages("k").await.is_none());
    }

    #[tokio::test]
    async fn packages_expired_stale_still_returns_data() {
        let cache = ExploreCache::with_ttl(Duration::from_nanos(1));
        cache
            .set_packages("k", sample_entries(), 1, vec!["npm".into()])
            .await;
        let result = cache.get_stale_packages("k").await;
        assert!(result.is_some());
        let (items, _) = result.unwrap();
        assert_eq!(items[0].name, "lodash");
    }

    #[tokio::test]
    async fn packages_stale_returns_none_when_never_set() {
        let cache = ExploreCache::new();
        assert!(cache.get_stale_packages("never").await.is_none());
    }

    // ── registry stats ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn stats_fresh_hit_returns_data() {
        let cache = ExploreCache::new();
        cache
            .set_stats("k", sample_stats(), vec!["npm".into()])
            .await;
        let result = cache.get_stats("k").await;
        assert!(result.is_some());
        assert_eq!(result.unwrap()[0].registry, "npm");
    }

    #[tokio::test]
    async fn stats_expired_returns_none() {
        let cache = ExploreCache::with_ttl(Duration::from_nanos(1));
        cache
            .set_stats("k", sample_stats(), vec!["npm".into()])
            .await;
        assert!(cache.get_stats("k").await.is_none());
    }

    #[tokio::test]
    async fn stats_expired_stale_still_returns_data() {
        let cache = ExploreCache::with_ttl(Duration::from_nanos(1));
        cache
            .set_stats("k", sample_stats(), vec!["npm".into()])
            .await;
        let result = cache.get_stale_stats("k").await;
        assert!(result.is_some());
        assert_eq!(result.unwrap()[0].registry, "npm");
    }

    // ── invalidation ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn invalidate_all_clears_packages_and_stats() {
        let cache = ExploreCache::new();
        cache
            .set_packages("pk", sample_entries(), 1, vec!["npm".into()])
            .await;
        cache
            .set_stats("sk", sample_stats(), vec!["npm".into()])
            .await;

        cache.invalidate(None).await;

        assert!(cache.get_stale_packages("pk").await.is_none());
        assert!(cache.get_stale_stats("sk").await.is_none());
    }

    #[tokio::test]
    async fn invalidate_by_registry_removes_only_matching_entries() {
        let cache = ExploreCache::new();
        cache
            .set_packages("npm-k", sample_entries(), 1, vec!["npm".into()])
            .await;
        cache
            .set_packages("cargo-k", vec![], 0, vec!["cargo".into()])
            .await;
        cache
            .set_stats("npm-s", sample_stats(), vec!["npm".into()])
            .await;
        cache
            .set_stats("cargo-s", vec![], vec!["cargo".into()])
            .await;

        cache.invalidate(Some("npm")).await;

        assert!(
            cache.get_stale_packages("npm-k").await.is_none(),
            "npm entry should be gone"
        );
        assert!(
            cache.get_stale_packages("cargo-k").await.is_some(),
            "cargo entry should remain"
        );
        assert!(
            cache.get_stale_stats("npm-s").await.is_none(),
            "npm stats should be gone"
        );
        assert!(
            cache.get_stale_stats("cargo-s").await.is_some(),
            "cargo stats should remain"
        );
    }

    #[tokio::test]
    async fn invalidate_unknown_registry_leaves_cache_intact() {
        let cache = ExploreCache::new();
        cache
            .set_packages("k", sample_entries(), 1, vec!["npm".into()])
            .await;
        cache.invalidate(Some("pypi")).await;
        assert!(cache.get_stale_packages("k").await.is_some());
    }

    #[tokio::test]
    async fn invalidate_entry_covering_multiple_registries() {
        let cache = ExploreCache::new();
        // An "all registries" query covers both npm and cargo
        cache
            .set_packages("k", sample_entries(), 1, vec!["npm".into(), "cargo".into()])
            .await;

        // Invalidating npm should also remove this cross-registry entry
        cache.invalidate(Some("npm")).await;
        assert!(cache.get_stale_packages("k").await.is_none());
    }

    // ── cache key helpers ─────────────────────────────────────────────────────

    #[test]
    fn packages_cache_key_is_deterministic_regardless_of_registries_order() {
        let f1 = ExploreFilter {
            registry: None,
            registries: vec!["cargo".into(), "npm".into()],
            name_contains: None,
            sort_by: ExploreSortBy::Downloads,
            limit: 20,
            offset: 0,
        };
        let f2 = ExploreFilter {
            registries: vec!["npm".into(), "cargo".into()], // reversed
            ..f1.clone()
        };
        assert_eq!(packages_cache_key(&f1), packages_cache_key(&f2));
    }

    #[test]
    fn packages_cache_key_differs_across_filter_fields() {
        let base = ExploreFilter {
            registry: None,
            registries: vec!["npm".into()],
            name_contains: None,
            sort_by: ExploreSortBy::Downloads,
            limit: 20,
            offset: 0,
        };

        let with_name = ExploreFilter {
            name_contains: Some("lodash".into()),
            ..base.clone()
        };
        let with_sort = ExploreFilter {
            sort_by: ExploreSortBy::Name,
            ..base.clone()
        };
        let with_page = ExploreFilter {
            offset: 20,
            ..base.clone()
        };
        let with_registry = ExploreFilter {
            registry: Some("npm".into()),
            registries: vec![],
            ..base.clone()
        };

        let keys: Vec<_> = [base, with_name, with_sort, with_page, with_registry]
            .iter()
            .map(packages_cache_key)
            .collect();

        // All five keys must be distinct
        for i in 0..keys.len() {
            for j in (i + 1)..keys.len() {
                assert_ne!(keys[i], keys[j], "keys[{i}] == keys[{j}]: {}", keys[i]);
            }
        }
    }

    #[test]
    fn stats_cache_key_sorts_registries() {
        let k1 = stats_cache_key(&["npm".into(), "cargo".into()]);
        let k2 = stats_cache_key(&["cargo".into(), "npm".into()]);
        assert_eq!(k1, k2);
    }

    #[test]
    fn packages_entry_registries_includes_registry_field() {
        let filter = ExploreFilter {
            registry: Some("npm".into()),
            registries: vec!["cargo".into()],
            ..ExploreFilter::default()
        };
        let regs = packages_entry_registries(&filter);
        assert!(regs.contains(&"npm".to_string()));
        assert!(regs.contains(&"cargo".to_string()));
    }

    #[test]
    fn packages_entry_registries_no_duplicates_when_registry_in_registries() {
        let filter = ExploreFilter {
            registry: Some("npm".into()),
            registries: vec!["npm".into()],
            ..ExploreFilter::default()
        };
        let regs = packages_entry_registries(&filter);
        assert_eq!(regs.iter().filter(|r| r.as_str() == "npm").count(), 1);
    }
}
