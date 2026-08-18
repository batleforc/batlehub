use std::sync::Arc;
use std::time::Instant;

use crate::entities::AccessEvent;
use crate::error::CoreError;
use crate::ports::{DocumentKind, VersionDocument};
use crate::rules::{evaluate_rules, RuleContext, RuleDecision};

use super::{ProxyRequest, ProxyResponse, ProxyService, RequestTiming};

/// Largest artifact that re-serve verification (`verify_on_serve`) will retain in
/// memory so it can be hashed and served from the same buffer in a single read.
/// Artifacts above this size are hashed by streaming (memory stays bounded) and
/// then re-opened from storage to serve. 32 MiB comfortably covers typical
/// package artifacts (npm tarballs, wheels, crates) while capping per-request
/// memory for pathologically large ones.
pub(crate) const REVERIFY_BUFFER_LIMIT: usize = 32 * 1024 * 1024;

/// Everything `handle` (and the metadata-only entry point) derives from a
/// `ProxyRequest` before any I/O: the hot-config snapshot for the registry plus
/// the metadata cache key/TTL. Extracted so `resolve_metadata_for` shares the
/// exact same validation, lock discipline, and key derivation as `handle`.
pub(super) struct RequestPrelude {
    pub(super) client: Arc<dyn crate::ports::RegistryClient>,
    pub(super) policy: Option<Arc<crate::services::hot_config::RegistryPolicy>>,
    pub(super) integrity: crate::services::hot_config::IntegrityPolicy,
    pub(super) limit: u64,
    pub(super) cache_key: String,
    pub(super) ttl: Option<std::time::Duration>,
    pub(super) registry_label: Arc<str>,
}

impl ProxyService {
    /// Validate the coordinate, snapshot the registry's hot config (one brief
    /// read lock, released before any async I/O), and derive the metadata
    /// cache key + TTL.
    pub(super) async fn request_prelude(
        &self,
        req: &ProxyRequest,
    ) -> Result<RequestPrelude, CoreError> {
        // Edge chokepoint: reject any package coordinate that would escape the
        // storage root once interpolated into the cache key, before it reaches the
        // metadata cache or the storage backend. Covers every registry that proxies
        // through here, regardless of per-adapter input validation.
        crate::services::validate_coordinate(
            &req.package_id.name,
            &req.package_id.version,
            req.package_id.artifact.as_deref(),
        )?;

        let registry_name: &str = req.package_id.registry.as_str();
        // Arc<str> instead of String: every downstream metrics call clones this
        // cheaply (atomic refcount bump) instead of copying the registry name's
        // bytes on every `counter!`/`histogram!` invocation.
        let registry_label: Arc<str> = Arc::from(registry_name);

        let (client, policy, integrity, limit) = {
            let hot = self.hot.read().await;
            let client = hot
                .registries
                .get(registry_name)
                .ok_or_else(|| CoreError::UnknownRegistry(registry_name.to_owned()))?
                .clone();
            let policy = hot.policies.get(registry_name).cloned();
            // Registries without an explicit `[registries.integrity]` block get the
            // default policy: verify against any advertised checksum, block on mismatch.
            let integrity = hot
                .integrity
                .get(registry_name)
                .cloned()
                .unwrap_or_default();
            let limit = hot.max_artifact_size_bytes.unwrap_or(500 * 1024 * 1024);
            (client, policy, integrity, limit)
        };

        let cache_key = format!("meta:{}", req.package_id.cache_key());
        let ttl = policy.as_ref().and_then(|p| p.metadata_ttl);

        Ok(RequestPrelude {
            client,
            policy,
            integrity,
            limit,
            cache_key,
            ttl,
            registry_label,
        })
    }

