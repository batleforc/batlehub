//! One search path, for every ecosystem that has one.
//!
//! RFC 0009 §7.7. Search was five separate non-answers before this: NuGet's
//! `/v3/query` returned a hardcoded `{"totalHits": 0, "data": []}` in proxy and
//! hybrid mode while the service index advertised `SearchQueryService`, `vsx`
//! free-text did the same (honestly, in a comment), and npm, cargo and Composer
//! had no route at all.
//!
//! The NuGet stub is the sharper failure of the two shapes: it *routes*,
//! returns `200`, and is valid JSON of the right form. Every signal a test or a
//! conformance fixture reads is green, and `dotnet package search` reports zero
//! results against a registry holding thousands of packages. That is why §5.1
//! needed a `must_find` assertion class — a collection endpoint observed only
//! returning an empty list is indistinguishable from a stub.
//!
//! # The three rungs
//!
//! ```text
//! 1. cached response for this query   → serve it
//! 2. upstream, then cache the result  → serve it
//! 3. upstream unreachable:
//!      a. stale cached response, when the registry allows stale
//!      b. otherwise: the packages this registry already holds
//! ```
//!
//! Rung 3b is what makes this different from every other cached path, and it is
//! the answer to "what should a search return when the upstream is gone": not an
//! error, and not an empty list, but **what we actually have**. A registry that
//! has cached four hundred packages can answer a search from those four hundred.
//! That is a true answer about this proxy, degraded but honest, and it is
//! strictly better than the empty `200` that shipped — which was a false answer
//! about the upstream.
//!
//! Rung 3a is bounded by the registry's existing `serve_stale_metadata`, so an
//! operator who turned stale serving off — because for their estate a stale
//! answer is worse than none — gets that decision honoured here without
//! discovering a second switch.
//!
//! # Egress
//!
//! Rung 2 forwards the user's query string upstream. That is what makes search
//! useful and it is not free: search queries are a record of what an
//! organisation is looking for. It is documented per registry rather than
//! buried, and `serve_stale = false` with no upstream is the configuration for
//! an operator who wants rung 3b and nothing else.

use std::time::Duration;

use crate::entities::{PackageFilter, RegistryKind};
use crate::error::CoreError;
use crate::ports::UpstreamPackage;
use crate::services::proxy::Freshness;
use crate::services::ProxyService;

/// One search result, in the shape every protocol's renderer needs.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SearchHit {
    pub name: String,
    pub version: String,
    pub description: Option<String>,
}

impl From<UpstreamPackage> for SearchHit {
    fn from(p: UpstreamPackage) -> Self {
        Self {
            name: p.name,
            version: p.latest_version,
            description: p.description,
        }
    }
}

pub struct SearchResults {
    pub hits: Vec<SearchHit>,
    /// Total matching hits **after** filtering.
    ///
    /// Adjusted rather than passed through: clients paginate by offset, so a
    /// page silently shortened by the filter is its own bug — the client asks
    /// for the next page and skips whatever the removal shifted.
    pub total: usize,
    pub freshness: Freshness,
}

/// Which sources a search may draw on, from the registry's mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchMode {
    /// Local only: there is no upstream to ask.
    Local,
    /// All three rungs.
    Proxy,
    /// Upstream results merged with locally held ones, deduped by name.
    Hybrid,
}

/// TTL for a cached search response when the registry sets none.
///
/// Shorter than a metadata TTL on purpose: a search result is a view over a
/// whole registry rather than one package's facts, so it goes stale for reasons
/// no per-package invalidation would catch.
const DEFAULT_SEARCH_TTL: Duration = Duration::from_secs(5 * 60);

impl ProxyService {
    /// Search one registry, through the three rungs.
    ///
    /// `local` is what this registry has *published* — supplied by the caller
    /// because only the web layer holds a `LocalRegistryService`, and because
    /// published packages live in a different store from the one
    /// [`Self::held_packages`] reads. That separation is real rather than a
    /// test artifact: `PackageRepository` records what has been *accessed*
    /// through the proxy, `LocalRegistryBackend` records what has been
    /// *published* to it, and a local-mode registry has only the second.
    pub async fn search(
        &self,
        registry: &str,
        query: &str,
        limit: usize,
        mode: SearchMode,
        local: Vec<SearchHit>,
    ) -> Result<SearchResults, CoreError> {
        let limit = limit.clamp(1, 250);

        if mode == SearchMode::Local {
            // No upstream by definition, and nothing has been proxied into the
            // held set — published packages are the whole answer.
            return Ok(self.finish(registry, local, Freshness::Fresh).await);
        }

        let key = format!("search:{registry}:{limit}:{query}");

        // Rung 1.
        if let Ok(Some(entry)) = self.cache.get(&key).await {
            if let Some(hits) = decode_hits(&entry.metadata.extra) {
                return Ok(self.finish(registry, hits, Freshness::Cached).await);
            }
        }

        // Rung 2.
        let upstream = {
            let hot = self.hot.read().await;
            hot.registries.get(registry).cloned()
        };
        let client = upstream.ok_or_else(|| CoreError::UnknownRegistry(registry.to_owned()))?;

        match client.search_packages(query, limit).await {
            Ok(found) => {
                let mut hits: Vec<SearchHit> = found.into_iter().map(SearchHit::from).collect();

                if mode == SearchMode::Hybrid {
                    merge_local(&mut hits, local.clone());
                }

                let ttl = self.search_ttl(registry).await;
                let entry = crate::ports::CacheEntry {
                    metadata: crate::entities::PackageMetadata::minimal(
                        crate::entities::PackageId::new(registry, "_search", ""),
                        encode_hits(&hits),
                    ),
                    cached_at: chrono::Utc::now(),
                    expires_at: None,
                };
                if let Err(e) = self.cache.set(&key, entry, Some(ttl)).await {
                    tracing::warn!(key = %key, error = %e, "caching search results failed");
                }

                Ok(self.finish(registry, hits, Freshness::Fresh).await)
            }
            Err(e) => {
                // Rung 3a.
                if self.serves_stale(registry).await {
                    if let Ok(Some(stale)) = self.cache.get_stale(&key).await {
                        if let Some(hits) = decode_hits(&stale.metadata.extra) {
                            tracing::warn!(
                                registry, error = %e,
                                "search upstream unavailable, serving stale results"
                            );
                            return Ok(self.finish(registry, hits, Freshness::Stale).await);
                        }
                    }
                }

                // Rung 3b — the packages we actually hold. Never an error and
                // never an empty-because-we-gave-up list: an unreachable
                // upstream degrades search to what this proxy can honestly
                // answer for.
                tracing::warn!(
                    registry, error = %e,
                    "search upstream unavailable, answering from held packages"
                );
                // Rung 3b draws on both stores: what has been proxied through
                // (and so is cached here) and what has been published here.
                let mut hits = self.held_packages(registry, query, limit).await;
                merge_local(&mut hits, local);
                Ok(self.finish(registry, hits, Freshness::Stale).await)
            }
        }
    }

