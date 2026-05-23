use async_trait::async_trait;

use crate::entities::{AccessEvent, EventFilter, PackageFilter, PackageId, PackageStatus, PackageSummary};
use crate::error::CoreError;

/// Persistent store for package statuses and access audit logs.
///
/// Backed by a relational database (PostgreSQL, MySQL, …).
#[async_trait]
pub trait PackageRepository: Send + Sync {
    /// Record an access event (download attempt, block action, etc.).
    async fn record_access(&self, event: AccessEvent) -> Result<(), CoreError>;

    /// Get the current administrative status of a package.
    /// Returns `PackageStatus::Available` if the package has never been seen.
    async fn get_status(&self, pkg: &PackageId) -> Result<PackageStatus, CoreError>;

    /// Update the administrative status of a package.
    async fn set_status(&self, pkg: &PackageId, status: PackageStatus) -> Result<(), CoreError>;

    /// List all known packages with optional filtering and pagination.
    async fn list_packages(&self, filter: PackageFilter) -> Result<Vec<PackageSummary>, CoreError>;

    /// Count matching packages without applying `limit`/`offset`. Used for accurate pagination totals.
    async fn count_packages(&self, filter: PackageFilter) -> Result<u64, CoreError>;

    /// Query the access event log.
    async fn list_events(&self, filter: EventFilter) -> Result<Vec<AccessEvent>, CoreError>;
}