    /// Resolve a package's metadata through the cache-first / stale-on-error
    /// pipeline **without** streaming an artifact, enforcing the registry's
    /// policy rules against the resolved metadata (`AccessDenied` on deny).
    ///
    /// This is the metadata-only sibling of [`Self::handle`] — same coordinate
    /// validation, same hot-config snapshot, same `meta:` cache key and TTL —
    /// for handlers that render responses from `PackageMetadata.extra` (e.g.
    /// the JetBrains Marketplace per-plugin endpoints). Because it goes through
    /// `resolve_metadata_cached`, anything resolved once keeps resolving from
    /// cache (or stale cache, when `serve_stale` allows) after upstream loss.
    pub async fn resolve_metadata_for(
        &self,
        req: &ProxyRequest,
    ) -> Result<crate::entities::PackageMetadata, CoreError> {
        let prelude = self.request_prelude(req).await?;
        let metadata = self
            .resolve_metadata_cached(
                &prelude.client,
                &prelude.policy,
                req,
                &prelude.cache_key,
                prelude.ttl,
                &prelude.registry_label,
            )
            .await?;

        let empty: Vec<Box<dyn crate::rules::Rule>> = vec![];
        let rules = prelude
            .policy
            .as_ref()
            .map(|p| p.rules.as_slice())
            .unwrap_or(empty.as_slice());
        let ctx = RuleContext {
            identity: &req.identity,
            package: &metadata,
            resource_type: &req.resource_type,
            cache_entry: None,
            requested_version: Some(&req.package_id.version),
        };
        if let RuleDecision::Deny { reason } = evaluate_rules(rules, &ctx).await {
            return Err(CoreError::AccessDenied(reason));
        }

        Ok(metadata)
    }

    pub async fn handle(&self, req: ProxyRequest) -> Result<ProxyResponse, CoreError> {
        let start = Instant::now();
        let registry_name: &str = req.package_id.registry.as_str();
        let RequestPrelude {
            client,
            policy,
            integrity,
            limit,
            cache_key,
            ttl,
            registry_label,
        } = self.request_prelude(&req).await?;

        // ── 1. Resolve metadata (cache-first) ─────────────────────────────────
        let metadata = self
            .resolve_metadata_cached(&client, &policy, &req, &cache_key, ttl, &registry_label)
            .await?;

        // ── 2. Evaluate rules ──────────────────────────────────────────────────
        let empty: Vec<Box<dyn crate::rules::Rule>> = vec![];
        let rules = policy
            .as_ref()
            .map(|p| p.rules.as_slice())
            .unwrap_or(empty.as_slice());

        let ctx = RuleContext {
            identity: &req.identity,
            package: &metadata,
            resource_type: &req.resource_type,
            cache_entry: None,
            requested_version: Some(&req.package_id.version),
        };

        if let RuleDecision::Deny { reason } = evaluate_rules(rules, &ctx).await {
            super::warn_if_audit_failed(
                self.repo
                    .record_access(AccessEvent::denied_download(
                        req.package_id,
                        req.identity.user_id,
                        req.identity.role,
                        reason.clone(),
                    ))
                    .await,
                "denied download",
            );
            super::finish_request(&registry_label, "denied", start);
            return Ok(ProxyResponse::Denied { reason });
        }

        // ── 3. Firewall-only: stream directly from upstream, skip all caching ──
        let firewall_only = policy.as_ref().map(|p| p.firewall_only).unwrap_or(false);

        if firewall_only {
            tracing::debug!(registry = %registry_name, "firewall-only mode, streaming from upstream");
            let upstream_start = Instant::now();
            let mut upstream = self
                .fetch_artifact_or_record_error(&client, &req, &registry_label, upstream_start)
                .await?;
            // Times the whole body transfer, not just time-to-headers — this is the
            // only latency signal firewall-only registries get, since they never hit
            // the artifact cache path.
            upstream.stream = super::time_upstream_stream(
                Arc::clone(&registry_label),
                "fetch_artifact",
                upstream_start,
                Arc::clone(&self.metrics),
                upstream.stream,
            );
            super::warn_if_audit_failed(
                self.repo
                    .record_access(AccessEvent::allowed_download(
                        req.package_id,
                        req.identity.user_id,
                        req.identity.role,
                    ))
                    .await,
                "allowed download",
            );
            super::finish_request(&registry_label, "allowed", start);
            return Ok(ProxyResponse::Stream(upstream.stream));
        }

        // ── 4. Check artifact cache ────────────────────────────────────────────
        let artifact_key = format!("artifact:{}", req.package_id.cache_key());
        let artifact_ttl = policy.as_ref().and_then(|p| p.artifact_ttl);
        let cached_artifact_is_fresh = self
            .artifact_is_fresh(&artifact_key, artifact_ttl, registry_name)
            .await?;

        if cached_artifact_is_fresh {
            // ── 5a. Cache hit (see `cache::serve_cache_hit`) ──────────────────
            let timing = RequestTiming {
                registry_label,
                start,
            };
            return self
                .serve_cache_hit(req, artifact_key, &integrity, &timing)
                .await;
        }

        // ── 5b. Cache miss: fetch + cache (see `cache::fetch_and_cache`) ───────
        let timing = RequestTiming {
            registry_label,
            start,
        };
        self.fetch_and_cache(req, client, metadata, &integrity, limit, &timing)
            .await
    }

