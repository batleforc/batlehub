use std::sync::Arc;

use async_trait::async_trait;

use proxy_cache_core::{
    entities::{PackageId, PackageMetadata},
    error::CoreError,
    ports::{ArtifactStream, RegistryClient},
};

/// Tries a list of upstream clients in priority order.
///
/// On `CoreError::NotFound` the next client is tried; any other error is
/// propagated immediately. If every client returns `NotFound` the last error
/// is returned.
pub struct FanoutRegistryClient {
    clients: Vec<Arc<dyn RegistryClient>>,
    registry_type: String,
}

impl FanoutRegistryClient {
    pub fn new(
        registry_type: impl Into<String>,
        clients: Vec<Arc<dyn RegistryClient>>,
    ) -> Self {
        Self { registry_type: registry_type.into(), clients }
    }
}

#[async_trait]
impl RegistryClient for FanoutRegistryClient {
    fn registry_type(&self) -> &str {
        &self.registry_type
    }

    async fn resolve_metadata(&self, pkg: &PackageId) -> Result<PackageMetadata, CoreError> {
        let mut last = CoreError::NotFound(format!("{} not found in any upstream", pkg.name));
        for client in &self.clients {
            match client.resolve_metadata(pkg).await {
                Ok(meta) => return Ok(meta),
                Err(CoreError::NotFound(msg)) => {
                    tracing::debug!(upstream = client.registry_type(), %msg, "upstream miss, trying next");
                    last = CoreError::NotFound(msg);
                }
                Err(e) => return Err(e),
            }
        }
        Err(last)
    }

