use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use chrono::Utc;
use futures::StreamExt;

use crate::entities::{AccessEvent, Identity, PackageId};
use crate::error::CoreError;
use crate::ports::{
    ArtifactStream, CacheEntry, CacheStore, PackageRepository, RegistryClient, StorageBackend,
    StorageMeta,
};
use crate::rules::{evaluate_rules, Rule, RuleContext, RuleDecision};

pub struct RegistryPolicy {
    pub metadata_ttl: Option<Duration>,
    /// Rules evaluated in order for every request to this registry.
    pub rules: Vec<Box<dyn Rule>>,
}

pub struct ProxyRequest {
    pub package_id: PackageId,
    pub identity: Identity,
    /// The operation being checked against RBAC (e.g. `"releases:read"`).
    pub resource_type: String,
}

pub enum ProxyResponse {
    /// Artifact stream to forward to the HTTP client.
    Stream(ArtifactStream),
    /// Access was denied; the caller should receive a 403.
    Denied { reason: String },
}

pub struct ProxyService {
    pub registries: HashMap<String, Arc<dyn RegistryClient>>,
    pub storage: Arc<dyn StorageBackend>,
    pub cache: Arc<dyn CacheStore>,
    pub repo: Arc<dyn PackageRepository>,
    pub policies: HashMap<String, RegistryPolicy>,
}

impl ProxyService {
    pub async fn handle(&self, req: ProxyRequest) -> Result<ProxyResponse, CoreError> {
        let registry_name = req.package_id.registry.as_str();

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
            entry.metadata
        } else {
            tracing::debug!(key = %cache_key, "metadata cache miss, fetching from upstream");
            let meta = client.resolve_metadata(&req.package_id).await?;
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
        };

        if let RuleDecision::Deny { reason } = evaluate_rules(rules, &ctx).await {
            self.repo
                .record_access(AccessEvent::denied_download(
                    req.package_id,
                    req.identity.user_id,
                    req.identity.role,
                    reason.clone(),
                ))
                .await
                .unwrap_or_else(|e| tracing::warn!(error = %e, "failed to record denied access"));
            return Ok(ProxyResponse::Denied { reason });
        }

        // ── 3. Check artifact cache ────────────────────────────────────────────
        let artifact_key = format!("artifact:{}", req.package_id.cache_key());

        if self.storage.exists(&artifact_key).await? {
            tracing::debug!(key = %artifact_key, "artifact cache hit");
            let artifact = self
                .storage
                .retrieve(&artifact_key)
                .await?
                .expect("exists() returned true");

            self.repo
                .record_access(AccessEvent::allowed_download(
                    req.package_id,
                    req.identity.user_id,
                    req.identity.role,
                ))
                .await
                .unwrap_or_else(|e| tracing::warn!(error = %e, "failed to record access"));

            return Ok(ProxyResponse::Stream(artifact.stream));
        }

        // ── 4. Fetch from upstream and cache ──────────────────────────────────
        tracing::debug!(key = %artifact_key, "artifact not cached, fetching from upstream");
        let mut upstream = client.fetch_artifact(&req.package_id).await?;

        let mut buf: Vec<u8> = Vec::new();
        while let Some(chunk) = upstream.next().await {
            buf.extend_from_slice(&chunk?);
        }
        let data = Bytes::from(buf);

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

        self.repo
            .record_access(AccessEvent::allowed_download(
                req.package_id,
                req.identity.user_id,
                req.identity.role,
            ))
            .await
            .unwrap_or_else(|e| tracing::warn!(error = %e, "failed to record access"));

        let stream = futures::stream::once(async move { Ok(data) });
        Ok(ProxyResponse::Stream(Box::pin(stream)))
    }
}