    /// Snapshot a registry's policy out of [`HotConfig`], cloning the `Arc` so
    /// the read lock is released before any `await` on a rule.
    async fn policy_for(
        &self,
        package_id: &crate::entities::PackageId,
    ) -> Option<Arc<crate::services::hot_config::RegistryPolicy>> {
        let hot = self.hot.read().await;
        hot.policies.get(package_id.registry.as_str()).cloned()
    }

    /// The coordinate the authorization entry points judge when no upstream
    /// metadata has been resolved for it — a path-addressed file, or a listing
    /// that names no single version. Every version-derived field is `None`,
    /// which is what confines these calls to identity-keyed rules.
    fn synthetic_metadata(
        package_id: &crate::entities::PackageId,
    ) -> crate::entities::PackageMetadata {
        crate::entities::PackageMetadata {
            id: package_id.clone(),
            published_at: None,
            download_url: None,
            checksum: None,
            is_signed: None,
            extra: serde_json::Value::Null,
            cache_control: None,
        }
    }

    /// Authorize a read against a registry's policy rules **without** resolving
    /// upstream metadata or streaming an artifact.
    ///
    /// Path-addressed registries (deb/rpm) serve approved files straight from
    /// local storage, bypassing [`Self::handle`]. They call this first so a
    /// Local/Hybrid read enforces the same RBAC as the proxy fall-through (which
    /// builds the same synthetic `repo` coordinate and runs the full rule chain).
    /// Returns `AccessDenied` when the policy denies the read.
    pub async fn authorize_read(
        &self,
        package_id: &crate::entities::PackageId,
        identity: &crate::entities::Identity,
        resource_type: &str,
    ) -> Result<(), CoreError> {
        let policy = self.policy_for(package_id).await;
        let empty: Vec<Box<dyn crate::rules::Rule>> = vec![];
        let rules = policy
            .as_ref()
            .map(|p| p.rules.as_slice())
            .unwrap_or(empty.as_slice());

        // Minimal metadata: deb/rpm files have no per-version upstream metadata,
        // and the RBAC rule keys only off the identity. (The proxy fall-through
        // evaluates the same rule set against the same synthetic coordinate.)
        let metadata = Self::synthetic_metadata(package_id);
        let ctx = RuleContext {
            identity,
            package: &metadata,
            resource_type,
            cache_entry: None,
            requested_version: Some(&package_id.version),
        };
        match evaluate_rules(rules, &ctx).await {
            RuleDecision::Deny { reason } => Err(CoreError::AccessDenied(reason)),
            _ => Ok(()),
        }
    }

