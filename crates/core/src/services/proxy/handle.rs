use std::sync::Arc;
use std::time::Instant;

use bytes::Bytes;
use futures::StreamExt;

use crate::entities::AccessEvent;
use crate::error::CoreError;
use crate::ports::StorageMeta;
use crate::rules::{evaluate_rules, RuleContext, RuleDecision};
use crate::services::cache_control::parse_cache_control;

use super::{ProxyRequest, ProxyResponse, ProxyService};

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
        let registry_label = registry_name.to_owned();
        let start = Instant::now();

        // Acquire the read lock briefly to clone the Arc<RegistryClient> and
        // Arc<RegistryPolicy>. The lock is released before any async I/O begins.
        let (client, policy, limit) = {
            let hot = self.hot.read().await;
            let client = hot
                .registries
                .get(registry_name)
                .ok_or_else(|| CoreError::UnknownRegistry(registry_name.to_owned()))?
                .clone();
            let policy = hot.policies.get(registry_name).cloned();
            let limit = hot.max_artifact_size_bytes.unwrap_or(500 * 1024 * 1024);
            (client, policy, limit)
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
            metrics::counter!("batlehub_requests_total", "registry" => registry_label.clone(), "outcome" => "denied").increment(1);
            metrics::histogram!("batlehub_request_duration_seconds", "registry" => registry_label.clone()).record(start.elapsed().as_secs_f64());
            return Ok(ProxyResponse::Denied { reason });
        }

        // ── 3. Firewall-only: stream directly from upstream, skip all caching ──
        let firewall_only = policy.as_ref().map(|p| p.firewall_only).unwrap_or(false);

        if firewall_only {
            tracing::debug!(registry = %registry_name, "firewall-only mode, streaming from upstream");
            let upstream = match client.fetch_artifact(&req.package_id).await {
                Ok(s) => s,
                Err(e) => {
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
            metrics::counter!("batlehub_requests_total", "registry" => registry_label.clone(), "outcome" => "allowed").increment(1);
            metrics::histogram!("batlehub_request_duration_seconds", "registry" => registry_label.clone()).record(start.elapsed().as_secs_f64());
            return Ok(ProxyResponse::Stream(upstream.stream));
        }

        // ── 4. Check artifact cache ────────────────────────────────────────────
        let artifact_key = format!("artifact:{}", req.package_id.cache_key());
        let artifact_ttl = policy.as_ref().and_then(|p| p.artifact_ttl);
        let cached_artifact_is_fresh = self
            .artifact_is_fresh(&artifact_key, artifact_ttl, registry_name)
            .await?;

        if cached_artifact_is_fresh {
            tracing::debug!(key = %artifact_key, "artifact cache hit");
            metrics::counter!("batlehub_artifact_cache_hits_total", "registry" => registry_label.clone()).increment(1);
            self.metrics.record_artifact_hit(registry_name);
            let artifact = self.storage.retrieve(&artifact_key).await?.ok_or_else(|| {
                CoreError::Registry(format!(
                    "artifact '{artifact_key}' vanished between exists and retrieve"
                ))
            })?;

            let meta_repo = Arc::clone(&self.artifact_meta);
            let key_clone = artifact_key.clone();
            tokio::spawn(async move {
                if let Err(e) = meta_repo.touch_artifact(&key_clone).await {
                    tracing::warn!(key = %key_clone, error = %e, "touch_artifact failed");
                }
            });

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
            metrics::counter!("batlehub_requests_total", "registry" => registry_label.clone(), "outcome" => "allowed").increment(1);
            metrics::histogram!("batlehub_request_duration_seconds", "registry" => registry_label.clone()).record(start.elapsed().as_secs_f64());
            return Ok(ProxyResponse::Stream(artifact.stream));
        }

        // ── 5. Fetch from upstream and (conditionally) cache ──────────────────
        tracing::debug!(key = %artifact_key, "artifact not cached, fetching from upstream");
        metrics::counter!("batlehub_artifact_cache_misses_total", "registry" => registry_label.clone()).increment(1);
        self.metrics.record_artifact_miss(registry_name);
        let mut upstream = match client.fetch_artifact(&req.package_id).await {
            Ok(s) => s,
            Err(e) => {
                metrics::counter!("batlehub_upstream_errors_total", "registry" => registry_label.clone()).increment(1);
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

        let skip_artifact_cache = upstream
            .cache_control
            .as_deref()
            .map(|h| parse_cache_control(h).no_store)
            .unwrap_or(false);

        let mut buf: Vec<u8> = Vec::new();
        while let Some(chunk) = upstream.stream.next().await {
            let chunk = chunk?;
            if buf.len() as u64 + chunk.len() as u64 > limit {
                return Err(CoreError::PayloadTooLarge(format!(
                    "artifact exceeds the {} byte limit",
                    limit
                )));
            }
            buf.extend_from_slice(&chunk);
        }
        let data = Bytes::from(buf);

        if !skip_artifact_cache {
            self.storage
                .store(
                    &artifact_key,
                    data.clone(),
                    StorageMeta {
                        size: Some(data.len() as u64),
                        ..Default::default()
                    },
                )
                .await?;

            if let Err(e) = self
                .artifact_meta
                .record_artifact(
                    &artifact_key,
                    registry_name,
                    &req.package_id.name,
                    &req.package_id.version,
                    Some(data.len() as u64),
                )
                .await
            {
                tracing::warn!(key = %artifact_key, error = %e, "record_artifact failed");
            }

            self.maybe_trigger_sbom(
                registry_name,
                &artifact_key,
                &data,
                &metadata,
                client.registry_type(),
            )
            .await;
        } else {
            tracing::debug!(key = %artifact_key, "upstream Cache-Control: no-store; skipping artifact cache");
        }

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
        metrics::counter!("batlehub_requests_total", "registry" => registry_label.clone(), "outcome" => "allowed").increment(1);
        metrics::histogram!("batlehub_request_duration_seconds", "registry" => registry_label)
            .record(start.elapsed().as_secs_f64());

        let stream = futures::stream::once(async move { Ok(data) });
        Ok(ProxyResponse::Stream(Box::pin(stream)))
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
