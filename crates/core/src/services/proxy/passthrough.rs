//! Cached passthrough for upstream calls that are not version documents.
//!
//! RFC 0009 §4.2. `ProxyService::handle` has always been cache-first with
//! stale-on-error, but the endpoints that bypass it — `npm audit`, the Go
//! vulnerability database, NuGet's vulnerability pages, and the Go checksum
//! database this RFC adds — each made a bare outbound request with no cache
//! read and no cache write. So every one of them failed outright the moment its
//! upstream was unreachable, including when we had answered the identical
//! request a minute earlier.
//!
//! That is not a degraded cache, it is no cache: a proxy whose vulnerability
//! check only works while the advisory database is up has moved the dependency
//! rather than removed it, and it is the check most likely to be running in a
//! pipeline that must not stop.
//!
//! The three rungs are the ones `cached_version_document` already uses, and
//! rung 3 is bounded by the *same* `serve_stale_metadata` policy — so an
//! operator who turned stale serving off, because for their estate a stale
//! answer is worse than none, gets that decision honoured here without
//! discovering a second switch.
//!
//! Transport lives in the caller. Core does not know what HTTP is, so the
//! fetch is a closure and this module owns only the policy around it.

use std::future::Future;
use std::time::Duration;

use base64::{engine::general_purpose::STANDARD, Engine as _};

use crate::entities::{PackageId, PackageMetadata};
use crate::error::CoreError;
use crate::ports::CacheEntry;

use super::ProxyService;

/// An upstream response held verbatim.
///
/// Bytes rather than a parsed document on purpose: the sumdb is a signed
/// transparency log and the audit responses are somebody else's schema. Nothing
/// here is filtered or rewritten, so nothing here needs to be understood — and
/// [`crate::ports::DocumentBody`] deliberately has no binary variant, which is
/// the same reasoning from the other side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpstreamBytes {
    pub content_type: String,
    pub body: Vec<u8>,
}

impl UpstreamBytes {
    pub fn json(body: Vec<u8>) -> Self {
        Self {
            content_type: "application/json".to_owned(),
            body,
        }
    }

    fn encode(&self) -> serde_json::Value {
        serde_json::json!({
            "passthrough": {
                "content_type": self.content_type,
                "body_b64": STANDARD.encode(&self.body),
            }
        })
    }

    fn decode(value: &serde_json::Value) -> Option<Self> {
        let v = value.get("passthrough")?;
        Some(Self {
            content_type: v.get("content_type")?.as_str()?.to_owned(),
            body: STANDARD.decode(v.get("body_b64")?.as_str()?).ok()?,
        })
    }
}

/// Whether a passthrough answer came from upstream or from a cache we fell back
/// to. Callers surface rung 3 to the client, so a degraded answer is visible
/// rather than silently short.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    /// Rung 1 — a fresh cache entry.
    Cached,
    /// Rung 2 — fetched from upstream on this request.
    Fresh,
    /// Rung 3 — upstream failed and a stale entry answered instead.
    Stale,
}

impl Freshness {
    /// The `X-BatleHub-Cache` value for this outcome.
    pub fn header_value(&self) -> &'static str {
        match self {
            Self::Cached => "hit",
            Self::Fresh => "miss",
            Self::Stale => "stale",
        }
    }
}

/// What one upstream attempt produced.
///
/// The distinction that matters is between an upstream that *failed* and an
/// upstream that *answered something other than success*. A connection refused
/// is an outage and rung 3 exists for it; a `404` is a fact, and answering it
/// from a stale `200` would be inventing data rather than surviving an outage.
pub enum FetchOutcome {
    /// A success worth keeping. Cached, and served with `200`.
    Cacheable(UpstreamBytes),
    /// A definite non-success — the upstream is up and said no. Forwarded
    /// verbatim, never cached, and never replaced by a stale entry.
    Definite { status: u16, bytes: UpstreamBytes },
}

pub struct Passthrough {
    pub status: u16,
    pub bytes: UpstreamBytes,
    pub freshness: Freshness,
}

impl ProxyService {
    /// Whether this registry's policy allows serving stale metadata.
    ///
    /// The same flag `cached_version_document` consults, read the same way, so
    /// the two paths cannot drift into different answers for one registry.
    pub async fn serves_stale(&self, registry: &str) -> bool {
        let hot = self.hot.read().await;
        hot.policies
            .get(registry)
            .map(|p| p.serve_stale_metadata)
            .unwrap_or(false)
    }