    /// Authorize a *listing* — a request for a whole package's version document,
    /// not for one version of it.
    ///
    /// Only the identity-keyed `rbac` rule runs. Every other rule in the chain
    /// judges a **concrete version**, and a listing has none: the coordinate
    /// carries the pseudo-version `"latest"` and metadata that is synthetic by
    /// construction (`published_at`, `is_signed` and `checksum` are all `None`,
    /// because no upstream document has been resolved for a single version).
    ///
    /// Handing that to the full chain does not gate the listing, it blanks it.
    /// `LicenseGateRule` with `allow_unknown = false` finds no licence recorded
    /// for `"latest"` and denies; `ReleaseAgeGateRule` with
    /// `deny_missing_timestamp = true` sees `published_at: None` and denies;
    /// `require_signed_release` sees `is_signed: None` and denies; a
    /// `version_gate` allowlist matches nothing against the literal `"latest"`.
    /// Each of those turns "one version in this package is gated" into "`npm
    /// install` of anything from this registry fails", which is the opposite of
    /// letting a resolver route *past* a gated version to one it may have.
    ///
    /// The chain is not skipped, only deferred: it still runs in full on the
    /// download that follows, against the concrete version and its real
    /// metadata. Blocked versions are separately stripped from the document
    /// itself by [`Self::version_document`].
    async fn authorize_listing(
        &self,
        package_id: &crate::entities::PackageId,
        identity: &crate::entities::Identity,
        resource_type: &str,
    ) -> Result<(), CoreError> {
        let Some(policy) = self.policy_for(package_id).await else {
            return Ok(());
        };
        let metadata = Self::synthetic_metadata(package_id);
        for rule in policy.rules.iter().filter(|r| r.name() == "rbac") {
            let ctx = RuleContext {
                identity,
                package: &metadata,
                resource_type,
                cache_entry: None,
                requested_version: None,
            };
            if let RuleDecision::Deny { reason } = rule.evaluate(&ctx).await {
                return Err(CoreError::AccessDenied(reason));
            }
        }
        Ok(())
    }
    /// Authorise a listing read, filing a denial as its own audit event.
    ///
    /// A denial is recorded individually, with the identity, the coordinate and
    /// the reason. It is a security event that has to be inspectable one at a
    /// time, there are few of them, and an operator asking "who was refused,
    /// and why" needs the answer rather than a count.
    ///
    /// `what` names the document in the audit-write warning: the two callers
    /// serve different listing shapes, and a failed audit write should say
    /// which one it was.
    async fn authorize_listing_audited(
        &self,
        req: &ProxyRequest,
        what: &'static str,
    ) -> Result<(), CoreError> {
        let Err(e) = self
            .authorize_listing(&req.package_id, &req.identity, &req.resource_type)
            .await
        else {
            return Ok(());
        };
        if let CoreError::AccessDenied(reason) = &e {
            super::warn_if_audit_failed(
                self.repo
                    .record_access(AccessEvent::denied_metadata(
                        req.package_id.clone(),
                        req.identity.user_id.clone(),
                        req.identity.role.clone(),
                        reason.clone(),
                    ))
                    .await,
                what,
            );
        }
        Err(e)
    }

    /// Serve a proxied registry's version-listing document — for npm, the
    /// packument — with blocked versions removed and artifact URLs pointed back
    /// at this proxy.
    ///
    /// The two rewrites are what make the document *this* proxy's answer rather
    /// than a copy of the upstream's:
    ///
    /// - **Blocked versions are stripped** and `dist-tags.latest` recomputed, so
    ///   a resolver asking for `latest` or a range never selects a version the
    ///   operator has blocked. Without this the resolver picks the blocked
    ///   version from the upstream listing and the install fails at download —
    ///   the block reads as breakage rather than policy.
    /// - **`dist.tarball` is rewritten** to this proxy's own download route.
    ///   The upstream document points at the upstream CDN; served unchanged it
    ///   would route every download around the proxy, past its cache, its audit
    ///   trail and the download-time gate that is the block's other half.
    ///
    /// Only RBAC is evaluated here, not the whole rule chain. The chain judges a
    /// *concrete* version and still runs on the download that follows; applying
    /// it to the listing would deny the entire document because one version in
    /// it is gated, which is the opposite of letting a client resolve past that
    /// version to one it may have.
    pub async fn version_document(
        &self,
        req: &ProxyRequest,
        doc_kind: DocumentKind,
        public_base: &str,
    ) -> Result<VersionDocument, CoreError> {
        let prelude = self.request_prelude(req).await?;
        self.authorize_listing_audited(req, "denied version document")
            .await?;

        let name = req.package_id.name.as_str();
        let mut doc = self
            .cached_version_document(&prelude, req, name, doc_kind)
            .await?;

        let kind = prelude.client.registry_type().parse().unwrap_or_else(|_| {
            // Unreachable in practice: `registry_type()` returns the same
            // kebab-case string `RegistryKind` round-trips. Treating an unknown
            // one as `Generic` keeps the listing served and unfiltered, which is
            // the fail-open direction this whole path takes.
            tracing::warn!(
                registry_type = prelude.client.registry_type(),
                "registry client reports a type RegistryKind does not know; not filtering"
            );
            crate::entities::RegistryKind::Generic
        });
        let ctx = crate::services::blocking::ListingContext {
            registry: &req.package_id.registry,
            kind,
            document: doc_kind,
            package: name,
            public_base,
        };

        let blocked = self
            .blocked_versions_for(&req.package_id.registry, name, kind)
            .await;

        crate::services::blocking::dispatch(&ctx, &mut doc, &blocked);
        crate::services::blocking::rewrite_urls(&ctx, &mut doc);

        // An allowed listing is counted, not filed. `StatsRollupService` turns
        // this into one durable row per registry per hour, so a `cargo build`
        // over a 400-crate graph moves a counter 400 times and writes nothing.
        // What that gives up is per-package and per-identity attribution for
        // *allowed* listing reads; "who downloaded this artifact" and "who was
        // refused" both keep their own rows.
        self.metrics.record_listing_read(&req.package_id.registry);

        Ok(doc)
    }

