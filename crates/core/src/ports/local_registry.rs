use std::time::Duration;

use async_trait::async_trait;

use crate::{entities::PublishedPackage, error::CoreError};

/// Authoritative storage for packages published directly to BatleHub.
///
/// Each method is scoped to a `registry` name so one instance serves all
/// local registries. Index metadata is ecosystem-specific opaque JSON stored
/// inside `PublishedPackage::index_metadata`.
///
/// ## Transactional publish protocol
///
/// To survive a hard crash between the index write and the artifact write,
/// callers must use the three-step protocol:
///
/// 1. `publish(pkg)` — reserve the version; implementations may insert in a
///    *pending* state invisible to `get_versions`/`exists`.
/// 2. Write the artifact bytes to `StorageBackend`.
/// 3. `commit_publish(registry, name, version)` — promote the row to the
///    visible *published* state.
///
/// On any failure after step 1, call `remove_version` to clean up the pending
/// row.  Hard-crashed pending rows are recovered by `cleanup_pending`.
#[async_trait]
pub trait LocalRegistryBackend: Send + Sync {
    /// Reserve a new version. Returns `CoreError::Conflict` if a *published*
    /// version already exists. Implementations may insert in a *pending* state
    /// that is invisible to `get_versions` and `exists` until `commit_publish`
    /// is called.
    async fn publish(&self, pkg: PublishedPackage) -> Result<(), CoreError>;

    /// Promote a previously `publish`-ed row to the visible *published* state.
    /// Called after artifact storage succeeds. The default no-op is correct for
    /// backends that insert in published state directly (e.g. in-memory mocks).
    async fn commit_publish(
        &self,
        _registry: &str,
        _name: &str,
        _version: &str,
    ) -> Result<(), CoreError> {
        Ok(())
    }

    /// Mark a version as yanked. Also updates `index_metadata.yanked`.
    async fn yank(&self, registry: &str, name: &str, version: &str) -> Result<(), CoreError>;

    /// Reverse a yank. Also updates `index_metadata.yanked`.
    async fn unyank(&self, registry: &str, name: &str, version: &str) -> Result<(), CoreError>;

    /// Return all versions of `name` in `registry`, sorted by `published_at` ASC.
    /// Returns an empty vec (not an error) when the crate has never been published.
    /// Must only return rows in the *published* state.
    async fn get_versions(
        &self,
        registry: &str,
        name: &str,
    ) -> Result<Vec<PublishedPackage>, CoreError>;

    /// Return `true` if at least one *published* version of `name` exists in `registry`.
    async fn exists(&self, registry: &str, name: &str) -> Result<bool, CoreError>;

    /// Remove an exact version record from the index regardless of its state.
    /// Used to roll back a partially completed publish. Implementations that
    /// cannot support this operation should return `Ok(())` (best-effort).
    async fn remove_version(
        &self,
        _registry: &str,
        _name: &str,
        _version: &str,
    ) -> Result<(), CoreError> {
        Ok(())
    }

    /// Delete *pending* rows that were created before `older_than` ago.
    /// These are left by hard crashes between `publish` and `commit_publish`.
    /// Returns the number of rows deleted. The default no-op is correct for
    /// backends that have no pending state.
    async fn cleanup_pending(&self, _older_than: Duration) -> Result<u64, CoreError> {
        Ok(0)
    }

    /// Return the distinct package names published in `registry`.
    /// Used to build registry index files (e.g. Composer `packages.json`).
    /// The default implementation returns an empty vec.
    async fn list_package_names(&self, _registry: &str) -> Result<Vec<String>, CoreError> {
        Ok(vec![])
    }

    /// Yank multiple versions in one call.
    /// The default implementation loops over `yank`. Override for efficiency.
    async fn bulk_yank(
        &self,
        registry: &str,
        items: &[(String, String)],
    ) -> Result<BulkResult, CoreError> {
        let mut result = BulkResult {
            processed: items.len(),
            succeeded: 0,
            failed: vec![],
        };
        for (name, version) in items {
            match self.yank(registry, name, version).await {
                Ok(()) => result.succeeded += 1,
                Err(e) => result
                    .failed
                    .push((name.clone(), version.clone(), e.to_string())),
            }
        }
        Ok(result)
    }

    /// Unyank multiple versions in one call.
    /// The default implementation loops over `unyank`. Override for efficiency.
    async fn bulk_unyank(
        &self,
        registry: &str,
        items: &[(String, String)],
    ) -> Result<BulkResult, CoreError> {
        let mut result = BulkResult {
            processed: items.len(),
            succeeded: 0,
            failed: vec![],
        };
        for (name, version) in items {
            match self.unyank(registry, name, version).await {
                Ok(()) => result.succeeded += 1,
                Err(e) => result
                    .failed
                    .push((name.clone(), version.clone(), e.to_string())),
            }
        }
        Ok(result)
    }

    /// Permanently delete multiple versions in one call.
    /// The default implementation loops over `remove_version`. Override for efficiency.
    async fn bulk_remove_versions(
        &self,
        registry: &str,
        items: &[(String, String)],
    ) -> Result<BulkResult, CoreError> {
        let mut result = BulkResult {
            processed: items.len(),
            succeeded: 0,
            failed: vec![],
        };
        for (name, version) in items {
            match self.remove_version(registry, name, version).await {
                Ok(()) => result.succeeded += 1,
                Err(e) => result
                    .failed
                    .push((name.clone(), version.clone(), e.to_string())),
            }
        }
        Ok(result)
    }
}

/// Result of a bulk yank/unyank/delete operation.
#[derive(Debug)]
pub struct BulkResult {
    /// Total items submitted.
    pub processed: usize,
    /// Items processed without error.
    pub succeeded: usize,
    /// Items that failed: (name, version, error message).
    pub failed: Vec<(String, String, String)>,
}
