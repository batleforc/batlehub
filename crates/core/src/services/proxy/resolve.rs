use std::sync::Arc;

use chrono::Utc;

use crate::entities::SbomFormat;
use crate::error::CoreError;
use crate::ports::CacheEntry;
use crate::services::cache_control::parse_cache_control;
use crate::services::sbom::SbomProxiedOptions;

use super::{ProxyRequest, ProxyService};

impl ProxyService {
    /// Resolves metadata from cache (hit) or upstream (miss/stale).
    pub(super) async fn resolve_metadata_cached(
        &self,
        client: &Arc<dyn crate::ports::RegistryClient>,
        policy: &Option<Arc<crate::services::hot_config::RegistryPolicy>>,
        req: &ProxyRequest,
        cache_key: &str,
        ttl: Option<std::time::Duration>,
        registry_label: &Arc<str>,
    ) -> Result<crate::entities::PackageMetadata, CoreError> {
        if let Some(entry) = self.cache.get(cache_key).await? {
            tracing::debug!(key = %cache_key, "metadata cache hit");
            metrics::counter!("batlehub_metadata_cache_hits_total", "registry" => Arc::clone(registry_label)).increment(1);
            return Ok(entry.metadata);
        }
        tracing::debug!(key = %cache_key, "metadata cache miss, fetching from upstream");
        metrics::counter!("batlehub_metadata_cache_misses_total", "registry" => Arc::clone(registry_label)).increment(1);
        let meta = match super::time_upstream_call(
            registry_label,
            "resolve_metadata",
            &self.metrics,
            client.resolve_metadata(&req.package_id),
        )
        .await
        {
            Ok(m) => {
                self.metrics.record_upstream_outcome(registry_label, true);
                m
            }
            Err(e) => {
                self.metrics.record_upstream_outcome(registry_label, false);
                let serve_stale = policy
                    .as_ref()
                    .map(|p| p.serve_stale_metadata)
                    .unwrap_or(false);
                if serve_stale && matches!(e, CoreError::Registry(_)) {
                    if let Some(stale) = self.cache.get_stale(cache_key).await? {
                        tracing::warn!(key = %cache_key, error = %e, "upstream unavailable; serving stale metadata");
                        return Ok(stale.metadata);
                    }
                }
                metrics::counter!("batlehub_upstream_errors_total", "registry" => Arc::clone(registry_label)).increment(1);
                super::warn_if_audit_failed(
                    self.repo
                        .record_access(crate::entities::AccessEvent::proxy_error(
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
        let skip = meta
            .cache_control
            .as_deref()
            .map(|h| parse_cache_control(h).no_store)
            .unwrap_or(false);
        if !skip {
            self.cache
                .set(
                    cache_key,
                    CacheEntry {
                        metadata: meta.clone(),
                        cached_at: Utc::now(),
                        expires_at: None,
                    },
                    ttl,
                )
                .await?;
        }
        Ok(meta)
    }

    /// Returns `true` if a cached artifact exists and has not yet exceeded its TTL.
    pub(super) async fn artifact_is_fresh(
        &self,
        artifact_key: &str,
        artifact_ttl: Option<std::time::Duration>,
        registry_name: &str,
    ) -> Result<bool, CoreError> {
        if !self.storage.exists(artifact_key).await? {
            return Ok(false);
        }
        let Some(ttl) = artifact_ttl else {
            return Ok(true);
        };
        match chrono::Duration::from_std(ttl) {
            Ok(d) => {
                let expired = self
                    .artifact_meta
                    .is_artifact_expired(artifact_key, Utc::now() - d)
                    .await?;
                Ok(!expired)
            }
            Err(e) => {
                tracing::warn!(registry = %registry_name, error = %e, "artifact_ttl overflows chrono::Duration; treating artifact as fresh");
                Ok(true)
            }
        }
    }

    /// Spawns SBOM generation for a freshly cached artifact (non-blocking, non-fatal).
    ///
    /// The artifact bytes are re-read from storage **inside the spawned task**
    /// rather than passed in, so the request hot path never holds the full
    /// artifact in memory — the buffering cost is paid only when SBOM is enabled
    /// for the registry, and off the critical path.
    pub(super) async fn maybe_trigger_sbom(
        &self,
        registry_name: &str,
        artifact_key: &str,
        metadata: &crate::entities::PackageMetadata,
        registry_type: &str,
    ) {
        let Some(ref sbom_svc) = self.sbom else {
            return;
        };
        let sbom_cfg = {
            let hot = self.hot.read().await;
            hot.sbom.get(registry_name).cloned()
        };
        let Some(cfg) = sbom_cfg.filter(|c| c.enabled) else {
            return;
        };
        let sbom = Arc::clone(sbom_svc);
        let storage = Arc::clone(&self.storage);
        let meta_clone = metadata.clone();
        let key_clone = artifact_key.to_owned();
        let registry_type = registry_type.to_owned();
        let formats: Vec<SbomFormat> = cfg
            .formats
            .iter()
            .filter_map(|s| SbomFormat::parse(s))
            .collect();
        tokio::spawn(async move {
            // Pull the just-stored bytes back from storage for manifest extraction.
            let data = match storage.retrieve(&key_clone).await {
                Ok(Some(artifact)) => {
                    match crate::ports::collect_byte_stream(artifact.stream).await {
                        Ok(bytes) => bytes,
                        Err(e) => {
                            tracing::warn!(key = %key_clone, error = %e, "sbom: failed to read cached artifact (non-fatal)");
                            return;
                        }
                    }
                }
                Ok(None) => {
                    tracing::warn!(key = %key_clone, "sbom: cached artifact vanished before generation (non-fatal)");
                    return;
                }
                Err(e) => {
                    tracing::warn!(key = %key_clone, error = %e, "sbom: storage retrieve failed (non-fatal)");
                    return;
                }
            };
            if let Err(e) = sbom
                .record_for_proxied(
                    &meta_clone,
                    &key_clone,
                    &data,
                    SbomProxiedOptions {
                        registry_type: &registry_type,
                        formats: &formats,
                        fetch_upstream: cfg.fetch_upstream,
                    },
                )
                .await
            {
                tracing::warn!(key = %key_clone, error = %e, "sbom generation failed (non-fatal)");
            }
        });
    }
}
