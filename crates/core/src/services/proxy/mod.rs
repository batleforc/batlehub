mod cache;
mod handle;
mod resolve;

use std::sync::Arc;
use std::time::Instant;

use futures::{stream, StreamExt};

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

/// The registry metrics label plus the request's start time — the pair every
/// helper in `cache.rs`'s fetch/verify/evict/serve chain needs so it can label
/// its own metrics and, on its return path, call [`finish_request`] with the
/// same values [`ProxyService::handle`](super::ProxyService::handle) captured
/// at the top of the request. Grouped into one value instead of two loose
/// trailing parameters repeated across 8 functions.
#[derive(Clone)]
pub(super) struct RequestTiming {
    pub(super) registry_label: Arc<str>,
    pub(super) start: Instant,
}

/// Emit the terminal per-request metrics — the `batlehub_requests_total{outcome}`
/// counter and the `batlehub_request_duration_seconds` histogram — at a request's
/// exit point. Collapses the counter+histogram pair that every return path repeats.
pub(super) fn finish_request(registry_label: &Arc<str>, outcome: &'static str, start: Instant) {
    metrics::counter!("batlehub_requests_total", "registry" => Arc::clone(registry_label), "outcome" => outcome).increment(1);
    metrics::histogram!("batlehub_request_duration_seconds", "registry" => Arc::clone(registry_label))
        .record(start.elapsed().as_secs_f64());
}

/// Records a single upstream-latency sample under
/// `batlehub_upstream_request_duration_seconds{registry,operation}`.
pub(super) fn record_upstream_duration(
    registry_label: &Arc<str>,
    operation: &'static str,
    start: Instant,
) {
    metrics::histogram!(
        "batlehub_upstream_request_duration_seconds",
        "registry" => Arc::clone(registry_label),
        "operation" => operation
    )
    .record(start.elapsed().as_secs_f64());
}

/// Times a call out to an upstream registry client and records it under
/// `batlehub_upstream_request_duration_seconds`, regardless of whether the call
/// succeeds — a hung or slow-failing upstream is exactly the "degraded" case this
/// metric exists to catch, so failures must count too.
///
/// Only appropriate for calls whose future resolves once the *entire* answer is
/// available, e.g. `resolve_metadata`. `fetch_artifact` returns as soon as response
/// headers arrive and hands back a lazily-consumed body stream, so timing its
/// future alone would only measure time-to-first-byte and miss a slow/degraded
/// body transfer — use [`time_upstream_stream`] for that instead.
pub(super) async fn time_upstream_call<T, E>(
    registry_label: &Arc<str>,
    operation: &'static str,
    fut: impl std::future::Future<Output = Result<T, E>>,
) -> Result<T, E> {
    let start = Instant::now();
    let result = fut.await;
    record_upstream_duration(registry_label, operation, start);
    result
}

/// Wraps a freshly-fetched artifact byte stream so `operation`'s latency is
/// recorded once the stream is fully drained — cleanly exhausted or ended by an
/// error — rather than when the initial response headers arrived. `start` should
/// be captured immediately before the `fetch_artifact` call that produced this
/// stream, so the recorded duration covers the whole transfer.
///
/// Every consumer of a `FetchedArtifact::stream` (the cached-fetch path, the
/// no-store passthrough, firewall-only mode, and cache warming) must route the
/// stream through this so a slow/degraded body transfer — not just a slow
/// response header — trips the upstream-latency alert.
pub(super) fn time_upstream_stream(
    registry_label: Arc<str>,
    operation: &'static str,
    start: Instant,
    stream: ArtifactStream,
) -> ArtifactStream {
    Box::pin(stream::unfold(
        (stream, registry_label, operation, start, false),
        |(mut stream, registry_label, operation, start, done)| async move {
            if done {
                return None;
            }
            let next = stream.next().await;
            let finished = !matches!(next, Some(Ok(_)));
            if finished {
                record_upstream_duration(&registry_label, operation, start);
            }
            next.map(|item| (item, (stream, registry_label, operation, start, finished)))
        },
    ))
}

#[cfg(test)]
mod tests;
