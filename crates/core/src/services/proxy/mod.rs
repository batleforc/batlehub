mod cache;
mod handle;
mod resolve;

use std::sync::Arc;
use std::time::Instant;

use crate::error::CoreError;
use crate::ports::{
    ArtifactCacheMeta, ArtifactStream, CacheStore, PackageRepository, StorageBackend,
};
use crate::services::hot_config::HotConfigLock;
use crate::services::metrics::ProxyMetrics;
use crate::services::sbom::SbomService;

/// Input to `ProxyService::handle`.
pub struct ProxyRequest {
    pub package_id: crate::entities::PackageId,
    pub identity: crate::entities::Identity,
    /// The operation being checked against RBAC (e.g. `"releases:read"`).
    pub resource_type: String,
    /// Caller's IP address (for audit log enrichment).
    pub ip_address: Option<String>,
    /// HTTP User-Agent header (for audit log enrichment).
    pub user_agent: Option<String>,
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
    /// Hot-swappable state (registries, policies, size limit). Replaced atomically on reload.
    pub hot: HotConfigLock,
    pub storage: Arc<dyn StorageBackend>,
    pub cache: Arc<dyn CacheStore>,
    pub repo: Arc<dyn PackageRepository>,
    pub artifact_meta: Arc<dyn ArtifactCacheMeta>,
    /// In-memory counters for the stats dashboard (reset on restart).
    pub metrics: Arc<ProxyMetrics>,
    /// Optional SBOM service; when `None`, SBOM generation is disabled globally.
    pub sbom: Option<Arc<SbomService>>,
}

pub(super) fn warn_if_audit_failed(r: Result<(), CoreError>, ctx: &str) {
    if let Err(e) = r {
        tracing::warn!(error = %e, ctx, "audit log write failed");
    }
}

/// Emit the terminal per-request metrics — the `batlehub_requests_total{outcome}`
/// counter and the `batlehub_request_duration_seconds` histogram — at a request's
/// exit point. Collapses the counter+histogram pair that every return path repeats.
pub(super) fn finish_request(registry_label: &Arc<str>, outcome: &'static str, start: Instant) {
    metrics::counter!("batlehub_requests_total", "registry" => Arc::clone(registry_label), "outcome" => outcome).increment(1);
    metrics::histogram!("batlehub_request_duration_seconds", "registry" => Arc::clone(registry_label))
        .record(start.elapsed().as_secs_f64());
}

#[cfg(test)]
mod tests;
