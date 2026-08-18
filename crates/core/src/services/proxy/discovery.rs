//! The console's discovery read: asking upstream about a package this instance
//! holds nothing of (RFC 0007 §5.5).
//!
//! The one path in this crate a **page view** can start, and it is deliberately
//! not on the write side of anything. It records no `AccessEvent`, writes no
//! `package_statuses` row, increments no download count, touches no
//! `last_accessed`, consumes no quota, creates no storage entry, and does not
//! make the package appear in the catalogue. A page view must not be able to
//! change what the catalogue claims this instance has — otherwise browsing the
//! console silently rewrites the inventory an operator reads to make decisions
//! (§4.4).
//!
//! Nothing new leaves that the proxy would not otherwise fetch: the request is
//! the same version document a package manager pointed at this registry causes
//! on its first resolve, to the same host, through the same client, with the
//! same timeouts, TLS settings, upstream auth and SSRF guards.

use std::time::Duration;

use crate::entities::{Identity, PackageId, RegistryKind, UpstreamDetailSupport};
use crate::error::CoreError;
use crate::ports::DocumentKind;
use crate::services::upstream_detail::{self, UpstreamDetail, UpstreamVersion};

use super::{Freshness, ProxyRequest, ProxyService};

/// How long a reader waits for another reader's in-flight fetch before giving
/// up and doing its own.
///
/// Longer than a healthy upstream fetch and shorter than a page load anybody
/// would sit through: the point is to collapse concurrent readers, not to make
/// one reader's slow upstream everybody's problem.
const SINGLE_FLIGHT_WAIT: Duration = Duration::from_secs(10);

/// What one discovery read did, so the page can say which rung answered.
pub struct DiscoveryOutcome {
    pub detail: UpstreamDetail,
    pub freshness: Freshness,
    /// `max_versions` shortened the list. Surfaced, because a silently
    /// shortened list is a lie about the registry.
    pub truncated: bool,
}

impl ProxyService {
    /// Ask upstream what versions of `name` exist, for the console only.
    ///
    /// `Ok(None)` means the read was not attempted, and the four reasons are
    /// all legitimate answers rather than failures: the registry has it turned
    /// off, the kind has no upstream to ask, the registry is `local`-mode, or
    /// upstream is already known not to have this package.
    ///
    /// `Err` means it was attempted and every rung failed — the page then
    /// answers from local rows with the error reported, never an empty page
    /// presented as an answer.
    pub async fn upstream_detail(
        &self,
        registry: &str,
        name: &str,
        identity: &Identity,
    ) -> Result<Option<DiscoveryOutcome>, CoreError> {
        let cfg = {
            let hot = self.hot.read().await;
            hot.upstream_detail
                .get(registry)
                .cloned()
                .unwrap_or_default()
        };
        if !cfg.enabled {
            return Ok(None);
        }

        // `request_prelude` rather than a second config read, so the access
        // checks, the client, the TTL and the stale policy are the ones the
        // proxy path already applies rather than a second copy that can drift.
        //
        // A listing document addresses a *package*, not a version, but
        // `validate_coordinate` refuses an empty one — so a placeholder stands
        // in, exactly as `rubygems/compact.rs` uses `__compact__`. It never
        // reaches a URL or a storage key: the cache key below is built from the
        // document kind, and nothing on this path fetches an artifact.
        let req = ProxyRequest {
            package_id: PackageId::new(registry, name, "__listing__"),
            identity: identity.clone(),
            resource_type: "releases:read".to_owned(),
            ip_address: None,
            user_agent: None,
        };
        let prelude = self.request_prelude(&req).await?;

        let kind: RegistryKind = prelude
            .client
            .registry_type()
            .parse()
            .map_err(|_| CoreError::NotSupported("unknown registry type".to_owned()))?;

        let doc_kind = match kind.upstream_detail() {
            UpstreamDetailSupport::Document(name) => document_kind(name),
            UpstreamDetailSupport::ListVersions => {
                return self.upstream_detail_by_listing(&prelude, name, &cfg).await
            }
            UpstreamDetailSupport::None(_) => return Ok(None),
        };

        // The single-flight and negative-cache key is the metadata cache key,
        // so a page reload during a fetch collapses into the same flight and an
        // absence is remembered per document rather than per package.
        let key = format!("doc:{registry}:{}:{name}", doc_kind.as_str());
        if self.discovery.is_absent(&key) {
            return Ok(None);
        }

        let guard = self.discovery.claim(&key, SINGLE_FLIGHT_WAIT).await;
        if guard.is_none() {
            // Somebody else just finished, and wrote the answer before
            // releasing. Reading their cache entry is rung 1 by definition, and
            // is the whole point of coalescing: N readers, one request.
            return Ok(self.cached_document_only(&key).await.map(|doc| {
                let (detail, truncated) = capped(upstream_detail::dispatch(kind, &doc), &cfg);
                DiscoveryOutcome {
                    detail,
                    freshness: Freshness::Cached,
                    truncated,
                }
            }));
        }

        // Whether the answer was already there decides which rung this is, and
        // it has to be asked *before* the fetch that would put it there.
        let fresh_before = self.cached_document_only(&key).await.is_some();
        let doc = match self
            .cached_version_document(&prelude, &req, name, doc_kind)
            .await
        {
            Ok(doc) => doc,
            Err(CoreError::NotFound(_)) => {
                // Upstream *answered*, and the answer was "no such package".
                // That is a fact, so it is remembered — a bad URL or a crawler
                // must not turn every reload into an upstream request.
                self.discovery.record_absent(&key, cfg.negative_ttl);
                return Ok(None);
            }
            // A connection failure is not a fact about the package, so it is
            // not cached. The caller reports it and falls back to local rows.
            //
            // Nor is a `NotSupported`: a client that has no such document has
            // told us about *itself*, not about the package, and remembering it
            // as an absence would hide every package on that registry.
            Err(e) => return Err(e),
        };

        let (detail, truncated) = capped(upstream_detail::dispatch(kind, &doc), &cfg);
        Ok(Some(DiscoveryOutcome {
            detail,
            // Rung 1 when the entry was already there, rung 2 when this call
            // fetched it. `cached_version_document` serves a stale entry itself
            // when the registry allows it, and reports it as cached — the page
            // says how old from `extracted_at`, which is the honest granularity
            // available without changing that function's signature.
            freshness: if fresh_before {
                Freshness::Cached
            } else {
                Freshness::Fresh
            },
            truncated,
        }))
    }

