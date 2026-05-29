use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use chrono::Utc;
use futures::StreamExt;

use crate::entities::{AccessEvent, Identity, PackageId};
use crate::error::CoreError;
use crate::ports::{
    ArtifactMetaRepository, ArtifactStream, CacheEntry, CacheStore, PackageRepository,
    RegistryClient, StorageBackend, StorageMeta,
};
use crate::rules::{evaluate_rules, Rule, RuleContext, RuleDecision};
use crate::services::cache_control::parse_cache_control;
use crate::services::metrics::ProxyMetrics;

/// Per-registry behaviour configuration wired in at startup.
pub struct RegistryPolicy {
    pub metadata_ttl: Option<Duration>,
    /// Rules evaluated in order for every request to this registry.
    pub rules: Vec<Box<dyn Rule>>,
    /// When `true`, skip artifact storage entirely and stream directly from upstream.
    pub firewall_only: bool,
    /// When `true`, serve stale (expired) cached metadata if upstream returns a transient
    /// `Registry` error. Allows cached artifacts to keep being served during outages.
    pub serve_stale_metadata: bool,
    /// When set, artifacts are re-fetched from upstream after this duration even if
    /// present in storage. Implements TTL-based artifact expiry at request time.
    pub artifact_ttl: Option<Duration>,
}

/// Input to `ProxyService::handle`.
pub struct ProxyRequest {
    pub package_id: PackageId,
    pub identity: Identity,
    /// The operation being checked against RBAC (e.g. `"releases:read"`).
    pub resource_type: String,
}

/// Output of `ProxyService::handle`.
pub enum ProxyResponse {
    /// Artifact stream to forward to the HTTP client.
    Stream(ArtifactStream),
    /// Access was denied; the caller should receive a 403.
    Denied { reason: String },
}

/// Caching proxy service: resolves metadata, evaluates rules, streams artifacts.
pub struct ProxyService {
    pub registries: HashMap<String, Arc<dyn RegistryClient>>,
    pub storage: Arc<dyn StorageBackend>,
    pub cache: Arc<dyn CacheStore>,
    pub repo: Arc<dyn PackageRepository>,
    pub artifact_meta: Arc<dyn ArtifactMetaRepository>,
    pub policies: HashMap<String, RegistryPolicy>,
    /// Maximum artifact size allowed when buffering from upstream before writing
    /// to storage. Requests that exceed this limit return a 413 error rather than
    /// exhausting server memory. Defaults to 500 MiB when `None`.
    pub max_artifact_size_bytes: Option<u64>,
    /// In-memory counters for the stats dashboard (reset on restart).
    pub metrics: Arc<ProxyMetrics>,
}

fn warn_if_audit_failed(r: Result<(), CoreError>, ctx: &str) {
    if let Err(e) = r {
        tracing::warn!(error = %e, ctx, "audit log write failed");
    }
}