    async fn fetch_artifact(&self, pkg: &PackageId) -> Result<ArtifactStream, CoreError> {
        let mut last = CoreError::NotFound(format!("{} artifact not found in any upstream", pkg.name));
        for client in &self.clients {
            match client.fetch_artifact(pkg).await {
                Ok(stream) => return Ok(stream),
                Err(CoreError::NotFound(msg)) => {
                    tracing::debug!(upstream = client.registry_type(), %msg, "upstream miss, trying next");
                    last = CoreError::NotFound(msg);
                }
                Err(e) => return Err(e),
            }
        }
        Err(last)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use futures::TryStreamExt;

    use super::*;

    // ── Mock ──────────────────────────────────────────────────────────────────

    /// What a mock upstream returns for each method.
    #[derive(Clone, Copy)]
    enum MockOutcome {
        /// Return a successful result.
        Hit,
        /// Return `CoreError::NotFound` (fanout should skip to the next upstream).
        Miss,
        /// Return a hard `CoreError::Registry` error (fanout should stop immediately).
        Fail,
    }

    struct MockRegistry {
        label: &'static str,
        meta: MockOutcome,
        artifact: MockOutcome,
    }

    impl MockRegistry {
        fn new(label: &'static str, meta: MockOutcome, artifact: MockOutcome) -> Arc<Self> {
            Arc::new(Self { label, meta, artifact })
        }
    }

    fn dummy_meta(pkg: &PackageId) -> PackageMetadata {
        PackageMetadata {
            id: pkg.clone(),
            published_at: None,
            download_url: None,
            checksum: None,
            is_signed: None,
            extra: serde_json::Value::Null,
        }
    }

    fn dummy_stream() -> ArtifactStream {
        let stream = futures::stream::once(async {
            Ok::<Bytes, CoreError>(Bytes::from_static(b"artifact-data"))
        });
        Box::pin(stream)
    }

    #[async_trait]
    impl RegistryClient for MockRegistry {
        fn registry_type(&self) -> &str {
            self.label
        }

        async fn resolve_metadata(&self, pkg: &PackageId) -> Result<PackageMetadata, CoreError> {
            match self.meta {
                MockOutcome::Hit  => Ok(dummy_meta(pkg)),
                MockOutcome::Miss => Err(CoreError::NotFound(
                    format!("[{}] {} not found", self.label, pkg.name),
                )),
                MockOutcome::Fail => Err(CoreError::Registry(
                    format!("[{}] internal error", self.label),
                )),
            }
        }

        async fn fetch_artifact(&self, pkg: &PackageId) -> Result<ArtifactStream, CoreError> {
            match self.artifact {
                MockOutcome::Hit  => Ok(dummy_stream()),
                MockOutcome::Miss => Err(CoreError::NotFound(
                    format!("[{}] {} artifact not found", self.label, pkg.name),
                )),
                MockOutcome::Fail => Err(CoreError::Registry(
                    format!("[{}] internal error", self.label),
                )),
            }
        }
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn pkg() -> PackageId {
        PackageId::new("npm", "lodash", "4.17.21")
    }

    fn fanout(clients: Vec<Arc<dyn RegistryClient>>) -> FanoutRegistryClient {
        FanoutRegistryClient::new("npm", clients)
    }

    // ── resolve_metadata tests ────────────────────────────────────────────────

    #[tokio::test]
    async fn meta_first_upstream_hits() {
        let f = fanout(vec![
            MockRegistry::new("private", MockOutcome::Hit, MockOutcome::Miss),
            MockRegistry::new("public",  MockOutcome::Hit, MockOutcome::Miss),
        ]);
        // Should return immediately from "private" — "public" is never called.
        assert!(f.resolve_metadata(&pkg()).await.is_ok());
    }

    #[tokio::test]
    async fn meta_falls_through_to_second_on_not_found() {
        let f = fanout(vec![
            MockRegistry::new("private", MockOutcome::Miss, MockOutcome::Miss),
            MockRegistry::new("public",  MockOutcome::Hit,  MockOutcome::Miss),
        ]);
        let result = f.resolve_metadata(&pkg()).await;
        assert!(result.is_ok(), "expected hit from public fallback, got {result:?}");
    }

    #[tokio::test]
    async fn meta_all_miss_returns_not_found() {
        let f = fanout(vec![
            MockRegistry::new("private", MockOutcome::Miss, MockOutcome::Miss),
            MockRegistry::new("public",  MockOutcome::Miss, MockOutcome::Miss),
        ]);
        let err = f.resolve_metadata(&pkg()).await.unwrap_err();
        assert!(matches!(err, CoreError::NotFound(_)), "expected NotFound, got {err:?}");
    }

    #[tokio::test]
    async fn meta_hard_error_stops_fanout() {
        let f = fanout(vec![
            MockRegistry::new("private", MockOutcome::Fail, MockOutcome::Miss),
            MockRegistry::new("public",  MockOutcome::Hit,  MockOutcome::Miss),
        ]);
        // A Registry error from the first upstream must NOT fall through.
        let err = f.resolve_metadata(&pkg()).await.unwrap_err();
        assert!(matches!(err, CoreError::Registry(_)), "expected Registry error, got {err:?}");
    }

    #[tokio::test]
    async fn meta_miss_then_hard_error_propagates() {
        let f = fanout(vec![
            MockRegistry::new("private", MockOutcome::Miss, MockOutcome::Miss),
            MockRegistry::new("public",  MockOutcome::Fail, MockOutcome::Miss),
        ]);
        let err = f.resolve_metadata(&pkg()).await.unwrap_err();
        assert!(matches!(err, CoreError::Registry(_)), "expected Registry error, got {err:?}");
    }

    #[tokio::test]
    async fn meta_empty_client_list_returns_not_found() {
        let f = fanout(vec![]);
        let err = f.resolve_metadata(&pkg()).await.unwrap_err();
        assert!(matches!(err, CoreError::NotFound(_)));
    }

    // ── fetch_artifact tests ──────────────────────────────────────────────────

    #[tokio::test]
    async fn artifact_first_upstream_hits() {
        let f = fanout(vec![
            MockRegistry::new("private", MockOutcome::Miss, MockOutcome::Hit),
            MockRegistry::new("public",  MockOutcome::Miss, MockOutcome::Hit),
        ]);
        assert!(f.fetch_artifact(&pkg()).await.is_ok());
    }

    #[tokio::test]
    async fn artifact_falls_through_to_second_on_not_found() {
        let f = fanout(vec![
            MockRegistry::new("private", MockOutcome::Miss, MockOutcome::Miss),
            MockRegistry::new("public",  MockOutcome::Miss, MockOutcome::Hit),
        ]);
        let stream = f.fetch_artifact(&pkg()).await.expect("expected stream from public fallback");
        let chunks: Vec<_> = stream.try_collect().await.expect("stream should yield data");
        assert_eq!(chunks, vec![Bytes::from_static(b"artifact-data")]);
    }

    #[tokio::test]
    async fn artifact_all_miss_returns_not_found() {
        let f = fanout(vec![
            MockRegistry::new("private", MockOutcome::Miss, MockOutcome::Miss),
            MockRegistry::new("public",  MockOutcome::Miss, MockOutcome::Miss),
        ]);
        let Err(err) = f.fetch_artifact(&pkg()).await else { panic!("expected error") };
        assert!(matches!(err, CoreError::NotFound(_)), "expected NotFound, got {err:?}");
    }

    #[tokio::test]
    async fn artifact_hard_error_stops_fanout() {
        let f = fanout(vec![
            MockRegistry::new("private", MockOutcome::Miss, MockOutcome::Fail),
            MockRegistry::new("public",  MockOutcome::Miss, MockOutcome::Hit),
        ]);
        let Err(err) = f.fetch_artifact(&pkg()).await else { panic!("expected error") };
        assert!(matches!(err, CoreError::Registry(_)), "expected Registry error, got {err:?}");
    }

    #[tokio::test]
    async fn artifact_empty_client_list_returns_not_found() {
        let f = fanout(vec![]);
        let Err(err) = f.fetch_artifact(&pkg()).await else { panic!("expected error") };
        assert!(matches!(err, CoreError::NotFound(_)));
    }
}
