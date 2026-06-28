use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::entities::{
    AccessEvent, EventFilter, ExploreEntry, ExploreFilter, PackageFilter, PackageId, PackageStatus,
    PackageSummary, RegistryStat,
};
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

    /// Remove a package's administrative record entirely.
    /// Returns `true` if a row was found and deleted, `false` if it did not exist.
    async fn delete_package(&self, pkg: &PackageId) -> Result<bool, CoreError>;

    /// List all known packages with optional filtering and pagination.
    async fn list_packages(&self, filter: PackageFilter) -> Result<Vec<PackageSummary>, CoreError>;

    /// Count matching packages without applying `limit`/`offset`. Used for accurate pagination totals.
    async fn count_packages(&self, filter: PackageFilter) -> Result<u64, CoreError>;

    /// Query the access event log.
    async fn list_events(&self, filter: EventFilter) -> Result<Vec<AccessEvent>, CoreError>;

    /// Delete access-event rows older than `before`. Returns the number of rows deleted.
    async fn purge_events_before(&self, before: DateTime<Utc>) -> Result<u64, CoreError> {
        let _ = before;
        Ok(0)
    }

    /// Explorer: collapsed list of packages (one entry per name) from both proxied and local sources.
    async fn explore_packages(
        &self,
        filter: ExploreFilter,
    ) -> Result<Vec<ExploreEntry>, CoreError> {
        let _ = filter;
        Ok(vec![])
    }

    /// Explorer: count of unique (registry, name) pairs matching the filter.
    async fn count_explore_packages(&self, filter: ExploreFilter) -> Result<u64, CoreError> {
        let _ = filter;
        Ok(0)
    }

    /// Explorer: per-registry package counts and download totals.
    async fn registry_explore_stats(
        &self,
        accessible_registries: &[String],
    ) -> Result<Vec<RegistryStat>, CoreError> {
        let _ = accessible_registries;
        Ok(vec![])
    }
}
