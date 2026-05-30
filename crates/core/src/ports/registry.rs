use std::pin::Pin;

use async_trait::async_trait;
use bytes::Bytes;
use futures::Stream;

use crate::entities::{PackageId, PackageMetadata};
use crate::error::CoreError;

pub type ArtifactStream = Pin<Box<dyn Stream<Item = Result<Bytes, CoreError>> + Send + 'static>>;

/// The result of fetching an artifact from an upstream registry, including the
/// byte stream and any `Cache-Control` header the upstream returned.
pub struct FetchedArtifact {
    pub stream: ArtifactStream,
    /// Raw `Cache-Control` header value from the upstream artifact response, if any.
    pub cache_control: Option<String>,
}

/// A lightweight package hit returned by upstream search.
#[derive(Debug, Clone)]
pub struct UpstreamPackage {
    pub name: String,
    pub latest_version: String,
    pub description: Option<String>,
}

/// A client for a specific upstream package registry.
///
/// Each registry type (GitHub, Cargo, npm, …) provides its own implementation.
/// Rule evaluation happens in `crates/core/src/rules/` using data returned by this trait.
#[async_trait]
pub trait RegistryClient: Send + Sync {
    /// Short identifier matching the `registry` field of `PackageId` (e.g. `"github"`).
    fn registry_type(&self) -> &str;

    /// Fetch metadata for a package from the upstream registry.
    ///
    /// Implementations should populate `PackageMetadata::published_at` and
    /// `PackageMetadata::is_signed` when the upstream provides that information,
    /// as the rule engine depends on them.
    async fn resolve_metadata(&self, pkg: &PackageId) -> Result<PackageMetadata, CoreError>;

    /// Stream the raw artifact bytes from the upstream registry, along with any
    /// upstream `Cache-Control` header.
    async fn fetch_artifact(&self, pkg: &PackageId) -> Result<FetchedArtifact, CoreError>;

    /// Return all known version strings for `package`, oldest-first.
    ///
    /// The default implementation returns an empty list; registries that do not
    /// support version enumeration (e.g. GitHub Releases, OpenVSX) can rely on it.
    async fn list_versions(&self, package: &str) -> Result<Vec<String>, CoreError> {
        let _ = package;
        Ok(vec![])
    }

    /// Search the upstream registry for packages matching `query`.
    ///
    /// Returns up to `limit` results. The default implementation returns an empty
    /// list; registries without a search API (e.g. GitHub, Go) can rely on it.
    async fn search_packages(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<UpstreamPackage>, CoreError> {
        let _ = (query, limit);
        Ok(vec![])
    }
}