    /// One package's blocked versions, normalised for its protocol, **failing
    /// open**.
    ///
    /// A repository error logs a warning and returns an empty set, matching
    /// `BlockListRule` and the local path's `filter_blocked`: a database blip
    /// should degrade to showing more versions than intended, never to
    /// reporting every package as empty. The download gate re-checks the
    /// concrete coordinate on every request and denies as soon as the store
    /// recovers, so no failure mode here makes blocked *bytes* retrievable.
    ///
    /// Public because JetBrains Marketplace renders three listing documents
    /// from one intermediate version list rather than from a fetched document,
    /// so its handler filters at that chokepoint instead of going through
    /// [`Self::version_document`].
    pub async fn blocked_versions_for(
        &self,
        registry: &str,
        package: &str,
        kind: crate::entities::RegistryKind,
    ) -> crate::services::blocking::BlockedVersions {
        let versions = self
            .repo
            .blocked_versions(registry, package)
            .await
            .unwrap_or_else(|e| {
                tracing::warn!(
                    registry = %registry,
                    package = %package,
                    error = %e,
                    "failed to load blocked versions for listing, failing open"
                );
                Vec::new()
            });
        crate::services::blocking::BlockedVersions::new(kind, versions)
    }

    /// The blocked `(package, version)` set for a whole registry, behind a
    /// short-lived snapshot.
    ///
    /// **The one place in this design where a block is not effective on the very
    /// next request.** Every other path reads the blocked set through on each
    /// request; this one cannot, because a multi-package index —
    /// `repodata.json` for a busy conda channel is tens of megabytes — is
    /// fetched on every `conda install`, and re-querying per request would put
    /// the whole channel's block list on that path.
    ///
    /// Thirty seconds is short enough that nobody waits on it during an
    /// incident and long enough to collapse a burst of installs into one query.
    /// The delay is documented in the admin guide rather than left to be
    /// discovered, because an undocumented delay is indistinguishable from a
    /// block that did not work.
    ///
    /// The snapshot lives in the metadata cache rather than in a process-local
    /// map, so a Redis-backed deployment shares one query across replicas
    /// instead of one per replica.
    /// The fingerprint of this registry's current blocked-set snapshot.
    ///
    /// For a caller that caches something *derived* from a filtered
    /// multi-package document — conda's compressed `repodata.json` — and needs
    /// the derived entry to change when the blocks do. Reads the same snapshot
    /// the filter uses, so the two cannot disagree about what is blocked.
    /// The registry-wide blocked set, for callers outside the listing path.
    ///
    /// Search needs it: a result page names many packages, so a per-package
    /// query would be one query per hit.
    pub async fn blocked_in_registry_snapshot_public(
        &self,
        registry: &str,
        kind: crate::entities::RegistryKind,
    ) -> crate::services::blocking::MultiPackageBlocks {
        self.blocked_in_registry_snapshot(registry, kind).await
    }

