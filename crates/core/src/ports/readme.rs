use async_trait::async_trait;

use crate::entities::PackageReadme;
use crate::error::CoreError;

/// Durable storage for the README of a version this instance **holds bytes for
/// or hosts**.
///
/// Nothing here holds a row for a version the instance only knows about from an
/// upstream document: that answer is derived on read from the cached document
/// (RFC 0007 §5.6). A row written because somebody looked at a page would have
/// nothing that ever deletes it — deletion keys on a version being deleted, and
/// a version never held here is never deleted.
#[async_trait]
pub trait ReadmeRepository: Send + Sync {
    /// Store or replace the README for one coordinate.
    async fn upsert(&self, readme: PackageReadme) -> Result<(), CoreError>;

    /// The README for an exact coordinate, or `None` when none is stored.
    async fn get(
        &self,
        registry: &str,
        name: &str,
        version: &str,
    ) -> Result<Option<PackageReadme>, CoreError>;

    /// The newest version of this package that has a README, excluding the
    /// versions the caller names.
    ///
    /// `exclude_versions` is how the fallback rule keeps blocked and unlisted
    /// versions from becoming a fallback source (RFC 0007 §4.4): the repository
    /// knows nothing about firewall state, so the caller — which does — hands
    /// the exclusions down rather than the store growing a policy opinion.
    ///
    /// "Newest" is decided by the caller's ordering, not the store's: version
    /// strings do not sort as versions in SQL. The store returns candidates
    /// newest-*recorded* first and the service picks.
    async fn get_latest_with_readme(
        &self,
        registry: &str,
        name: &str,
        exclude_versions: &[String],
    ) -> Result<Option<PackageReadme>, CoreError>;

    /// One `(version, state)` pair per version of this package that has a
    /// stored README.
    ///
    /// One query for the whole version table, rather than a probe per row: the
    /// detail response carries a `readme` state for every version it lists, and
    /// a lookup per version would be N round trips for a page load.
    ///
    /// Versions absent from the result have no stored README; whether that is
    /// [`ReadmeState::None`] or [`ReadmeState::Unknown`] is a question about the
    /// registry kind and what bytes are held, which the caller answers.
    async fn list_versions_with_readme(
        &self,
        registry: &str,
        name: &str,
    ) -> Result<Vec<String>, CoreError>;

    /// Remove the README for one version, on local delete.
    async fn delete_for_version(
        &self,
        registry: &str,
        name: &str,
        version: &str,
    ) -> Result<(), CoreError>;

    /// Remove every README for a package, on package delete.
    async fn delete_for_package(&self, registry: &str, name: &str) -> Result<(), CoreError>;
}
