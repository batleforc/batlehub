mod handle;
mod resolve;

use std::sync::Arc;

use crate::error::CoreError;
use crate::ports::{
    ArtifactMetaRepository, ArtifactStream, CacheStore, PackageRepository, StorageBackend,
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
    pub artifact_meta: Arc<dyn ArtifactMetaRepository>,
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

#[cfg(test)]
mod tests;