    pub async fn blocked_snapshot_fingerprint(
        &self,
        registry: &str,
        kind: crate::entities::RegistryKind,
    ) -> String {
        self.blocked_in_registry_snapshot(registry, kind)
            .await
            .fingerprint()
    }

    async fn blocked_in_registry_snapshot(
        &self,
        registry: &str,
        kind: crate::entities::RegistryKind,
    ) -> crate::services::blocking::MultiPackageBlocks {
        const SNAPSHOT_TTL: std::time::Duration = std::time::Duration::from_secs(30);
        let key = format!("blocks:{registry}");

        if let Ok(Some(entry)) = self.cache.get(&key).await {
            if let Ok(pairs) = serde_json::from_value::<Vec<(String, String)>>(entry.metadata.extra)
            {
                return crate::services::blocking::MultiPackageBlocks::new(kind, pairs);
            }
        }

        let pairs = match self.repo.blocked_in_registry(registry).await {
            Ok(p) => p,
            Err(e) => {
                // Fail open, as everywhere else on this path.
                tracing::warn!(
                    registry = %registry,
                    error = %e,
                    "failed to load the registry's blocked set, serving the index unfiltered"
                );
                return crate::services::blocking::MultiPackageBlocks::new(kind, Vec::new());
            }
        };

        let entry = crate::ports::CacheEntry {
            metadata: crate::entities::PackageMetadata {
                id: crate::entities::PackageId::new(registry, "__blocks__", "__snapshot__"),
                published_at: None,
                download_url: None,
                checksum: None,
                is_signed: None,
                extra: serde_json::to_value(&pairs).unwrap_or(serde_json::Value::Null),
                cache_control: None,
            },
            cached_at: chrono::Utc::now(),
            expires_at: None,
        };
        if let Err(e) = self.cache.set(&key, entry, Some(SNAPSHOT_TTL)).await {
            tracing::warn!(key = %key, error = %e, "caching the blocked-set snapshot failed");
        }

        crate::services::blocking::MultiPackageBlocks::new(kind, pairs)
    }

    /// Serve a **multi-package** index — conda's `repodata.json` — with blocked
    /// packages removed.
    ///
    /// The sibling of [`Self::version_document`] for the listings that describe
    /// a whole channel rather than one package. Same authorisation, same audit
    /// treatment, same fail-open; what differs is the shape of the blocked set
    /// (see [`Self::blocked_in_registry_snapshot`]) and therefore its freshness.
    pub async fn multi_package_document(
        &self,
        req: &ProxyRequest,
        doc_kind: DocumentKind,
        public_base: &str,
    ) -> Result<VersionDocument, CoreError> {
        let prelude = self.request_prelude(req).await?;
        self.authorize_listing_audited(req, "denied multi-package index")
            .await?;

        let name = req.package_id.name.as_str();
        let mut doc = self
            .cached_version_document(&prelude, req, name, doc_kind)
            .await?;

        let kind = prelude
            .client
            .registry_type()
            .parse()
            .unwrap_or(crate::entities::RegistryKind::Generic);
        let ctx = crate::services::blocking::ListingContext {
            registry: &req.package_id.registry,
            kind,
            document: doc_kind,
            package: name,
            public_base,
        };

        let blocked = self
            .blocked_in_registry_snapshot(&req.package_id.registry, kind)
            .await;
        crate::services::blocking::dispatch_multi(&ctx, &mut doc, &blocked);

        self.metrics.record_listing_read(&req.package_id.registry);
        Ok(doc)
    }

