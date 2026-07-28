//! A generic path-passthrough registry client for repository formats that are
//! addressed purely by file path (Debian APT, RPM/YUM). The full upstream path is
//! carried in `PackageId::artifact`; `fetch_artifact` simply streams
//! `{base_url}/{artifact}`. Metadata resolution is a no-op (these formats have no
//! per-package metadata API — the index files *are* the metadata, fetched as
//! ordinary artifacts).
//!
//! Used for the `deb`, `rpm`, `pacman`, `jetbrains` and `generic` registry types;
//! the concrete type string is supplied at construction so a single
//! implementation serves them all.

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
    /// Compiled `path_allow` globs. Empty means "no allowlist configured", which
    /// allows everything — the mandatory-allowlist rule for `generic` registries
    /// is enforced by config validation, not here, so the deb/rpm/pacman/jetbrains
    /// kinds keep their existing unrestricted behaviour by default.
    path_allow: Vec<glob::Pattern>,
}

impl PathProxyRegistryClient {
    pub fn new(
        registry_type: impl Into<String>,
        base_url: impl Into<String>,
        opts: &UpstreamHttpOptions,
    ) -> Result<Self, CoreError> {
        let http =
            apply_upstream_options(reqwest::Client::builder().user_agent("batlehub/0.1"), opts)?;
        Ok(Self {
            registry_type: registry_type.into(),
            http,
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            basic_auth: opts.basic_auth.clone(),
            path_allow: Vec::new(),
        })
    }

    /// Restrict this client to upstream paths matching one of `patterns`.
    ///
    /// Invalid globs are rejected here rather than silently dropped — a pattern
    /// that fails to compile would otherwise shrink the allowlist, and a
    /// too-small allowlist looks like a 403 bug while a silently-dropped one
    /// looks like nothing at all. Config validation compiles the same patterns
    /// at startup, so this is a defence-in-depth check.
    pub fn with_path_allow(mut self, patterns: &[String]) -> Result<Self, CoreError> {
        self.path_allow = patterns
            .iter()
            .map(|p| {
                glob::Pattern::new(p)
                    .map_err(|e| CoreError::Registry(format!("invalid path_allow glob '{p}': {e}")))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(self)
    }

    /// The upstream path is whatever the handler placed in `artifact`.
    fn upstream_path(pkg: &PackageId) -> Result<&str, CoreError> {
        pkg.artifact.as_deref().ok_or_else(|| {
            CoreError::Registry("path-proxy fetch requires PackageId::artifact".to_owned())
        })
    }

    /// Reject paths outside the configured allowlist before any upstream request
    /// is made. Denials are `AccessDenied` (→ 403) rather than `NotFound`, so an
    /// operator sees "your allowlist blocked this" instead of chasing a phantom
    /// upstream 404.
    fn check_path_allowed(&self, path: &str) -> Result<(), CoreError> {
        if self.path_allow.is_empty() || self.path_allow.iter().any(|p| p.matches(path)) {
            return Ok(());
        }
        Err(CoreError::AccessDenied(format!(
            "path '{path}' is not in this registry's path_allow allowlist"
        )))
    }
}

#[async_trait]
impl RegistryClient for PathProxyRegistryClient {
    fn registry_type(&self) -> &str {
        &self.registry_type
    }

    async fn resolve_metadata(&self, pkg: &PackageId) -> Result<PackageMetadata, CoreError> {
        // Deny disallowed paths here too, not just in `fetch_artifact`: metadata
        // resolution runs first and is cached, so a path outside the allowlist
        // must never make it as far as a cache entry.
        if let Some(path) = pkg.artifact.as_deref() {
            self.check_path_allowed(path)?;
        }
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
        self.check_path_allowed(path)?;
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

    fn generic_with_allow(patterns: &[&str]) -> PathProxyRegistryClient {
        let owned: Vec<String> = patterns.iter().map(|p| (*p).to_owned()).collect();
        PathProxyRegistryClient::new("generic", "http://x", &UpstreamHttpOptions::default())
            .unwrap()
            .with_path_allow(&owned)
            .unwrap()
    }

    #[test]
    fn empty_path_allow_permits_everything() {
        let c = generic_with_allow(&[]);
        assert!(c.check_path_allowed("anything/at/all.tar.gz").is_ok());
    }

    #[test]
    fn path_allow_permits_matching_path() {
        let c = generic_with_allow(&["v*/node-v*-linux-x64.tar.gz"]);
        assert!(c
            .check_path_allowed("v24.18.0/node-v24.18.0-linux-x64.tar.gz")
            .is_ok());
    }

    #[test]
    fn path_allow_denies_non_matching_path() {
        let c = generic_with_allow(&["v*/node-v*-linux-x64.tar.gz"]);
        match c.check_path_allowed("../../etc/passwd") {
            Err(CoreError::AccessDenied(_)) => {}
            other => panic!("expected AccessDenied, got {other:?}"),
        }
    }

    #[test]
    fn path_allow_double_star_is_the_allow_everything_opt_out() {
        let c = generic_with_allow(&["**"]);
        assert!(c.check_path_allowed("some/deep/nested/file.bin").is_ok());
    }

    #[test]
    fn path_allow_matches_any_of_several_patterns() {
        let c = generic_with_allow(&["helm-v*-linux-amd64.tar.gz", "helm-v*-darwin-arm64.tar.gz"]);
        assert!(c
            .check_path_allowed("helm-v4.2.3-linux-amd64.tar.gz")
            .is_ok());
        assert!(c
            .check_path_allowed("helm-v4.2.3-darwin-arm64.tar.gz")
            .is_ok());
        assert!(c
            .check_path_allowed("helm-v4.2.3-windows-amd64.zip")
            .is_err());
    }

    #[test]
    fn invalid_path_allow_glob_is_rejected() {
        let bad = vec!["[unclosed".to_owned()];
        let res =
            PathProxyRegistryClient::new("generic", "http://x", &UpstreamHttpOptions::default())
                .unwrap()
                .with_path_allow(&bad);
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn disallowed_path_is_denied_before_any_upstream_request() {
        // No mock is registered: a request that reached the network would 501/error
        // out differently, so reaching AccessDenied proves nothing was sent.
        let server = mockito::Server::new_async().await;
        let client =
            PathProxyRegistryClient::new("generic", server.url(), &UpstreamHttpOptions::default())
                .unwrap()
                .with_path_allow(&["allowed/*".to_owned()])
                .unwrap();
        let pkg = PackageId::new("files", "repo", "_").with_artifact("denied/secret.bin");
        match client.fetch_artifact(&pkg).await {
            Err(CoreError::AccessDenied(_)) => {}
            other => panic!("expected AccessDenied, got {:?}", other.map(|_| "ok")),
        }
    }

    #[tokio::test]
    async fn resolve_metadata_denies_disallowed_path() {
        let client =
            PathProxyRegistryClient::new("generic", "http://x", &UpstreamHttpOptions::default())
                .unwrap()
                .with_path_allow(&["allowed/*".to_owned()])
                .unwrap();
        let pkg = PackageId::new("files", "repo", "_").with_artifact("denied/secret.bin");
        match client.resolve_metadata(&pkg).await {
            Err(CoreError::AccessDenied(_)) => {}
            other => panic!("expected AccessDenied, got {:?}", other.map(|_| "ok")),
        }
    }
}