    /// One version's README, from its own metadata document.
    ///
    /// The second half of the derived read, for the kinds whose *listing*
    /// document carries no README but whose per-version metadata does:
    ///
    /// - **PyPI** keeps the long description in `/pypi/{name}/{version}/json`,
    ///   so filling it for a whole version table would be N upstream requests
    ///   per page view. The panel asks on selection instead, which is the cost
    ///   RFC 0007 open question 7 accepts.
    /// - **OpenVSX** and the **VS Code Marketplace** answer with a *URL*, which
    ///   this follows through the client's own same-origin and SSRF guards
    ///   (§7.4).
    ///
    /// Cache-first, and it **writes nothing**: `resolve_metadata_uncaptured` is
    /// the same resolve the download path uses with the capture removed, so N
    /// readers of one version during a TTL produce one request and a later real
    /// download finds the entry warm — while no `package_readmes` row appears
    /// because somebody looked at a page (§5.6).
    pub async fn upstream_version_readme(
        &self,
        registry: &str,
        name: &str,
        version: &str,
        identity: &Identity,
    ) -> Result<Option<(String, crate::entities::ReadmeFormat, bool, Freshness)>, CoreError> {
        let cfg = {
            let hot = self.hot.read().await;
            hot.upstream_detail
                .get(registry)
                .cloned()
                .unwrap_or_default()
        };
        if !cfg.enabled {
            return Ok(None);
        }

        let req = ProxyRequest {
            package_id: PackageId::new(registry, name, version),
            identity: identity.clone(),
            resource_type: "releases:read".to_owned(),
            ip_address: None,
            user_agent: None,
        };
        let prelude = self.request_prelude(&req).await?;

        let kind: RegistryKind = prelude
            .client
            .registry_type()
            .parse()
            .map_err(|_| CoreError::NotSupported("unknown registry type".to_owned()))?;
        // Only the kinds that promise a README for an unheld version. Asking an
        // archive-borne kind would resolve a version whose text is inside bytes
        // we do not have, which is a request for nothing.
        if !kind.readme_support().answers_for_unheld_versions() {
            return Ok(None);
        }
        // And not the kinds whose listing document *is* their README source.
        // npm's packument is both; if the listing read found nothing there, a
        // per-version resolve re-fetches the same document to find the same
        // nothing. Verified against the real registry: `express` ships
        // `readme: ""`, and this is what stops that costing a second fetch per
        // version viewed.
        if upstream_detail::listing_carries_readmes(kind) {
            return Ok(None);
        }

        // Whether the answer was already cached decides the rung, and it has to
        // be asked before the resolve that would cache it.
        let was_cached = self
            .cache
            .get(&prelude.cache_key)
            .await
            .ok()
            .flatten()
            .is_some();
        let meta = self
            .resolve_metadata_uncaptured(
                &prelude.client,
                &prelude.policy,
                &req,
                &prelude.cache_key,
                prelude.ttl,
                &prelude.registry_label,
            )
            .await?;

        let Some(found) = crate::entities::MetadataReadme::from_extra(&meta.extra) else {
            return Ok(None);
        };
        let freshness = if was_cached {
            Freshness::Cached
        } else {
            Freshness::Fresh
        };

        if let Some(content) = found.content {
            return Ok(Some((
                content,
                found.format,
                found.package_level,
                freshness,
            )));
        }
        // A link, which is an outbound request in its own right — made here
        // rather than on any protocol path, and guarded by the client that owns
        // the origin check.
        let Some(url) = found.url.as_deref() else {
            return Ok(None);
        };
        let cfg = {
            let hot = self.hot.read().await;
            hot.readme.get(registry).cloned().unwrap_or_default()
        };
        Ok(prelude
            .client
            .fetch_linked_readme(url, cfg.max_bytes)
            .await?
            .map(|text| (text, found.format, found.package_level, freshness)))
    }

