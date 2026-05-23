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
}
