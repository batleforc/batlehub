use async_trait::async_trait;

use crate::{entities::Identity, error::CoreError};

/// An owner entry in the `package_owners` table.
#[derive(Debug, Clone)]
pub struct OwnerEntry {
    pub principal_type: String, // "user" or "group"
    pub principal_id: String,
    pub role: String, // "admin" or "maintainer"
    pub granted_by: Option<String>,
}

/// Port for per-package ownership management.
///
/// On first publish, call `initialize_owner` to make the publisher the package admin.
/// Before subsequent publishes, call `can_publish` to verify the caller is an owner.
#[async_trait]
pub trait OwnershipPort: Send + Sync {
    /// Grant `user_id` the 'admin' role on `package` in `registry`.
    /// Called exactly once: when the first version of a package is published.
    /// Silently succeeds if an owner row for this user already exists (idempotent).
    async fn initialize_owner(
        &self,
        registry: &str,
        package: &str,
        user_id: &str,
    ) -> Result<(), CoreError>;

    /// Return `true` if `identity` is allowed to publish `package` in `registry`.
    ///
    /// Returns `true` when:
    /// - The package has no owner rows yet (new package — anyone with User role may publish).
    /// - The identity's `user_id` has a row for this package, OR
    /// - Any group in `identity.groups` has a row for this package.
    async fn can_publish(
        &self,
        registry: &str,
        package: &str,
        identity: &Identity,
    ) -> Result<bool, CoreError>;

    /// Add an owner entry. Returns `CoreError::Conflict` if already present.
    async fn add_owner(
        &self,
        registry: &str,
        package: &str,
        entry: OwnerEntry,
    ) -> Result<(), CoreError>;

    /// Remove an owner entry. Succeeds even if the entry does not exist.
    async fn remove_owner(
        &self,
        registry: &str,
        package: &str,
        principal_type: &str,
        principal_id: &str,
    ) -> Result<(), CoreError>;

    /// List all owners of a package, ordered by `granted_at` ascending.
    async fn list_owners(
        &self,
        registry: &str,
        package: &str,
    ) -> Result<Vec<OwnerEntry>, CoreError>;
}