    /// The kinds with no listing document the proxy can read, but which can
    /// enumerate versions: the extension galleries, and conda — whose
    /// `repodata.json` describes a whole channel rather than a package.
    ///
    /// Produces rows with no publish times, which is honest and still the
    /// difference between a versions table and an empty state.
    async fn upstream_detail_by_listing(
        &self,
        prelude: &super::handle::RequestPrelude,
        name: &str,
        cfg: &crate::services::hot_config::UpstreamDetailConfig,
    ) -> Result<Option<DiscoveryOutcome>, CoreError> {
        let versions = prelude.client.list_versions(name).await?;
        let detail = UpstreamDetail {
            versions: versions.into_iter().map(UpstreamVersion::bare).collect(),
            readmes: Default::default(),
        };
        let (detail, truncated) = capped(detail, cfg);
        Ok(Some(DiscoveryOutcome {
            detail,
            // `list_versions` is not cached by this path, so every answer it
            // gives was fetched now. Saying `Cached` would be a claim about a
            // cache that is not involved.
            freshness: Freshness::Fresh,
            truncated,
        }))
    }

    /// A cached listing document, without fetching.
    ///
    /// Used to tell rung 1 from rung 2, and to answer a caller that waited out
    /// somebody else's flight. `get` returns only what the store still
    /// considers fresh, so freshness stays the store's job here exactly as it
    /// is in `cached_version_document`.
    async fn cached_document_only(&self, key: &str) -> Option<crate::ports::VersionDocument> {
        let entry = self.cache.get(key).await.ok().flatten()?;
        serde_json::from_value(entry.metadata.extra).ok()
    }
}

/// The `DocumentKind` a support string names.
///
/// The strings are checked against the real enum by the drift test in
/// `services::upstream_detail`, so an unrecognised one here is unreachable —
/// but it degrades to the primary listing rather than panicking on a page.
fn document_kind(name: &str) -> DocumentKind {
    match name {
        "simple-json" => DocumentKind::SIMPLE_JSON,
        "registration" => DocumentKind::REGISTRATION,
        "latest" => DocumentKind::LATEST,
        "p2-dev" => DocumentKind::P2_DEV,
        "gem" => DocumentKind::GEM,
        _ => DocumentKind::Versions,
    }
}

/// Apply `max_versions`, and say whether it bit.
///
/// The cap keeps the **first N rows of the table**, in the table's own order:
/// stable before pre-release, then newest first. Consistency with what the
/// reader sees matters more than a second ordering here — a truncated list
/// whose kept rows were not the ones at the top would be confusing in a way a
/// shorter list is not, and the reader deciding whether to adopt something is
/// looking at the top of the table either way.
fn capped(
    mut detail: UpstreamDetail,
    cfg: &crate::services::hot_config::UpstreamDetailConfig,
) -> (UpstreamDetail, bool) {
    if detail.versions.len() <= cfg.max_versions {
        return (detail, false);
    }
    // The same crude ordering the version table already uses — stable before
    // pre-release, then by string, descending — so a row does not move
    // depending on which list it came from.
    detail.versions.sort_by(|a, b| {
        a.is_prerelease
            .cmp(&b.is_prerelease)
            .then(b.version.cmp(&a.version))
    });
    detail.versions.truncate(cfg.max_versions);
    let kept: std::collections::HashSet<&str> =
        detail.versions.iter().map(|v| v.version.as_str()).collect();
    detail
        .readmes
        .retain(|version, _| kept.contains(version.as_str()));
    (detail, true)
}
