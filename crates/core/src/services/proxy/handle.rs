use std::sync::Arc;
use std::time::Instant;

use crate::entities::AccessEvent;
use crate::error::CoreError;
use crate::rules::{evaluate_rules, RuleContext, RuleDecision};

use super::{ProxyRequest, ProxyResponse, ProxyService};

/// Largest artifact that re-serve verification (`verify_on_serve`) will retain in
/// memory so it can be hashed and served from the same buffer in a single read.
/// Artifacts above this size are hashed by streaming (memory stays bounded) and
/// then re-opened from storage to serve. 32 MiB comfortably covers typical
/// package artifacts (npm tarballs, wheels, crates) while capping per-request
/// memory for pathologically large ones.
pub(crate) const RESERVE_VERIFY_BUFFER_LIMIT: usize = 32 * 1024 * 1024;

impl ProxyService {
    pub async fn handle(&self, req: ProxyRequest) -> Result<ProxyResponse, CoreError> {
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
        // Arc<str> instead of String: every downstream metrics call below clones
        // this cheaply (atomic refcount bump) instead of copying the registry
        // name's bytes on every `counter!`/`histogram!` invocation.
        let registry_label: Arc<str> = Arc::from(registry_name);
        let start = Instant::now();

        // Acquire the read lock briefly to clone the Arc<RegistryClient> and
        // Arc<RegistryPolicy>. The lock is released before any async I/O begins.
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

        // ── 1. Resolve metadata (cache-first) ─────────────────────────────────
        let cache_key = format!("meta:{}", req.package_id.cache_key());
        let ttl = policy.as_ref().and_then(|p| p.metadata_ttl);
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
            let mut upstream = match client.fetch_artifact(&req.package_id).await {
                Ok(s) => s,
                Err(e) => {
                    super::record_upstream_duration(
                        &registry_label,
                        "fetch_artifact",
                        upstream_start,
                    );
                    super::warn_if_audit_failed(
                        self.repo
                            .record_access(AccessEvent::proxy_error(
                                req.package_id.clone(),
                                req.identity.user_id.clone(),
                                req.identity.role.clone(),
                                e.to_string(),
                            ))
                            .await,
                        "proxy error",
                    );
                    return Err(e);
                }
            };
            // Times the whole body transfer, not just time-to-headers — this is the
            // only latency signal firewall-only registries get, since they never hit
            // the artifact cache path.
            upstream.stream = super::time_upstream_stream(
                Arc::clone(&registry_label),
                "fetch_artifact",
                upstream_start,
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
            return self
                .serve_cache_hit(req, artifact_key, &integrity, registry_label, start)
                .await;
        }

        // ── 5b. Cache miss: fetch + cache (see `cache::fetch_and_cache`) ───────
        self.fetch_and_cache(
            req,
            client,
            metadata,
            artifact_key,
            &integrity,
            limit,
            registry_label,
            start,
        )
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
