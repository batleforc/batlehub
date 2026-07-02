//! A generic path-passthrough registry client for repository formats that are
//! addressed purely by file path (Debian APT, RPM/YUM). The full upstream path is
//! carried in `PackageId::artifact`; `fetch_artifact` simply streams
//! `{base_url}/{artifact}`. Metadata resolution is a no-op (these formats have no
//! per-package metadata API — the index files *are* the metadata, fetched as
//! ordinary artifacts).
//!
//! Used for the `deb` and `rpm` registry types; the concrete type string is
//! supplied at construction so a single implementation serves both.

use async_trait::async_trait;
use futures::TryStreamExt;

use batlehub_core::{
    entities::{PackageId, PackageMetadata},
    error::CoreError,
    ports::{FetchedArtifact, RegistryClient},
};

use super::http_client::{
    apply_upstream_options, basic_auth_get, to_registry_error, UpstreamHttpOptions,
};

pub struct PathProxyRegistryClient {
    registry_type: String,
    http: reqwest::Client,
    /// Upstream repository root (no trailing slash).
    base_url: String,
    basic_auth: Option<(String, String)>,
}

impl PathProxyRegistryClient {
    pub fn new(
        registry_type: impl Into<String>,
        base_url: impl Into<String>,
        opts: &UpstreamHttpOptions,
    ) -> anyhow::Result<Self> {
        let http =
            apply_upstream_options(reqwest::Client::builder().user_agent("batlehub/0.1"), opts)?;
        Ok(Self {
            registry_type: registry_type.into(),
            http,
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            basic_auth: opts.basic_auth.clone(),
        })
    }

    /// The upstream path is whatever the handler placed in `artifact`.
    fn upstream_path(pkg: &PackageId) -> Result<&str, CoreError> {
        pkg.artifact.as_deref().ok_or_else(|| {
            CoreError::Registry("path-proxy fetch requires PackageId::artifact".to_owned())
        })
    }
}

#[async_trait]
impl RegistryClient for PathProxyRegistryClient {
    fn registry_type(&self) -> &str {
        &self.registry_type
    }

    async fn resolve_metadata(&self, pkg: &PackageId) -> Result<PackageMetadata, CoreError> {
        // No metadata API; the requested file is fetched directly as an artifact.
        Ok(PackageMetadata {
            id: pkg.clone(),
            published_at: None,
            download_url: None,
            checksum: None,
            is_signed: None,
            extra: serde_json::Value::Null,
            cache_control: None,
        })
    }

    async fn fetch_artifact(&self, pkg: &PackageId) -> Result<FetchedArtifact, CoreError> {
        let path = Self::upstream_path(pkg)?;
        let url = format!("{}/{}", self.base_url, path);
        tracing::debug!(url = %url, "fetching {} artifact", self.registry_type);

        let resp = basic_auth_get(&self.http, &self.basic_auth, &url)
            .send()
            .await
            .map_err(to_registry_error)?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(CoreError::NotFound(format!("{path} not found upstream")));
        }
        let resp = resp.error_for_status().map_err(to_registry_error)?;

        let cache_control = resp
            .headers()
            .get("cache-control")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);

        let stream = resp.bytes_stream().map_err(to_registry_error);
        Ok(FetchedArtifact {
            stream: Box::pin(stream),
            cache_control,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fetches_artifact_by_path() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("GET", "/dists/stable/Release")
            .with_status(200)
            .with_body("Origin: Debian\n")
            .create_async()
            .await;

        let client =
            PathProxyRegistryClient::new("deb", server.url(), &UpstreamHttpOptions::default())
                .unwrap();
        let pkg = PackageId::new("apt", "repo", "_").with_artifact("dists/stable/Release");
        let fetched = client.fetch_artifact(&pkg).await.unwrap();
        let body: Vec<u8> = fetched
            .stream
            .try_fold(Vec::new(), |mut acc, chunk| async move {
                acc.extend_from_slice(&chunk);
                Ok(acc)
            })
            .await
            .unwrap();
        assert_eq!(body, b"Origin: Debian\n");
    }

    #[tokio::test]
    async fn missing_path_is_not_found() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("GET", "/repodata/repomd.xml")
            .with_status(404)
            .create_async()
            .await;
        let client =
            PathProxyRegistryClient::new("rpm", server.url(), &UpstreamHttpOptions::default())
                .unwrap();
        let pkg = PackageId::new("yum", "repo", "_").with_artifact("repodata/repomd.xml");
        match client.fetch_artifact(&pkg).await {
            Err(e) => assert!(matches!(e, CoreError::NotFound(_))),
            Ok(_) => panic!("expected NotFound"),
        }
    }

    #[test]
    fn registry_type_is_configurable() {
        let c = PathProxyRegistryClient::new("rpm", "http://x", &UpstreamHttpOptions::default())
            .unwrap();
        assert_eq!(c.registry_type(), "rpm");
    }
}
