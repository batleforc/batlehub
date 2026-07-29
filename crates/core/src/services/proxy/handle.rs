use std::sync::Arc;
use std::time::Instant;

use crate::entities::AccessEvent;
use crate::error::CoreError;
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
        let policy = {
            let hot = self.hot.read().await;
            hot.policies.get(package_id.registry.as_str()).cloned()
        };
        let empty: Vec<Box<dyn crate::rules::Rule>> = vec![];
        let rules = policy
            .as_ref()
            .map(|p| p.rules.as_slice())
            .unwrap_or(empty.as_slice());

        // Minimal metadata: deb/rpm files have no per-version upstream metadata,
        // and the RBAC rule keys only off the identity. (The proxy fall-through
        // evaluates the same rule set against the same synthetic coordinate.)
        let metadata = crate::entities::PackageMetadata {
            id: package_id.clone(),
            published_at: None,
            download_url: None,
            checksum: None,
            is_signed: None,
            extra: serde_json::Value::Null,
            cache_control: None,
        };
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
}