impl ProxyService {
    pub async fn handle(&self, req: ProxyRequest) -> Result<ProxyResponse, CoreError> {
        let registry_name: &str = req.package_id.registry.as_str();
        let registry_label = registry_name.to_owned();
        let start = Instant::now();

        let client = self
            .registries
            .get(registry_name)
            .ok_or_else(|| CoreError::UnknownRegistry(registry_name.to_owned()))?;

        // ── 1. Resolve metadata (cache-first) ─────────────────────────────────
        let cache_key = format!("meta:{}", req.package_id.cache_key());
        let ttl = self
            .policies
            .get(registry_name)
            .and_then(|p| p.metadata_ttl);

        let metadata = if let Some(entry) = self.cache.get(&cache_key).await? {
            tracing::debug!(key = %cache_key, "metadata cache hit");
            metrics::counter!("batlehub_metadata_cache_hits_total", "registry" => registry_label.clone()).increment(1);
            entry.metadata
        } else {
            tracing::debug!(key = %cache_key, "metadata cache miss, fetching from upstream");
            metrics::counter!("batlehub_metadata_cache_misses_total", "registry" => registry_label.clone()).increment(1);
            let meta = match client.resolve_metadata(&req.package_id).await {
                Ok(m) => m,
                Err(e) => {
                    let serve_stale = self
                        .policies
                        .get(registry_name)
                        .map(|p| p.serve_stale_metadata)
                        .unwrap_or(false);

                    if serve_stale && matches!(e, CoreError::Registry(_)) {
                        match self.cache.get_stale(&cache_key).await? {
                            Some(stale) => {
                                tracing::warn!(
                                    key = %cache_key,
                                    error = %e,
                                    "upstream unavailable; serving stale metadata"
                                );
                                stale.metadata
                            }
                            None => {
                                metrics::counter!("batlehub_upstream_errors_total", "registry" => registry_label.clone()).increment(1);
                                warn_if_audit_failed(
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
                        }
                    } else {
                        metrics::counter!("batlehub_upstream_errors_total", "registry" => registry_label.clone()).increment(1);
                        warn_if_audit_failed(
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
                }
            };
            // Honour upstream Cache-Control: skip metadata caching on no-store.
            let skip_meta_cache = meta
                .cache_control
                .as_deref()
                .map(|h| parse_cache_control(h).no_store)
                .unwrap_or(false);

            if !skip_meta_cache {
                self.cache
                    .set(
                        &cache_key,
                        CacheEntry {
                            metadata: meta.clone(),
                            cached_at: Utc::now(),
                            expires_at: None,
                        },
                        ttl,
                    )
                    .await?;
            }
            meta
        };

        // ── 2. Evaluate rules ──────────────────────────────────────────────────
        let rules = self
            .policies
            .get(registry_name)
            .map(|p| p.rules.as_slice())
            .unwrap_or(&[]);

        let ctx = RuleContext {
            identity: &req.identity,
            package: &metadata,
            resource_type: &req.resource_type,
            cache_entry: None,
            requested_version: Some(&req.package_id.version),
        };

        if let RuleDecision::Deny { reason } = evaluate_rules(rules, &ctx).await {
            warn_if_audit_failed(
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
        let firewall_only = self
            .policies
            .get(registry_name)
            .map(|p| p.firewall_only)
            .unwrap_or(false);

        if firewall_only {
            tracing::debug!(registry = %registry_name, "firewall-only mode, streaming from upstream");
            let upstream = match client.fetch_artifact(&req.package_id).await {
                Ok(s) => s,
                Err(e) => {
                    warn_if_audit_failed(
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
            warn_if_audit_failed(
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

        let artifact_ttl = self
            .policies
            .get(registry_name)
            .and_then(|p| p.artifact_ttl);
        let cached_artifact_is_fresh = if self.storage.exists(&artifact_key).await? {
            // When an artifact TTL is set, do a point-lookup for this specific key.
            if let Some(ttl) = artifact_ttl {
                match chrono::Duration::from_std(ttl) {
                    Ok(d) => {
                        let expired = self
                            .artifact_meta
                            .is_artifact_expired(&artifact_key, Utc::now() - d)
                            .await?;
                        !expired
                    }
                    Err(e) => {
                        // TTL is larger than chrono's range (≥292 years); treat as "never expire".
                        tracing::warn!(registry = %registry_name, error = %e, "artifact_ttl overflows chrono::Duration; treating artifact as fresh");
                        true
                    }
                }
            } else {
                true
            }
        } else {
            false
        };

        if cached_artifact_is_fresh {
            tracing::debug!(key = %artifact_key, "artifact cache hit");
            metrics::counter!("batlehub_artifact_cache_hits_total", "registry" => registry_label.clone()).increment(1);
            self.metrics.record_artifact_hit(registry_name);
            let artifact = self.storage.retrieve(&artifact_key).await?.ok_or_else(|| {
                CoreError::Registry(format!(
                    "artifact '{artifact_key}' vanished between exists and retrieve"
                ))
            })?;

            // Update last-accessed timestamp without blocking the response.
            let meta_repo = Arc::clone(&self.artifact_meta);
            let key_clone = artifact_key.clone();
            tokio::spawn(async move {
                if let Err(e) = meta_repo.touch_artifact(&key_clone).await {
                    tracing::warn!(key = %key_clone, error = %e, "touch_artifact failed");
                }
            });

            warn_if_audit_failed(
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
                warn_if_audit_failed(
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

        // Honour upstream Cache-Control: no-store means we must not persist the artifact.
        let skip_artifact_cache = upstream
            .cache_control
            .as_deref()
            .map(|h| parse_cache_control(h).no_store)
            .unwrap_or(false);

        let limit = self.max_artifact_size_bytes.unwrap_or(500 * 1024 * 1024);
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

            // Record artifact metadata for eviction tracking.
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
        } else {
            tracing::debug!(key = %artifact_key, "upstream Cache-Control: no-store; skipping artifact cache");
        }

        warn_if_audit_failed(
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
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use bytes::Bytes;
    use chrono::Utc;
    use futures::stream;

    use super::*;
    use crate::entities::{
        AccessEvent, AccessResult, EventFilter, Identity, PackageFilter, PackageId,
        PackageMetadata, PackageStatus, PackageSummary,
    };
    use crate::ports::ByteStream;
    use crate::ports::{
        ArtifactMeta, ArtifactMetaRepository, CacheStore, FetchedArtifact, PackageRepository,
        RegistryClient, StorageBackend, StorageMeta, StoredArtifact,
    };
    use crate::services::metrics::ProxyMetrics;

    // ── Minimal in-memory mocks ───────────────────────────────────────────────

    struct NoopArtifactMeta;
    impl NoopArtifactMeta {
        fn arc() -> Arc<dyn ArtifactMetaRepository> {
            Arc::new(Self)
        }
    }
    #[async_trait]
    impl ArtifactMetaRepository for NoopArtifactMeta {
        async fn record_artifact(
            &self,
            _key: &str,
            _registry: &str,
            _name: &str,
            _ver: &str,
            _size: Option<u64>,
        ) -> Result<(), CoreError> {
            Ok(())
        }
        async fn touch_artifact(&self, _key: &str) -> Result<(), CoreError> {
            Ok(())
        }
        async fn list_artifacts(&self, _registry: &str) -> Result<Vec<ArtifactMeta>, CoreError> {
            Ok(vec![])
        }
        async fn list_artifacts_by_package(&self) -> Result<Vec<ArtifactMeta>, CoreError> {
            Ok(vec![])
        }
        async fn delete_artifact_meta(&self, _key: &str) -> Result<(), CoreError> {
            Ok(())
        }
        async fn is_artifact_expired(
            &self,
            _key: &str,
            _older_than: chrono::DateTime<chrono::Utc>,
        ) -> Result<bool, CoreError> {
            Ok(false)
        }
        async fn list_expired_by_ttl(
            &self,
            _registry: &str,
            _older_than: chrono::DateTime<chrono::Utc>,
        ) -> Result<Vec<ArtifactMeta>, CoreError> {
            Ok(vec![])
        }
        async fn list_idle(
            &self,
            _registry: &str,
            _idle_since: chrono::DateTime<chrono::Utc>,
        ) -> Result<Vec<ArtifactMeta>, CoreError> {
            Ok(vec![])
        }
        async fn total_size_bytes(&self, _registry: &str) -> Result<u64, CoreError> {
            Ok(0)
        }
        async fn list_lru(
            &self,
            _registry: &str,
            _limit: i64,
        ) -> Result<Vec<ArtifactMeta>, CoreError> {
            Ok(vec![])
        }
    }

    /// Records `record_artifact` and `touch_artifact` calls; returns configurable
    /// expired-artifact list from `list_expired_by_ttl`.
    struct SpyArtifactMeta {
        recorded: Mutex<Vec<String>>,
        touched: Mutex<Vec<String>>,
        expired: Mutex<Vec<ArtifactMeta>>,
    }
    impl SpyArtifactMeta {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                recorded: Mutex::new(vec![]),
                touched: Mutex::new(vec![]),
                expired: Mutex::new(vec![]),
            })
        }
        fn with_expired(expired: Vec<ArtifactMeta>) -> Arc<Self> {
            Arc::new(Self {
                recorded: Mutex::new(vec![]),
                touched: Mutex::new(vec![]),
                expired: Mutex::new(expired),
            })
        }
        fn recorded_keys(&self) -> Vec<String> {
            self.recorded.lock().unwrap().clone()
        }
        fn touched_keys(&self) -> Vec<String> {
            self.touched.lock().unwrap().clone()
        }
    }
    #[async_trait]
    impl ArtifactMetaRepository for SpyArtifactMeta {
        async fn record_artifact(
            &self,
            key: &str,
            _: &str,
            _: &str,
            _: &str,
            _: Option<u64>,
        ) -> Result<(), CoreError> {
            self.recorded.lock().unwrap().push(key.to_owned());
            Ok(())
        }
        async fn touch_artifact(&self, key: &str) -> Result<(), CoreError> {
            self.touched.lock().unwrap().push(key.to_owned());
            Ok(())
        }
        async fn list_artifacts(&self, _: &str) -> Result<Vec<ArtifactMeta>, CoreError> {
            Ok(vec![])
        }
        async fn list_artifacts_by_package(&self) -> Result<Vec<ArtifactMeta>, CoreError> {
            Ok(vec![])
        }
        async fn delete_artifact_meta(&self, _: &str) -> Result<(), CoreError> {
            Ok(())
        }
        async fn is_artifact_expired(
            &self,
            key: &str,
            older_than: chrono::DateTime<chrono::Utc>,
        ) -> Result<bool, CoreError> {
            // If the key has been "recorded" (has metadata) and is NOT explicitly in the
            // expired list, treat it as fresh.  If no metadata has been recorded, treat
            // it as expired (matches PgArtifactMetaRepository semantics: missing row → expired).
            let recorded = self.recorded.lock().unwrap();
            let expired = self.expired.lock().unwrap();
            let has_meta =
                recorded.contains(&key.to_owned()) || expired.iter().any(|m| m.artifact_key == key);
            if !has_meta {
                return Ok(true);
            }
            let is_expired = expired
                .iter()
                .any(|m| m.artifact_key == key && m.cached_at < older_than);
            Ok(is_expired)
        }
        async fn list_expired_by_ttl(
            &self,
            _: &str,
            _: chrono::DateTime<chrono::Utc>,
        ) -> Result<Vec<ArtifactMeta>, CoreError> {
            Ok(self.expired.lock().unwrap().clone())
        }
        async fn list_idle(
            &self,
            _: &str,
            _: chrono::DateTime<chrono::Utc>,
        ) -> Result<Vec<ArtifactMeta>, CoreError> {
            Ok(vec![])
        }
        async fn total_size_bytes(&self, _: &str) -> Result<u64, CoreError> {
            Ok(0)
        }
        async fn list_lru(&self, _: &str, _: i64) -> Result<Vec<ArtifactMeta>, CoreError> {
            Ok(vec![])
        }
    }

    struct TestCacheStore {
        data: Mutex<HashMap<String, crate::ports::CacheEntry>>,
    }

    impl TestCacheStore {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                data: Mutex::new(HashMap::new()),
            })
        }

        fn seed_expired(&self, key: &str, metadata: PackageMetadata) {
            let entry = crate::ports::CacheEntry {
                metadata,
                cached_at: Utc::now() - chrono::Duration::hours(2),
                expires_at: Some(Utc::now() - chrono::Duration::hours(1)),
            };
            self.data.lock().unwrap().insert(key.to_owned(), entry);
        }
    }

    #[async_trait]
    impl CacheStore for TestCacheStore {
        async fn get(&self, key: &str) -> Result<Option<crate::ports::CacheEntry>, CoreError> {
            let map = self.data.lock().unwrap();
            Ok(map.get(key).filter(|e| !e.is_expired()).cloned())
        }
        async fn set(
            &self,
            key: &str,
            mut entry: crate::ports::CacheEntry,
            ttl: Option<std::time::Duration>,
        ) -> Result<(), CoreError> {
            if let Some(ttl) = ttl {
                entry.expires_at =
                    Some(Utc::now() + chrono::Duration::from_std(ttl).unwrap_or_default());
            }
            self.data.lock().unwrap().insert(key.to_owned(), entry);
            Ok(())
        }
        async fn invalidate(&self, key: &str) -> Result<(), CoreError> {
            self.data.lock().unwrap().remove(key);
            Ok(())
        }
        async fn get_stale(
            &self,
            key: &str,
        ) -> Result<Option<crate::ports::CacheEntry>, CoreError> {
            Ok(self.data.lock().unwrap().get(key).cloned())
        }
    }

    struct SpyRepo {
        events: Mutex<Vec<AccessEvent>>,
    }

    impl SpyRepo {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                events: Mutex::new(vec![]),
            })
        }

        fn events(&self) -> Vec<AccessEvent> {
            self.events.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl PackageRepository for SpyRepo {
        async fn record_access(&self, event: AccessEvent) -> Result<(), CoreError> {
            self.events.lock().unwrap().push(event);
            Ok(())
        }
        async fn get_status(&self, _pkg: &PackageId) -> Result<PackageStatus, CoreError> {
            Ok(PackageStatus::Available)
        }
        async fn set_status(
            &self,
            _pkg: &PackageId,
            _status: PackageStatus,
        ) -> Result<(), CoreError> {
            Ok(())
        }
        async fn list_packages(
            &self,
            _filter: PackageFilter,
        ) -> Result<Vec<PackageSummary>, CoreError> {
            Ok(vec![])
        }
        async fn count_packages(&self, _filter: PackageFilter) -> Result<u64, CoreError> {
            Ok(0)
        }
        async fn list_events(&self, _filter: EventFilter) -> Result<Vec<AccessEvent>, CoreError> {
            Ok(self.events.lock().unwrap().clone())
        }
    }

    struct MemStorage {
        data: Mutex<HashMap<String, Bytes>>,
    }

    impl MemStorage {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                data: Mutex::new(HashMap::new()),
            })
        }
    }

    #[async_trait]
    impl StorageBackend for MemStorage {
        async fn store(&self, key: &str, data: Bytes, _meta: StorageMeta) -> Result<(), CoreError> {
            self.data.lock().unwrap().insert(key.to_owned(), data);
            Ok(())
        }
        async fn retrieve(&self, key: &str) -> Result<Option<StoredArtifact>, CoreError> {
            let lock = self.data.lock().unwrap();
            Ok(lock.get(key).map(|bytes| {
                let b = bytes.clone();
                let s: ByteStream =
                    Box::pin(stream::once(async move { Ok::<Bytes, CoreError>(b) }));
                StoredArtifact {
                    stream: s,
                    meta: StorageMeta::default(),
                }
            }))
        }
        async fn exists(&self, key: &str) -> Result<bool, CoreError> {
            Ok(self.data.lock().unwrap().contains_key(key))
        }
        async fn delete(&self, key: &str) -> Result<(), CoreError> {
            self.data.lock().unwrap().remove(key);
            Ok(())
        }
        async fn delete_by_prefix(&self, prefix: &str) -> Result<usize, CoreError> {
            let mut map = self.data.lock().unwrap();
            let keys: Vec<String> = map
                .keys()
                .filter(|k| k.starts_with(prefix))
                .cloned()
                .collect();
            let count = keys.len();
            for k in keys {
                map.remove(&k);
            }
            Ok(count)
        }
        async fn stat_by_prefix(&self, prefix: &str) -> Result<(u64, u64), CoreError> {
            let map = self.data.lock().unwrap();
            let (count, bytes) = map
                .iter()
                .filter(|(k, _)| k.starts_with(prefix))
                .fold((0u64, 0u64), |(c, b), (_, v)| (c + 1, b + v.len() as u64));
            Ok((count, bytes))
        }
        async fn list_keys(&self, prefix: &str) -> Result<Vec<String>, CoreError> {
            let map = self.data.lock().unwrap();
            Ok(map
                .keys()
                .filter(|k| k.starts_with(prefix))
                .cloned()
                .collect())
        }
    }

    struct FixedRegistry;

    #[async_trait]
    impl RegistryClient for FixedRegistry {
        fn registry_type(&self) -> &str {
            "test"
        }

        async fn resolve_metadata(&self, pkg: &PackageId) -> Result<PackageMetadata, CoreError> {
            Ok(PackageMetadata {
                id: pkg.clone(),
                published_at: Some(Utc::now() - chrono::Duration::days(30)),
                download_url: None,
                checksum: None,
                is_signed: None,
                extra: serde_json::json!({}),
                cache_control: None,
            })
        }

        async fn fetch_artifact(&self, pkg: &PackageId) -> Result<FetchedArtifact, CoreError> {
            let data = Bytes::from(format!("artifact:{}", pkg.cache_key()));
            Ok(FetchedArtifact {
                stream: Box::pin(stream::once(async move { Ok::<Bytes, CoreError>(data) })),
                cache_control: None,
            })
        }
    }

    struct DenyRegistry;

    #[async_trait]
    impl RegistryClient for DenyRegistry {
        fn registry_type(&self) -> &str {
            "test"
        }
        async fn resolve_metadata(&self, pkg: &PackageId) -> Result<PackageMetadata, CoreError> {
            Ok(PackageMetadata {
                id: pkg.clone(),
                published_at: Some(Utc::now() - chrono::Duration::days(30)),
                download_url: None,
                checksum: None,
                is_signed: None,
                extra: serde_json::json!({}),
                cache_control: None,
            })
        }
        async fn fetch_artifact(&self, _pkg: &PackageId) -> Result<FetchedArtifact, CoreError> {
            Err(CoreError::Registry("should not be called".into()))
        }
    }

    struct AlwaysDenyRule;

    #[async_trait]
    impl crate::rules::Rule for AlwaysDenyRule {
        fn name(&self) -> &str {
            "always_deny"
        }
        async fn evaluate(
            &self,
            _ctx: &crate::rules::RuleContext<'_>,
        ) -> crate::rules::RuleDecision {
            crate::rules::RuleDecision::Deny {
                reason: "test denial".to_owned(),
            }
        }
    }

    fn req(registry: &str) -> ProxyRequest {
        ProxyRequest {
            package_id: PackageId::new(registry, "test-pkg", "1.0.0"),
            identity: Identity::anonymous(),
            resource_type: "releases:read".to_owned(),
        }
    }

    fn proxy(
        registry_name: &str,
        client: Arc<dyn RegistryClient>,
        repo: Arc<dyn PackageRepository>,
        rules: Vec<Box<dyn crate::rules::Rule>>,
    ) -> ProxyService {
        let mut registries = HashMap::new();
        registries.insert(registry_name.to_owned(), client);
        let mut policies = HashMap::new();
        policies.insert(
            registry_name.to_owned(),
            RegistryPolicy {
                metadata_ttl: None,
                firewall_only: false,
                serve_stale_metadata: false,
                artifact_ttl: None,
                rules,
            },
        );
        ProxyService {
            registries,
            storage: MemStorage::new(),
            cache: TestCacheStore::new(),
            repo,
            artifact_meta: NoopArtifactMeta::arc(),
            policies,
            max_artifact_size_bytes: None,
            metrics: Arc::new(ProxyMetrics::new(&[])),
        }
    }

    // ── Tests ─────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn unknown_registry_returns_error() {
        let svc = proxy("npm", Arc::new(FixedRegistry), SpyRepo::new(), vec![]);
        let result = svc.handle(req("unknown")).await;
        assert!(matches!(result, Err(CoreError::UnknownRegistry(_))));
    }

    #[tokio::test]
    async fn metadata_cache_miss_then_hit() {
        let repo = SpyRepo::new();
        let cache = TestCacheStore::new();
        let svc = ProxyService {
            registries: {
                let mut m = HashMap::new();
                m.insert(
                    "npm".to_owned(),
                    Arc::new(FixedRegistry) as Arc<dyn RegistryClient>,
                );
                m
            },
            storage: MemStorage::new(),
            cache: cache.clone(),
            repo: repo.clone(),
            artifact_meta: NoopArtifactMeta::arc(),
            policies: {
                let mut m = HashMap::new();
                m.insert(
                    "npm".to_owned(),
                    RegistryPolicy {
                        metadata_ttl: Some(Duration::from_secs(300)),
                        firewall_only: false,
                        serve_stale_metadata: false,
                        artifact_ttl: None,
                        rules: vec![],
                    },
                );
                m
            },
            max_artifact_size_bytes: None,
            metrics: Arc::new(ProxyMetrics::new(&[])),
        };

        let cache_key = format!("meta:{}", req("npm").package_id.cache_key());

        // First call: cache miss — metadata is fetched and stored
        assert!(cache.get(&cache_key).await.unwrap().is_none());
        let resp = svc.handle(req("npm")).await.unwrap();
        assert!(matches!(resp, ProxyResponse::Stream(_)));
        assert!(
            cache.get(&cache_key).await.unwrap().is_some(),
            "metadata should be cached after first call"
        );

        // Second call: cache hit — lines 86-87 are exercised
        let resp2 = svc.handle(req("npm")).await.unwrap();
        assert!(
            matches!(resp2, ProxyResponse::Stream(_)),
            "second call must still return Stream"
        );
    }

    #[tokio::test]
    async fn rule_denial_returns_denied_and_records_event() {
        let repo = SpyRepo::new();
        let svc = proxy(
            "npm",
            Arc::new(DenyRegistry),
            repo.clone(),
            vec![Box::new(AlwaysDenyRule)],
        );

        let resp = svc.handle(req("npm")).await.unwrap();
        assert!(
            matches!(resp, ProxyResponse::Denied { reason } if reason == "test denial"),
            "expected Denied response"
        );
        let events = repo.events();
        assert_eq!(events.len(), 1, "one denied event should be recorded");
        assert!(matches!(events[0].result, AccessResult::Denied { .. }));
    }

    #[tokio::test]
    async fn artifact_cache_hit_returns_stored_bytes() {
        let storage = MemStorage::new();
        let pkg = PackageId::new("npm", "test-pkg", "1.0.0");
        let artifact_key = format!("artifact:{}", pkg.cache_key());
        // Pre-populate storage
        storage
            .store(
                &artifact_key,
                Bytes::from("cached!"),
                StorageMeta::default(),
            )
            .await
            .unwrap();

        let repo = SpyRepo::new();
        let svc = ProxyService {
            registries: {
                let mut m = HashMap::new();
                m.insert(
                    "npm".to_owned(),
                    Arc::new(FixedRegistry) as Arc<dyn RegistryClient>,
                );
                m
            },
            storage: storage.clone(),
            cache: TestCacheStore::new(),
            repo: repo.clone(),
            artifact_meta: NoopArtifactMeta::arc(),
            policies: {
                let mut m = HashMap::new();
                m.insert(
                    "npm".to_owned(),
                    RegistryPolicy {
                        metadata_ttl: None,
                        firewall_only: false,
                        serve_stale_metadata: false,
                        artifact_ttl: None,
                        rules: vec![],
                    },
                );
                m
            },
            max_artifact_size_bytes: None,
            metrics: Arc::new(ProxyMetrics::new(&[])),
        };

        let resp = svc.handle(req("npm")).await.unwrap();
        assert!(matches!(resp, ProxyResponse::Stream(_)));
        // Access event should be recorded for the cache hit
        assert!(!repo.events().is_empty(), "access event should be recorded");
    }

    #[tokio::test]
    async fn artifact_cache_miss_fetches_from_upstream() {
        let repo = SpyRepo::new();
        let svc = proxy("npm", Arc::new(FixedRegistry), repo.clone(), vec![]);

        let pkg = PackageId::new("npm", "test-pkg", "1.0.0");
        let artifact_key = format!("artifact:{}", pkg.cache_key());

        // Storage is empty — must fetch from upstream
        assert!(!svc.storage.exists(&artifact_key).await.unwrap());

        let resp = svc.handle(req("npm")).await.unwrap();
        assert!(matches!(resp, ProxyResponse::Stream(_)));

        // Artifact should now be stored
        assert!(
            svc.storage.exists(&artifact_key).await.unwrap(),
            "artifact should be stored after fetch"
        );
        assert!(!repo.events().is_empty(), "access event should be recorded");
    }

    #[tokio::test]
    async fn payload_too_large_returns_error() {
        let repo = SpyRepo::new();
        let svc = ProxyService {
            registries: {
                let mut m = HashMap::new();
                m.insert(
                    "npm".to_owned(),
                    Arc::new(FixedRegistry) as Arc<dyn RegistryClient>,
                );
                m
            },
            storage: MemStorage::new(),
            cache: TestCacheStore::new(),
            repo: repo.clone(),
            artifact_meta: NoopArtifactMeta::arc(),
            policies: {
                let mut m = HashMap::new();
                m.insert(
                    "npm".to_owned(),
                    RegistryPolicy {
                        metadata_ttl: None,
                        firewall_only: false,
                        serve_stale_metadata: false,
                        artifact_ttl: None,
                        rules: vec![],
                    },
                );
                m
            },
            max_artifact_size_bytes: Some(5), // FixedRegistry sends >5 bytes
            metrics: Arc::new(ProxyMetrics::new(&[])),
        };

        let result = svc.handle(req("npm")).await;
        assert!(matches!(result, Err(CoreError::PayloadTooLarge(_))));
    }

    #[tokio::test]
    async fn unused_registry_id_in_policies_does_not_panic() {
        let repo = SpyRepo::new();
        let svc = ProxyService {
            registries: {
                let mut m = HashMap::new();
                m.insert(
                    "npm".to_owned(),
                    Arc::new(FixedRegistry) as Arc<dyn RegistryClient>,
                );
                m
            },
            storage: MemStorage::new(),
            cache: TestCacheStore::new(),
            repo: repo.clone(),
            artifact_meta: NoopArtifactMeta::arc(),
            policies: HashMap::new(), // no policy for "npm" — should use empty rule set
            max_artifact_size_bytes: None,
            metrics: Arc::new(ProxyMetrics::new(&[])),
        };

        let resp = svc.handle(req("npm")).await.unwrap();
        assert!(matches!(resp, ProxyResponse::Stream(_)));
    }

    #[tokio::test]
    async fn firewall_only_streams_without_storing() {
        let storage = MemStorage::new();
        let repo = SpyRepo::new();
        let svc = ProxyService {
            registries: {
                let mut m = HashMap::new();
                m.insert(
                    "npm".to_owned(),
                    Arc::new(FixedRegistry) as Arc<dyn RegistryClient>,
                );
                m
            },
            storage: storage.clone(),
            cache: TestCacheStore::new(),
            repo: repo.clone(),
            artifact_meta: NoopArtifactMeta::arc(),
            policies: {
                let mut m = HashMap::new();
                m.insert(
                    "npm".to_owned(),
                    RegistryPolicy {
                        metadata_ttl: None,
                        firewall_only: true,
                        serve_stale_metadata: false,
                        artifact_ttl: None,
                        rules: vec![],
                    },
                );
                m
            },
            max_artifact_size_bytes: None,
            metrics: Arc::new(ProxyMetrics::new(&[])),
        };

        let pkg = PackageId::new("npm", "test-pkg", "1.0.0");
        let artifact_key = format!("artifact:{}", pkg.cache_key());

        let resp = svc.handle(req("npm")).await.unwrap();
        assert!(matches!(resp, ProxyResponse::Stream(_)));
        assert!(
            !storage.exists(&artifact_key).await.unwrap(),
            "firewall-only: artifact must not be stored"
        );
        assert!(!repo.events().is_empty(), "access event should be recorded");
    }

    struct UnavailableRegistry;

    #[async_trait]
    impl RegistryClient for UnavailableRegistry {
        fn registry_type(&self) -> &str {
            "test"
        }
        async fn resolve_metadata(&self, _pkg: &PackageId) -> Result<PackageMetadata, CoreError> {
            Err(CoreError::Registry("upstream down".into()))
        }
        async fn fetch_artifact(&self, pkg: &PackageId) -> Result<FetchedArtifact, CoreError> {
            let data = Bytes::from(format!("artifact:{}", pkg.cache_key()));
            Ok(FetchedArtifact {
                stream: Box::pin(stream::once(async move { Ok::<Bytes, CoreError>(data) })),
                cache_control: None,
            })
        }
    }

    fn proxy_with_stale(
        client: Arc<dyn RegistryClient>,
        repo: Arc<dyn PackageRepository>,
        cache: Arc<dyn CacheStore>,
        serve_stale: bool,
    ) -> ProxyService {
        let mut registries = HashMap::new();
        registries.insert("npm".to_owned(), client);
        let mut policies = HashMap::new();
        policies.insert(
            "npm".to_owned(),
            RegistryPolicy {
                metadata_ttl: Some(Duration::from_secs(300)),
                firewall_only: false,
                serve_stale_metadata: serve_stale,
                artifact_ttl: None,
                rules: vec![],
            },
        );
        ProxyService {
            registries,
            storage: MemStorage::new(),
            cache,
            repo,
            artifact_meta: NoopArtifactMeta::arc(),
            policies,
            max_artifact_size_bytes: None,
            metrics: Arc::new(ProxyMetrics::new(&[])),
        }
    }

    #[tokio::test]
    async fn stale_metadata_served_when_upstream_unavailable() {
        let repo = SpyRepo::new();
        let cache = TestCacheStore::new();
        let pkg = PackageId::new("npm", "test-pkg", "1.0.0");
        let cache_key = format!("meta:{}", pkg.cache_key());
        let stale_meta = PackageMetadata {
            id: pkg.clone(),
            published_at: Some(Utc::now() - chrono::Duration::days(10)),
            download_url: None,
            checksum: None,
            is_signed: None,
            extra: serde_json::json!({}),
            cache_control: None,
        };
        cache.seed_expired(&cache_key, stale_meta);

        let svc = proxy_with_stale(Arc::new(UnavailableRegistry), repo.clone(), cache, true);
        let resp = svc.handle(req("npm")).await.unwrap();
        assert!(
            matches!(resp, ProxyResponse::Stream(_)),
            "stale fallback should succeed"
        );
        assert!(
            repo.events()
                .iter()
                .all(|e| !matches!(e.result, AccessResult::ProxyError { .. })),
            "no proxy_error should be recorded when stale metadata is served"
        );
    }

    #[tokio::test]
    async fn stale_not_used_when_serve_stale_false() {
        let repo = SpyRepo::new();
        let cache = TestCacheStore::new();
        let pkg = PackageId::new("npm", "test-pkg", "1.0.0");
        let cache_key = format!("meta:{}", pkg.cache_key());
        let stale_meta = PackageMetadata {
            id: pkg.clone(),
            published_at: Some(Utc::now() - chrono::Duration::days(10)),
            download_url: None,
            checksum: None,
            is_signed: None,
            extra: serde_json::json!({}),
            cache_control: None,
        };
        cache.seed_expired(&cache_key, stale_meta);

        let svc = proxy_with_stale(Arc::new(UnavailableRegistry), repo.clone(), cache, false);
        let result = svc.handle(req("npm")).await;
        assert!(
            matches!(result, Err(CoreError::Registry(_))),
            "should propagate the upstream error"
        );
        assert!(
            repo.events()
                .iter()
                .any(|e| matches!(e.result, AccessResult::ProxyError { .. })),
            "proxy_error must be recorded"
        );
    }

    #[tokio::test]
    async fn cold_start_with_upstream_down_returns_error() {
        let repo = SpyRepo::new();
        let cache = TestCacheStore::new(); // empty — no stale entry

        let svc = proxy_with_stale(Arc::new(UnavailableRegistry), repo.clone(), cache, true);
        let result = svc.handle(req("npm")).await;
        assert!(
            matches!(result, Err(CoreError::Registry(_))),
            "no stale entry + upstream down must return error"
        );
    }

    #[tokio::test]
    async fn not_found_from_upstream_is_not_stale_eligible() {
        struct NotFoundRegistry;
        #[async_trait]
        impl RegistryClient for NotFoundRegistry {
            fn registry_type(&self) -> &str {
                "test"
            }
            async fn resolve_metadata(
                &self,
                pkg: &PackageId,
            ) -> Result<PackageMetadata, CoreError> {
                Err(CoreError::NotFound(pkg.name.clone()))
            }
            async fn fetch_artifact(&self, _pkg: &PackageId) -> Result<FetchedArtifact, CoreError> {
                Err(CoreError::NotFound("no artifact".into()))
            }
        }

        let repo = SpyRepo::new();
        let cache = TestCacheStore::new();
        let pkg = PackageId::new("npm", "test-pkg", "1.0.0");
        let cache_key = format!("meta:{}", pkg.cache_key());
        let stale_meta = PackageMetadata {
            id: pkg.clone(),
            published_at: Some(Utc::now() - chrono::Duration::days(10)),
            download_url: None,
            checksum: None,
            is_signed: None,
            extra: serde_json::json!({}),
            cache_control: None,
        };
        cache.seed_expired(&cache_key, stale_meta);

        let svc = proxy_with_stale(Arc::new(NotFoundRegistry), repo.clone(), cache, true);
        let result = svc.handle(req("npm")).await;
        assert!(
            matches!(result, Err(CoreError::NotFound(_))),
            "NotFound must not fall back to stale"
        );
    }

    // ── Cache-Control and ArtifactMeta integration tests ─────────────────────

    struct NoStoreMetaRegistry;

    #[async_trait]
    impl RegistryClient for NoStoreMetaRegistry {
        fn registry_type(&self) -> &str {
            "test"
        }
        async fn resolve_metadata(&self, pkg: &PackageId) -> Result<PackageMetadata, CoreError> {
            Ok(PackageMetadata {
                id: pkg.clone(),
                published_at: Some(Utc::now() - chrono::Duration::days(1)),
                download_url: None,
                checksum: None,
                is_signed: None,
                extra: serde_json::json!({}),
                cache_control: Some("no-store".to_owned()),
            })
        }
        async fn fetch_artifact(&self, pkg: &PackageId) -> Result<FetchedArtifact, CoreError> {
            let data = Bytes::from(format!("artifact:{}", pkg.cache_key()));
            Ok(FetchedArtifact {
                stream: Box::pin(stream::once(async move { Ok::<Bytes, CoreError>(data) })),
                cache_control: None,
            })
        }
    }

    struct NoStoreArtifactRegistry;

    #[async_trait]
    impl RegistryClient for NoStoreArtifactRegistry {
        fn registry_type(&self) -> &str {
            "test"
        }
        async fn resolve_metadata(&self, pkg: &PackageId) -> Result<PackageMetadata, CoreError> {
            Ok(PackageMetadata {
                id: pkg.clone(),
                published_at: Some(Utc::now() - chrono::Duration::days(1)),
                download_url: None,
                checksum: None,
                is_signed: None,
                extra: serde_json::json!({}),
                cache_control: None,
            })
        }
        async fn fetch_artifact(&self, pkg: &PackageId) -> Result<FetchedArtifact, CoreError> {
            let data = Bytes::from(format!("artifact:{}", pkg.cache_key()));
            Ok(FetchedArtifact {
                stream: Box::pin(stream::once(async move { Ok::<Bytes, CoreError>(data) })),
                cache_control: Some("no-store".to_owned()),
            })
        }
    }

    #[tokio::test]
    async fn metadata_no_store_skips_cache() {
        let repo = SpyRepo::new();
        let cache = TestCacheStore::new();
        let mut registries = HashMap::new();
        registries.insert(
            "npm".to_owned(),
            Arc::new(NoStoreMetaRegistry) as Arc<dyn RegistryClient>,
        );
        let mut policies = HashMap::new();
        policies.insert(
            "npm".to_owned(),
            RegistryPolicy {
                metadata_ttl: Some(Duration::from_secs(300)),
                firewall_only: false,
                serve_stale_metadata: false,
                artifact_ttl: None,
                rules: vec![],
            },
        );
        let svc = ProxyService {
            registries,
            storage: MemStorage::new(),
            cache: cache.clone(),
            repo,
            artifact_meta: NoopArtifactMeta::arc(),
            policies,
            max_artifact_size_bytes: None,
            metrics: Arc::new(ProxyMetrics::new(&[])),
        };

        let cache_key = format!("meta:{}", req("npm").package_id.cache_key());
        let resp = svc.handle(req("npm")).await.unwrap();
        assert!(
            matches!(resp, ProxyResponse::Stream(_)),
            "response must still be a stream"
        );
        assert!(
            cache.get(&cache_key).await.unwrap().is_none(),
            "metadata must NOT be cached when upstream returns Cache-Control: no-store"
        );
    }

    #[tokio::test]
    async fn artifact_no_store_skips_storage() {
        let repo = SpyRepo::new();
        let storage = MemStorage::new();
        let mut registries = HashMap::new();
        registries.insert(
            "npm".to_owned(),
            Arc::new(NoStoreArtifactRegistry) as Arc<dyn RegistryClient>,
        );
        let svc = ProxyService {
            registries,
            storage: storage.clone(),
            cache: TestCacheStore::new(),
            repo,
            artifact_meta: NoopArtifactMeta::arc(),
            policies: HashMap::new(),
            max_artifact_size_bytes: None,
            metrics: Arc::new(ProxyMetrics::new(&[])),
        };

        let pkg = PackageId::new("npm", "test-pkg", "1.0.0");
        let artifact_key = format!("artifact:{}", pkg.cache_key());

        let resp = svc.handle(req("npm")).await.unwrap();
        assert!(
            matches!(resp, ProxyResponse::Stream(_)),
            "response must still be a stream"
        );
        assert!(
            !storage.exists(&artifact_key).await.unwrap(),
            "artifact must NOT be stored when upstream returns Cache-Control: no-store"
        );
    }

    #[tokio::test]
    async fn artifact_ttl_expired_refetches_from_upstream() {
        let storage = MemStorage::new();
        let pkg = PackageId::new("npm", "test-pkg", "1.0.0");
        let artifact_key = format!("artifact:{}", pkg.cache_key());
        storage
            .store(
                &artifact_key,
                Bytes::from("stale-bytes"),
                StorageMeta::default(),
            )
            .await
            .unwrap();

        // Spy meta says artifact is expired
        let expired_meta = ArtifactMeta {
            artifact_key: artifact_key.clone(),
            registry: "npm".to_owned(),
            package_name: "test-pkg".to_owned(),
            version: "1.0.0".to_owned(),
            size_bytes: Some(11),
            cached_at: Utc::now() - chrono::Duration::hours(2),
            last_accessed_at: Utc::now() - chrono::Duration::hours(2),
        };
        let spy_meta = SpyArtifactMeta::with_expired(vec![expired_meta]);

        let repo = SpyRepo::new();
        let mut registries = HashMap::new();
        registries.insert(
            "npm".to_owned(),
            Arc::new(FixedRegistry) as Arc<dyn RegistryClient>,
        );
        let mut policies = HashMap::new();
        policies.insert(
            "npm".to_owned(),
            RegistryPolicy {
                metadata_ttl: None,
                firewall_only: false,
                serve_stale_metadata: false,
                artifact_ttl: Some(Duration::from_secs(3600)), // 1h TTL
                rules: vec![],
            },
        );
        let svc = ProxyService {
            registries,
            storage: storage.clone(),
            cache: TestCacheStore::new(),
            repo,
            artifact_meta: spy_meta.clone(),
            policies,
            max_artifact_size_bytes: None,
            metrics: Arc::new(ProxyMetrics::new(&[])),
        };

        let resp = svc.handle(req("npm")).await.unwrap();
        assert!(matches!(resp, ProxyResponse::Stream(_)));
        // After re-fetch, record_artifact should have been called
        assert!(
            !spy_meta.recorded_keys().is_empty(),
            "record_artifact must be called after re-fetch"
        );
        // Storage should now contain the freshly fetched artifact
        assert!(storage.exists(&artifact_key).await.unwrap());
    }

    #[tokio::test]
    async fn artifact_cache_hit_records_touch() {
        let storage = MemStorage::new();
        let pkg = PackageId::new("npm", "test-pkg", "1.0.0");
        let artifact_key = format!("artifact:{}", pkg.cache_key());
        storage
            .store(
                &artifact_key,
                Bytes::from("cached!"),
                StorageMeta::default(),
            )
            .await
            .unwrap();

        // Pre-seed the artifact metadata to simulate a previous record_artifact call.
        // Without this, is_artifact_expired treats the missing row as expired (correct
        // production behavior) and the proxy re-fetches instead of serving from cache.
        let spy_meta = SpyArtifactMeta::new();
        spy_meta
            .record_artifact(&artifact_key, "npm", "test-pkg", "1.0.0", None)
            .await
            .unwrap();
        let mut registries = HashMap::new();
        registries.insert(
            "npm".to_owned(),
            Arc::new(FixedRegistry) as Arc<dyn RegistryClient>,
        );
        let mut policies = HashMap::new();
        policies.insert(
            "npm".to_owned(),
            RegistryPolicy {
                metadata_ttl: None,
                firewall_only: false,
                serve_stale_metadata: false,
                artifact_ttl: Some(Duration::from_secs(3600)),
                rules: vec![],
            },
        );
        let svc = ProxyService {
            registries,
            storage: storage.clone(),
            cache: TestCacheStore::new(),
            repo: SpyRepo::new(),
            artifact_meta: spy_meta.clone(),
            policies,
            max_artifact_size_bytes: None,
            metrics: Arc::new(ProxyMetrics::new(&[])),
        };

        let resp = svc.handle(req("npm")).await.unwrap();
        assert!(matches!(resp, ProxyResponse::Stream(_)));
        // touch_artifact is called from tokio::spawn — yield to let it complete
        tokio::task::yield_now().await;
        assert!(
            spy_meta.touched_keys().contains(&artifact_key),
            "touch_artifact must be called on cache hit"
        );
    }

    #[tokio::test]
    async fn artifact_cache_miss_records_meta() {
        let spy_meta = SpyArtifactMeta::new();
        let repo = SpyRepo::new();
        let mut registries = HashMap::new();
        registries.insert(
            "npm".to_owned(),
            Arc::new(FixedRegistry) as Arc<dyn RegistryClient>,
        );
        let svc = ProxyService {
            registries,
            storage: MemStorage::new(),
            cache: TestCacheStore::new(),
            repo,
            artifact_meta: spy_meta.clone(),
            policies: HashMap::new(),
            max_artifact_size_bytes: None,
            metrics: Arc::new(ProxyMetrics::new(&[])),
        };

        let pkg = PackageId::new("npm", "test-pkg", "1.0.0");
        let artifact_key = format!("artifact:{}", pkg.cache_key());

        let resp = svc.handle(req("npm")).await.unwrap();
        assert!(matches!(resp, ProxyResponse::Stream(_)));
        assert!(
            spy_meta.recorded_keys().contains(&artifact_key),
            "record_artifact must be called after a cache miss and successful upstream fetch"
        );
    }

    #[tokio::test]
    async fn metrics_artifact_miss_then_hit() {
        let proxy_metrics = Arc::new(ProxyMetrics::new(&["npm".to_owned()]));
        let storage = MemStorage::new();
        let svc = ProxyService {
            registries: {
                let mut m = HashMap::new();
                m.insert(
                    "npm".to_owned(),
                    Arc::new(FixedRegistry) as Arc<dyn RegistryClient>,
                );
                m
            },
            storage: storage.clone(),
            cache: TestCacheStore::new(),
            repo: SpyRepo::new(),
            artifact_meta: NoopArtifactMeta::arc(),
            policies: {
                let mut m = HashMap::new();
                m.insert(
                    "npm".to_owned(),
                    RegistryPolicy {
                        metadata_ttl: Some(Duration::from_secs(300)),
                        firewall_only: false,
                        serve_stale_metadata: false,
                        artifact_ttl: None,
                        rules: vec![],
                    },
                );
                m
            },
            max_artifact_size_bytes: None,
            metrics: proxy_metrics.clone(),
        };

        let npm = proxy_metrics.all().get("npm").unwrap();

        // First call: artifact not in storage → miss counter incremented
        svc.handle(req("npm")).await.unwrap();
        assert_eq!(npm.misses(), 1, "first call must register a miss");
        assert_eq!(npm.hits(), 0);

        // Second call: artifact now in storage → hit counter incremented
        svc.handle(req("npm")).await.unwrap();
        assert_eq!(npm.misses(), 1, "miss count must not change on second call");
        assert_eq!(npm.hits(), 1, "second call must register a hit");
    }
}