    /// Cache-first, upstream, then stale — for an upstream call whose response
    /// is opaque bytes.
    ///
    /// `key` must already be namespaced by the caller (`audit:`, `sumdb:`,
    /// `search:`) and must include everything that selects the response.
    /// `npm audit` POSTs the dependency set, so two projects asking the same
    /// registry are two different questions and must not share an entry.
    ///
    /// Only the fetch failing reaches rung 3. An upstream that answers "no"
    /// answers it, and that answer is the caller's to interpret — serving a
    /// stale 200 over a fresh 404 would be inventing data, not surviving an
    /// outage.
    pub async fn cached_passthrough<F, Fut>(
        &self,
        registry: &str,
        key: &str,
        ttl: Option<Duration>,
        fetch: F,
    ) -> Result<Passthrough, CoreError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<FetchOutcome, CoreError>>,
    {
        // `get` returns only what the store still considers fresh, so freshness
        // is the store's job — as it is in `cached_version_document`.
        if let Ok(Some(entry)) = self.cache.get(key).await {
            if let Some(bytes) = UpstreamBytes::decode(&entry.metadata.extra) {
                return Ok(Passthrough {
                    status: 200,
                    bytes,
                    freshness: Freshness::Cached,
                });
            }
        }

        match fetch().await {
            // Upstream is up and said no. Forward it and keep the cache as it
            // was: a 404 must not evict a good entry, and must not be answered
            // from a stale one.
            Ok(FetchOutcome::Definite { status, bytes }) => Ok(Passthrough {
                status,
                bytes,
                freshness: Freshness::Fresh,
            }),
            Ok(FetchOutcome::Cacheable(bytes)) => {
                let entry = CacheEntry {
                    metadata: PackageMetadata {
                        id: PackageId::new(registry, key, ""),
                        published_at: None,
                        download_url: None,
                        checksum: None,
                        is_signed: None,
                        extra: bytes.encode(),
                        cache_control: None,
                    },
                    cached_at: chrono::Utc::now(),
                    expires_at: None,
                };
                if let Err(e) = self.cache.set(key, entry, ttl).await {
                    tracing::warn!(key = %key, error = %e, "caching passthrough response failed");
                }
                Ok(Passthrough {
                    status: 200,
                    bytes,
                    freshness: Freshness::Fresh,
                })
            }
            Err(e) => {
                if self.serves_stale(registry).await {
                    if let Ok(Some(stale)) = self.cache.get_stale(key).await {
                        if let Some(bytes) = UpstreamBytes::decode(&stale.metadata.extra) {
                            tracing::warn!(
                                key = %key,
                                error = %e,
                                "upstream unavailable, serving stale passthrough response"
                            );
                            return Ok(Passthrough {
                                status: 200,
                                bytes,
                                freshness: Freshness::Stale,
                            });
                        }
                    }
                }
                Err(e)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_round_trip_through_the_cache_encoding() {
        let b = UpstreamBytes {
            content_type: "application/json".to_owned(),
            body: b"{\"advisories\":[]}".to_vec(),
        };
        assert_eq!(UpstreamBytes::decode(&b.encode()), Some(b));
    }

    /// The sumdb is not JSON and not UTF-8-guaranteed; base64 is why the same
    /// path can carry it.
    #[test]
    fn arbitrary_bytes_survive_the_round_trip() {
        let b = UpstreamBytes {
            content_type: "text/plain".to_owned(),
            body: vec![0x00, 0xff, 0xfe, 0x80, b'\n'],
        };
        assert_eq!(UpstreamBytes::decode(&b.encode()), Some(b));
    }

    #[test]
    fn a_cache_entry_from_some_other_writer_is_not_mistaken_for_one_of_ours() {
        assert_eq!(
            UpstreamBytes::decode(&serde_json::json!({"name": "express"})),
            None
        );
        assert_eq!(UpstreamBytes::decode(&serde_json::Value::Null), None);
    }

    #[test]
    fn freshness_names_the_rung_it_came_from() {
        assert_eq!(Freshness::Cached.header_value(), "hit");
        assert_eq!(Freshness::Fresh.header_value(), "miss");
        assert_eq!(Freshness::Stale.header_value(), "stale");
    }
}
