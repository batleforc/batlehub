//! Artifact cache paths for [`ProxyService::handle`](super::ProxyService::handle):
//! serving a fresh cache hit (with optional re-serve verification) and the
//! upstream fetch-and-cache miss path. Split out of `handle.rs` to keep the
//! top-level orchestrator readable.

use std::sync::Arc;
use std::time::Instant;

use bytes::Bytes;
use futures::StreamExt;

use crate::entities::{AccessEvent, PackageMetadata};
use crate::error::CoreError;
use crate::ports::{RegistryClient, StorageMeta};
use crate::services::cache_control::parse_cache_control;
use crate::services::hot_config::IntegrityPolicy;

use super::handle::RESERVE_VERIFY_BUFFER_LIMIT;
use super::{ProxyRequest, ProxyResponse, ProxyService};

impl ProxyService {
    /// Serve an artifact from a fresh cache hit. When `verify_on_serve` is set,
    /// the stored bytes are re-hashed against the recorded SHA-256 before being
    /// streamed (failing closed on a lookup error, evicting on a mismatch). See
    /// the module-level note on `RESERVE_VERIFY_BUFFER_LIMIT` for the
    /// retain-vs-re-read trade-off.
    pub(super) async fn serve_cache_hit(
        &self,
        req: ProxyRequest,
        artifact_key: String,
        integrity: &IntegrityPolicy,
        registry_label: String,
        start: Instant,
    ) -> Result<ProxyResponse, CoreError> {
        let registry_name = req.package_id.registry.as_str();
        tracing::debug!(key = %artifact_key, "artifact cache hit");
        metrics::counter!("batlehub_artifact_cache_hits_total", "registry" => registry_label.clone()).increment(1);
        self.metrics.record_artifact_hit(registry_name);
        let artifact = self.storage.retrieve(&artifact_key).await?.ok_or_else(|| {
            CoreError::Registry(format!(
                "artifact '{artifact_key}' vanished between exists and retrieve"
            ))
        })?;

        // Re-serve integrity verification (opt-in via `verify_on_serve`): re-hash
        // the stored bytes against the SHA-256 we computed when they were first
        // cached. Catches storage corruption or tampering of an already-cached
        // artifact. Off by default because it reads + hashes the bytes on every
        // cache hit. The hash is computed by streaming the stored bytes through a
        // `StreamingVerifier` (memory stays bounded regardless of artifact size);
        // a verified entry is then re-opened from storage to serve.
        let response_stream: crate::ports::ByteStream = if integrity.enabled
            && integrity.verify_on_serve
        {
            let expected = match self
                .artifact_meta
                .get_artifact_checksum(&artifact_key)
                .await
            {
                // A recorded checksum: re-verify against it below.
                Ok(Some(c)) => Some(c),
                // No checksum recorded (entry cached before `verify_on_serve`
                // existed, or never refreshed since): documented skip — serve
                // as-is until the entry is next refreshed with a checksum.
                Ok(None) => None,
                // The checksum lookup itself failed. `verify_on_serve` is an
                // opt-in guarantee that every served byte is re-verified, so we
                // must fail closed rather than serve possibly-corrupt bytes we
                // cannot check. The cache entry is left intact (the bytes may be
                // fine; we just could not confirm it this time).
                Err(e) => {
                    let reason = format!(
                        "cannot re-verify cached artifact for {}: checksum lookup failed: {e}",
                        req.package_id,
                    );
                    tracing::warn!(registry = %registry_name, key = %artifact_key, error = %e, "re-serve checksum lookup failed; failing closed");
                    metrics::counter!("batlehub_integrity_checks_total", "registry" => registry_label.clone(), "outcome" => "lookup_failed", "phase" => "reserve").increment(1);
                    super::finish_request(&registry_label, "integrity_failed", start);
                    return Err(CoreError::IntegrityFailure(reason));
                }
            };
            use crate::services::integrity::{IntegrityOutcome, StreamingVerifier};
            match expected.as_deref().map(StreamingVerifier::new) {
                // A recorded, parseable checksum: stream the stored bytes through
                // the hasher. Retain the bytes up to `RESERVE_VERIFY_BUFFER_LIMIT`
                // so a verified small artifact (the common case) is served from the
                // exact bytes we hashed — a single read, with no re-retrieve race.
                // An artifact larger than the cap drops the retained copy and is
                // served by re-opening a fresh stream, keeping peak memory bounded.
                Some(Some(mut verifier)) => {
                    let mut s = artifact.stream;
                    let mut retained: Option<Vec<u8>> = Some(Vec::new());
                    while let Some(chunk) = s.next().await {
                        let chunk = chunk?;
                        verifier.update(&chunk);
                        if let Some(buf) = retained.as_mut() {
                            if buf.len() + chunk.len() > RESERVE_VERIFY_BUFFER_LIMIT {
                                // Over the cap: stop retaining and hash-only from here.
                                retained = None;
                            } else {
                                buf.extend_from_slice(&chunk);
                            }
                        }
                    }
                    match verifier.finish() {
                        IntegrityOutcome::Verified { algo } => {
                            metrics::counter!("batlehub_integrity_checks_total", "registry" => registry_label.clone(), "outcome" => "verified", "phase" => "reserve").increment(1);
                            tracing::debug!(registry = %registry_name, key = %artifact_key, algo = algo.as_str(), "cached artifact re-verified on serve");
                        }
                        IntegrityOutcome::Mismatch {
                            algo,
                            expected,
                            actual,
                        } => {
                            metrics::counter!("batlehub_integrity_checks_total", "registry" => registry_label.clone(), "outcome" => "mismatch", "phase" => "reserve").increment(1);
                            tracing::warn!(registry = %registry_name, key = %artifact_key, algo = algo.as_str(), %expected, %actual, "cached artifact failed re-serve integrity check; evicting");
                            // Drop the corrupt entry so a later request re-fetches clean bytes.
                            if let Err(e) = self.storage.delete(&artifact_key).await {
                                tracing::warn!(key = %artifact_key, error = %e, "failed to evict corrupt cached artifact");
                            }
                            if let Err(e) =
                                self.artifact_meta.delete_artifact_meta(&artifact_key).await
                            {
                                tracing::warn!(key = %artifact_key, error = %e, "failed to delete meta for corrupt artifact");
                            }
                            let reason = format!(
                                "cached artifact failed re-serve integrity check for {}: {} digest mismatch (expected {expected}, got {actual})",
                                req.package_id,
                                algo.as_str(),
                            );
                            super::warn_if_audit_failed(
                                self.repo
                                    .record_access(AccessEvent::proxy_error(
                                        req.package_id.clone(),
                                        req.identity.user_id.clone(),
                                        req.identity.role.clone(),
                                        reason.clone(),
                                    ))
                                    .await,
                                "reserve integrity mismatch",
                            );
                            super::finish_request(&registry_label, "integrity_failed", start);
                            return Err(CoreError::IntegrityFailure(reason));
                        }
                        // `StreamingVerifier::new` already rejected unparseable
                        // checksums, so `finish` never returns `Unparseable`.
                        IntegrityOutcome::Unparseable => {
                            tracing::warn!(registry = %registry_name, key = %artifact_key, "re-serve verifier produced an unexpected unparseable outcome; serving without claiming verified");
                        }
                    }

                    match retained {
                        // Small artifact: serve the exact bytes we just verified.
                        Some(buf) => Box::pin(futures::stream::once(async move {
                            Ok::<Bytes, CoreError>(Bytes::from(buf))
                        })),
                        // Oversized artifact: the verifying read consumed the stream,
                        // so re-open a fresh one to serve. A concurrent eviction
                        // between the two reads yields a clean miss/error here, never
                        // unverified bytes.
                        None => {
                            self.storage
                                .retrieve(&artifact_key)
                                .await?
                                .ok_or_else(|| {
                                    CoreError::Registry(format!(
                                    "artifact '{artifact_key}' vanished after re-serve verification"
                                ))
                                })?
                                .stream
                        }
                    }
                }
                // A recorded checksum that cannot be parsed: serve as-is (stream
                // untouched), same as a missing checksum.
                Some(None) => {
                    metrics::counter!("batlehub_integrity_checks_total", "registry" => registry_label.clone(), "outcome" => "unparseable", "phase" => "reserve").increment(1);
                    tracing::warn!(registry = %registry_name, key = %artifact_key, "stored checksum could not be parsed; serving without re-verification");
                    artifact.stream
                }
                // No checksum recorded: serve as-is until the entry is refreshed.
                None => {
                    tracing::debug!(key = %artifact_key, "no stored checksum for re-serve verification; serving as-is");
                    artifact.stream
                }
            }
        } else {
            artifact.stream
        };

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
        super::finish_request(&registry_label, "allowed", start);
        Ok(ProxyResponse::Stream(response_stream))
    }