    /// The upstream version document, from the metadata cache when fresh.
    ///
    /// What is cached is the document **as the upstream sent it** — before
    /// blocks are applied and before tarball URLs are rewritten. Both of those
    /// must vary per request, and caching them would be wrong in two distinct
    /// ways:
    ///
    /// - A cached *filtered* document would keep serving a version for the rest
    ///   of the TTL after an operator blocked it. Blocks have to take effect on
    ///   the next request, not eventually.
    /// - A cached *rewritten* document would pin one ingress. The same registry
    ///   is reachable at `npm.acme.io` and at `hub.example.com/proxy/npm1`, and
    ///   whichever host warmed the cache would hand its own URLs to clients of
    ///   the other.
    ///
    /// On an upstream failure a stale entry is served when the registry's policy
    /// allows it, matching `resolve_metadata_cached`: an upstream outage should
    /// degrade to slightly old version lists, not to a broken registry.
    /// `pub(super)` so the console's discovery read can reuse the three rungs
    /// rather than inventing a second cache policy for the same document
    /// (RFC 0007 §5.5). Still not public: nothing outside `ProxyService` gets
    /// to fetch a listing document without going through a path that gates it.
    pub(super) async fn cached_version_document(
        &self,
        prelude: &RequestPrelude,
        req: &ProxyRequest,
        name: &str,
        doc_kind: DocumentKind,
    ) -> Result<VersionDocument, CoreError> {
        // Distinct from the `meta:` namespace: that key holds a `PackageMetadata`
        // for one version, this holds a whole package's upstream document.
        //
        // `doc_kind` is part of the key because a registry can have more than
        // one listing for the same name — NuGet's flat index and its
        // registration page, RubyGems' versions list and its gem document. Keyed
        // by name alone they collide, and one is served under the other's URL.
        let key = format!(
            "doc:{}:{}:{}",
            req.package_id.registry,
            doc_kind.as_str(),
            name
        );

        // `get` returns only entries the store still considers fresh, so freshness
        // is the store's job here exactly as it is in `resolve_metadata_cached` —
        // hence `expires_at: None` below rather than a second, independently
        // clocked expiry that could disagree with the backing store's own.
        if let Ok(Some(entry)) = self.cache.get(&key).await {
            if let Some(doc) = decode_cached_document(&key, entry.metadata.extra) {
                return Ok(doc);
            }
        }

        match prelude.client.fetch_version_document(name, doc_kind).await {
            Ok(doc) => {
                let encoded = serde_json::to_value(&doc).unwrap_or(serde_json::Value::Null);
                let entry = crate::ports::CacheEntry {
                    metadata: crate::entities::PackageMetadata {
                        id: req.package_id.clone(),
                        published_at: None,
                        download_url: None,
                        checksum: None,
                        is_signed: None,
                        extra: encoded,
                        cache_control: None,
                    },
                    cached_at: chrono::Utc::now(),
                    expires_at: None,
                };
                if let Err(e) = self.cache.set(&key, entry, prelude.ttl).await {
                    tracing::warn!(key = %key, error = %e, "caching version document failed");
                }
                Ok(doc)
            }
            Err(e) => {
                let serve_stale = prelude
                    .policy
                    .as_ref()
                    .map(|p| p.serve_stale_metadata)
                    .unwrap_or(false);
                if serve_stale {
                    if let Ok(Some(stale)) = self.cache.get_stale(&key).await {
                        if let Some(doc) = decode_cached_document(&key, stale.metadata.extra) {
                            tracing::warn!(
                                key = %key,
                                error = %e,
                                "upstream version document unavailable, serving stale"
                            );
                            return Ok(doc);
                        }
                    }
                }
                Err(e)
            }
        }
    }
}

/// Read a cached [`VersionDocument`] back out of the metadata cache's untyped
/// `extra` field.
///
/// `None` on anything that does not deserialize, which is treated as a miss.
/// The realistic cause is an entry written by an older build under a key shape
/// this one reuses; refetching is cheap and correct, where trusting a partially
/// understood document is neither.
fn decode_cached_document(key: &str, extra: serde_json::Value) -> Option<VersionDocument> {
    match serde_json::from_value(extra) {
        Ok(doc) => Some(doc),
        Err(e) => {
            tracing::debug!(key = %key, error = %e, "cached version document unreadable, refetching");
            None
        }
    }
}
