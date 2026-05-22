use async_trait::async_trait;

use crate::{entities::PublishedPackage, error::CoreError};

/// Authoritative storage for packages published directly to BatleHub.
///
/// Each method is scoped to a `registry` name so one instance serves all
/// local registries. Index metadata is ecosystem-specific opaque JSON stored
/// inside `PublishedPackage::index_metadata`.
#[async_trait]
pub trait LocalRegistryBackend: Send + Sync {
    /// Persist a new version. Returns `CoreError::Conflict` if the version
    /// already exists (registries disallow overwriting a published version).
    async fn publish(&self, pkg: PublishedPackage) -> Result<(), CoreError>;

    /// Mark a version as yanked. Also updates `index_metadata.yanked`.
    async fn yank(&self, registry: &str, name: &str, version: &str) -> Result<(), CoreError>;

    /// Reverse a yank. Also updates `index_metadata.yanked`.
    async fn unyank(&self, registry: &str, name: &str, version: &str) -> Result<(), CoreError>;

    /// Return all versions of `name` in `registry`, sorted by `published_at` ASC.
    /// Returns an empty vec (not an error) when the crate has never been published.
    async fn get_versions(
        &self,
        registry: &str,
        name: &str,
    ) -> Result<Vec<PublishedPackage>, CoreError>;

    /// Return `true` if at least one version of `name` exists in `registry`.
    async fn exists(&self, registry: &str, name: &str) -> Result<bool, CoreError>;
}