    /// Filter blocked versions out of a result set and count what is left.
    ///
    /// A hit names one version, and there is no list here to repair it against
    /// — the same situation conda's `channeldata.json` is in — so a hit whose
    /// named version is blocked is dropped rather than moved. The registry-wide
    /// snapshot answers every hit in one query.
    async fn finish(
        &self,
        registry: &str,
        hits: Vec<SearchHit>,
        freshness: Freshness,
    ) -> SearchResults {
        let kind = {
            let hot = self.hot.read().await;
            hot.registries
                .get(registry)
                .and_then(|c| c.registry_type().parse::<RegistryKind>().ok())
                .unwrap_or(RegistryKind::Generic)
        };
        let blocked = self
            .blocked_in_registry_snapshot_public(registry, kind)
            .await;

        let kept: Vec<SearchHit> = hits
            .into_iter()
            .filter(|h| !blocked.contains(&h.name, &h.version))
            .collect();

        SearchResults {
            total: kept.len(),
            hits: kept,
            freshness,
        }
    }

    /// The packages this registry already holds — rung 3b.
    ///
    /// `PackageRepository` is the same store `GET /api/v1/packages` reads, so
    /// this is a query against a table already maintained rather than a new
    /// index. Failure is an empty list, not an error: rung 3b exists to answer
    /// when something else has already gone wrong.
    async fn held_packages(&self, registry: &str, query: &str, limit: usize) -> Vec<SearchHit> {
        let filter = PackageFilter {
            registry: Some(registry.to_owned()),
            name_contains: (!query.is_empty()).then(|| query.to_owned()),
            limit: limit as u64,
            ..PackageFilter::new()
        };
        match self.repo.list_packages(filter).await {
            Ok(rows) => rows
                .into_iter()
                .map(|p| SearchHit {
                    name: p.package_id.name,
                    version: p.package_id.version,
                    description: None,
                })
                .collect(),
            Err(e) => {
                tracing::warn!(registry, error = %e, "listing held packages for search failed");
                Vec::new()
            }
        }
    }

    async fn search_ttl(&self, registry: &str) -> Duration {
        let hot = self.hot.read().await;
        hot.policies
            .get(registry)
            .and_then(|p| p.metadata_ttl)
            .unwrap_or(DEFAULT_SEARCH_TTL)
    }
}

/// Merge locally held packages into upstream results, upstream winning on name.
///
/// Hybrid mode publishes locally *and* proxies, so the same package can appear
/// twice; a client that sees one name twice with two versions has no way to
/// tell which this registry would actually serve.
fn merge_local(hits: &mut Vec<SearchHit>, local: Vec<SearchHit>) {
    for l in local {
        if !hits.iter().any(|h| h.name == l.name) {
            hits.push(l);
        }
    }
}

fn encode_hits(hits: &[SearchHit]) -> serde_json::Value {
    serde_json::json!({ "search_hits": hits })
}

fn decode_hits(value: &serde_json::Value) -> Option<Vec<SearchHit>> {
    serde_json::from_value(value.get("search_hits")?.clone()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hits_round_trip_through_the_cache_encoding() {
        let hits = vec![SearchHit {
            name: "express".to_owned(),
            version: "4.18.2".to_owned(),
            description: Some("web framework".to_owned()),
        }];
        assert_eq!(decode_hits(&encode_hits(&hits)), Some(hits));
    }

    #[test]
    fn another_writers_cache_entry_is_not_mistaken_for_search_results() {
        assert_eq!(decode_hits(&serde_json::json!({"name": "express"})), None);
    }

    /// Hybrid must not show one name twice: a client seeing two versions of the
    /// same package cannot tell which this registry would serve.
    #[test]
    fn merging_local_results_does_not_duplicate_a_name() {
        let mut hits = vec![SearchHit {
            name: "shared".to_owned(),
            version: "2.0.0".to_owned(),
            description: None,
        }];
        merge_local(
            &mut hits,
            vec![
                SearchHit {
                    name: "shared".to_owned(),
                    version: "1.0.0".to_owned(),
                    description: None,
                },
                SearchHit {
                    name: "local-only".to_owned(),
                    version: "1.0.0".to_owned(),
                    description: None,
                },
            ],
        );
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].version, "2.0.0", "upstream wins on a shared name");
        assert!(hits.iter().any(|h| h.name == "local-only"));
    }
}
