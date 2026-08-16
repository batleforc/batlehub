use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::entities::{
    AccessEvent, EventFilter, ExploreEntry, ExploreFilter, PackageFilter, PackageId, PackageStatus,
    PackageSummary, RegistryStat,
};
use crate::error::CoreError;

/// A single row from the `access_events` "recent errors" query surfaced on the
/// admin health dashboard: a denied or upstream-error event for a registry.
#[derive(Debug, Clone)]
pub struct RecentErrorRecord {
    pub created_at: DateTime<Utc>,
    pub user_id: Option<String>,
    pub package_name: String,
    pub package_version: String,
    /// "denied" or "error".
    pub outcome: String,
    pub deny_reason: Option<String>,
}

/// Page size the default [`PackageRepository::blocked_versions`] asks for. A
/// package with more blocked versions than this would have the excess treated as
/// unblocked, so it is set far above any plausible real count rather than at a
/// display-friendly page size.
pub const MAX_BLOCKED_VERSIONS_PER_PACKAGE: u64 = 10_000;

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

    /// Every blocked version of one package, in no particular order.
    ///
    /// The bulk counterpart to [`Self::get_status`], for the version *listing*
    /// paths: a packument or version index has to know which of a package's
    /// versions are blocked before it can leave them out, and asking per version
    /// would be one query per version on a hot metadata path.
    ///
    /// The default implementation derives the answer from
    /// [`Self::list_packages`] so every existing implementor keeps working;
    /// backends with a cheaper query (a single indexed `SELECT` rather than the
    /// listing query's audit-count joins) should override it.
    async fn blocked_versions(&self, registry: &str, name: &str) -> Result<Vec<String>, CoreError> {
        // `PackageFilter::default()` leaves `limit` at 0, which the SQL renders
        // as `LIMIT 0` — an explicit page size is required or this silently
        // returns nothing and every version reads as unblocked.
        let filter = PackageFilter {
            registry: Some(registry.to_owned()),
            name_exact: Some(name.to_owned()),
            blocked_only: true,
            limit: MAX_BLOCKED_VERSIONS_PER_PACKAGE,
            ..Default::default()
        };
        Ok(self
            .list_packages(filter)
            .await?
            .into_iter()
            .map(|p| p.package_id.version)
            .collect())
    }

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

    /// Count matching access events without applying `limit`/`offset`. Used for accurate pagination totals.
    async fn count_events(&self, filter: EventFilter) -> Result<u64, CoreError>;

    /// The caller's own successful downloads, newest first.
    ///
    /// Deliberately *not* `list_events` with a filter: this backs
    /// `GET /api/v1/me/downloads`, and the scoping that keeps one user out of
    /// another's history belongs here rather than in a handler, where a
    /// forgotten `user_id` would leak the whole log (RFC 0004 §6.2, §7). The
    /// three constraints — this principal, `AccessAction::Download`, allowed
    /// only — are the method's contract, not the caller's to assemble.
    ///
    /// `since` bounds how far back to look; `limit` caps the rows returned.
    /// Anonymous callers have no history: there is no `user_id` to scope by, so
    /// the handler must not call this for them.
    async fn list_own_downloads(
        &self,
        user_id: &str,
        since: DateTime<Utc>,
        limit: u64,
    ) -> Result<Vec<AccessEvent>, CoreError> {
        let (_, _, _) = (user_id, since, limit);
        Ok(vec![])
    }

    /// Delete access-event rows older than `before`. Returns the number of rows deleted.
    async fn purge_events_before(&self, before: DateTime<Utc>) -> Result<u64, CoreError> {
        let _ = before;
        Ok(0)
    }

    /// Distinct non-null `user_id`s the access log has seen, newest activity
    /// first, optionally narrowed to those containing `contains`.
    ///
    /// RFC 0004-bis A8. Four console fields ask an operator to type a subject
    /// and nothing can offer one: `/api/v1/admin/users/blocked` lists only the
    /// *blocked*. The failure is silent — filtering the audit log for `alice`
    /// on an instance that stores `oidc:alice` returns an empty table, which
    /// reads exactly like "this user did nothing" on the surface whose entire
    /// purpose is establishing what someone did.
    ///
    /// Scoped to identities this instance has actually seen, not a user
    /// directory: this product does not have one and should not grow one here.
    async fn distinct_event_subjects(
        &self,
        contains: Option<&str>,
        limit: u64,
    ) -> Result<Vec<String>, CoreError> {
        let _ = (contains, limit);
        Ok(vec![])
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

    /// Admin health dashboard: distinct package counts per registry, keyed by
    /// registry name. Registries with no packages are simply absent from the map.
    async fn registry_package_counts(
        &self,
        registries: &[String],
    ) -> Result<HashMap<String, i64>, CoreError> {
        let _ = registries;
        Ok(HashMap::new())
    }

    /// Admin health dashboard: per-registry download stats — last successful
    /// pull time, pulls in the last hour, and pulls in the last day. Keyed by
    /// registry name; registries with no matching events are absent from the map.
    async fn registry_event_stats(
        &self,
        registries: &[String],
    ) -> Result<HashMap<String, (Option<DateTime<Utc>>, i64, i64)>, CoreError> {
        let _ = registries;
        Ok(HashMap::new())
    }

    /// Admin health dashboard: most recent denied/error access events for a
    /// single registry within the last 24 hours, newest first, capped at `limit`.
    async fn recent_registry_errors(
        &self,
        registry: &str,
        limit: i64,
    ) -> Result<Vec<RecentErrorRecord>, CoreError> {
        let _ = (registry, limit);
        Ok(Vec::new())
    }
}