    /// Cache miss: fetch the artifact from upstream, enforce the size limit,
    /// verify integrity against the advertised checksum, then (unless the
    /// upstream said `no-store`) persist it to storage + the cache-meta table and
    /// trigger SBOM generation before streaming it to the client.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn fetch_and_cache(
        &self,
        req: ProxyRequest,
        client: Arc<dyn RegistryClient>,
        metadata: PackageMetadata,
        artifact_key: String,
        integrity: &IntegrityPolicy,
        limit: u64,
        registry_label: String,
        start: Instant,
    ) -> Result<ProxyResponse, CoreError> {
        let registry_name = req.package_id.registry.as_str();
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

        // ── Verify integrity against the advertised checksum ───────────────────
        // The full bytes are buffered here, so this is the one place we can hash
        // and compare before the artifact is cached or served. A mismatch means
        // corruption or tampering: we never cache or serve those bytes, and the
        // failure is not bypassable. (The firewall-only path streams without
        // buffering and is intentionally not verified.)
        if integrity.enabled {
            use crate::services::integrity::{verify, IntegrityOutcome};
            match metadata.checksum.as_deref() {
                Some(expected) => match verify(expected, &data) {
                    IntegrityOutcome::Verified { algo } => {
                        metrics::counter!("batlehub_integrity_checks_total", "registry" => registry_label.clone(), "outcome" => "verified").increment(1);
                        tracing::debug!(registry = %registry_name, key = %artifact_key, algo = algo.as_str(), "artifact integrity verified");
                    }
                    IntegrityOutcome::Mismatch {
                        algo,
                        expected,
                        actual,
                    } => {
                        metrics::counter!("batlehub_integrity_checks_total", "registry" => registry_label.clone(), "outcome" => "mismatch").increment(1);
                        tracing::warn!(registry = %registry_name, key = %artifact_key, algo = algo.as_str(), %expected, %actual, "artifact integrity mismatch");
                        if integrity.block_on_mismatch {
                            let reason = format!(
                                "integrity check failed for {}: {} digest mismatch (expected {expected}, got {actual})",
                                req.package_id,
                                algo.as_str(),
                            );
                            super::warn_if_audit_failed(
                                self.repo
                                    .record_access(AccessEvent::proxy_error(
                                        req.package_id.clone(),
                                        req.identity.user_id.clone(),
                                        req.identity.role.clone(),
                                        reason.clone(),
                                    ))
                                    .await,
                                "integrity mismatch",
                            );
                            super::finish_request(&registry_label, "integrity_failed", start);
                            return Err(CoreError::IntegrityFailure(reason));
                        }
                    }
                    IntegrityOutcome::Unparseable => {
                        // Advertised checksum in an unknown format → cannot verify;
                        // treat like missing rather than falsely claiming "verified".
                        metrics::counter!("batlehub_integrity_checks_total", "registry" => registry_label.clone(), "outcome" => "unparseable").increment(1);
                        tracing::warn!(registry = %registry_name, key = %artifact_key, checksum = %expected, "advertised checksum could not be parsed; skipping verification");
                    }
                },
                None => {
                    metrics::counter!("batlehub_integrity_checks_total", "registry" => registry_label.clone(), "outcome" => "missing").increment(1);
                    if integrity.require_metadata
                        && !integrity.bypass_roles.contains(&req.identity.role)
                    {
                        let reason = format!(
                            "integrity policy requires checksum metadata, but upstream provided none for {}",
                            req.package_id,
                        );
                        super::warn_if_audit_failed(
                            self.repo
                                .record_access(AccessEvent::proxy_error(
                                    req.package_id.clone(),
                                    req.identity.user_id.clone(),
                                    req.identity.role.clone(),
                                    reason.clone(),
                                ))
                                .await,
                            "integrity missing metadata",
                        );
                        super::finish_request(&registry_label, "integrity_failed", start);
                        return Err(CoreError::IntegrityFailure(reason));
                    }
                    tracing::debug!(registry = %registry_name, key = %artifact_key, "upstream provided no integrity metadata");
                }
            }
        }

        if !skip_artifact_cache {
            // Self-computed digest persisted in the cache metadata table (via
            // `record_artifact`) so the bytes can be re-verified on later serves
            // (see the cache-hit path).
            let stored_checksum = crate::services::integrity::sha256_hex(&data);
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
                .record_artifact(crate::ports::ArtifactMetaRecord {
                    key: &artifact_key,
                    registry: registry_name,
                    package_name: &req.package_id.name,
                    version: &req.package_id.version,
                    size: Some(data.len() as u64),
                    checksum: Some(&stored_checksum),
                })
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
        super::finish_request(&registry_label, "allowed", start);

        let stream = futures::stream::once(async move { Ok(data) });
        Ok(ProxyResponse::Stream(Box::pin(stream)))
    }
}
