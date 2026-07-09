use std::collections::HashMap;

use crate::db::DbResultExt;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::postgres::PgRow;
use sqlx::{PgPool, Row};

use crate::migrations::embedded_migrator;
use uuid::Uuid;

use batlehub_core::{
    entities::{
        AccessAction, AccessEvent, AccessResult, EventFilter, ExploreEntry, ExploreFilter,
        ExploreSortBy, PackageFilter, PackageId, PackageSource, PackageStatus, PackageSummary,
        RegistryStat, Role,
    },
    error::CoreError,
    ports::{PackageRepository, RecentErrorRecord},
};

pub mod crud;
pub mod explore;
pub mod health;

pub(super) fn prepare_registries_param(registries: &[String]) -> Option<Vec<String>> {
    if registries.is_empty() {
        None
    } else {
        Some(registries.to_vec())
    }
}

pub(super) fn map_package_status(r: &PgRow) -> PackageStatus {
    let status: String = r.get("status");
    if status == "blocked" {
        PackageStatus::Blocked {
            reason: r
                .get::<Option<String>, _>("block_reason")
                .unwrap_or_default(),
            blocked_by: r.get::<Option<String>, _>("blocked_by").unwrap_or_default(),
            blocked_at: r
                .get::<Option<DateTime<Utc>>, _>("blocked_at")
                .unwrap_or_else(Utc::now),
        }
    } else {
        PackageStatus::Available
    }
}

pub(super) fn map_package_summary(r: PgRow) -> PackageSummary {
    PackageSummary {
        id: r.get("id"),
        package_id: PackageId {
            registry: r.get("registry"),
            name: r.get("package_name"),
            version: r.get("package_version"),
            artifact: r.get("package_artifact"),
        },
        status: map_package_status(&r),
        last_accessed: r.get("last_accessed"),
        last_accessed_by: r.get("last_accessed_by"),
        access_count: r.get::<i64, _>("access_count") as u64,
    }
}

pub(super) fn sort_order_for(sort_by: &ExploreSortBy) -> &'static str {
    match sort_by {
        ExploreSortBy::Name => "package_name ASC",
        ExploreSortBy::Downloads => "total_downloads DESC NULLS LAST",
        ExploreSortBy::Recent => "last_accessed DESC NULLS LAST",
    }
}

pub(super) fn determine_package_source(has_proxied: bool, has_local: bool) -> PackageSource {
    match (has_proxied, has_local) {
        (true, true) => PackageSource::Both,
        (false, true) => PackageSource::Local,
        _ => PackageSource::Proxied,
    }
}

pub(super) fn map_explore_entry(r: PgRow) -> ExploreEntry {
    let has_proxied: bool = r.get("has_proxied");
    let has_local: bool = r.get("has_local");
    let source = determine_package_source(has_proxied, has_local);
    let downloads: i64 = r.get("total_downloads");
    ExploreEntry {
        registry: r.get("registry"),
        name: r.get("package_name"),
        version_count: r.get::<i64, _>("version_count") as u64,
        total_downloads: downloads as u64,
        last_accessed: r.get("last_accessed"),
        source,
        has_blocked: r.get("has_blocked"),
    }
}

// ── Helper conversions ────────────────────────────────────────────────────────

pub(super) fn role_to_str(role: &Role) -> &'static str {
    match role {
        Role::Anonymous => "anonymous",
        Role::User => "user",
        Role::Admin => "admin",
    }
}

pub(super) fn str_to_role(s: &str) -> Result<Role, CoreError> {
    s.parse()
        .map_err(|e| CoreError::Database(format!("invalid role in db: {e}")))
}

pub(super) fn action_to_str(action: &AccessAction) -> &'static str {
    match action {
        AccessAction::Download => "download",
        AccessAction::ViewMetadata => "view_metadata",
        AccessAction::Block => "block",
        AccessAction::Unblock => "unblock",
        AccessAction::Delete => "delete",
        AccessAction::AddOwner => "add_owner",
        AccessAction::RemoveOwner => "remove_owner",
        AccessAction::SetVisibility => "set_visibility",
        AccessAction::BlockUser => "block_user",
        AccessAction::UnblockUser => "unblock_user",
        AccessAction::BlockIp => "block_ip",
        AccessAction::UnblockIp => "unblock_ip",
        AccessAction::AuditPurge => "audit_purge",
        AccessAction::Yank => "yank",
        AccessAction::Unyank => "unyank",
        AccessAction::Deprecate => "deprecate",
        AccessAction::Undeprecate => "undeprecate",
        AccessAction::Unlist => "unlist",
        AccessAction::Relist => "relist",
        AccessAction::AddBetaMember => "add_beta_member",
        AccessAction::RemoveBetaMember => "remove_beta_member",
        AccessAction::ClaimNamespace => "claim_namespace",
        AccessAction::ReleaseNamespace => "release_namespace",
        AccessAction::ResetQuota => "reset_quota",
    }
}

pub(super) fn str_to_action(s: &str) -> Result<AccessAction, CoreError> {
    match s {
        "download" => Ok(AccessAction::Download),
        "view_metadata" => Ok(AccessAction::ViewMetadata),
        "block" => Ok(AccessAction::Block),
        "unblock" => Ok(AccessAction::Unblock),
        "delete" => Ok(AccessAction::Delete),
        "add_owner" => Ok(AccessAction::AddOwner),
        "remove_owner" => Ok(AccessAction::RemoveOwner),
        "set_visibility" => Ok(AccessAction::SetVisibility),
        "block_user" => Ok(AccessAction::BlockUser),
        "unblock_user" => Ok(AccessAction::UnblockUser),
        "block_ip" => Ok(AccessAction::BlockIp),
        "unblock_ip" => Ok(AccessAction::UnblockIp),
        "audit_purge" => Ok(AccessAction::AuditPurge),
        "yank" => Ok(AccessAction::Yank),
        "unyank" => Ok(AccessAction::Unyank),
        "deprecate" => Ok(AccessAction::Deprecate),
        "undeprecate" => Ok(AccessAction::Undeprecate),
        "unlist" => Ok(AccessAction::Unlist),
        "relist" => Ok(AccessAction::Relist),
        "add_beta_member" => Ok(AccessAction::AddBetaMember),
        "remove_beta_member" => Ok(AccessAction::RemoveBetaMember),
        "claim_namespace" => Ok(AccessAction::ClaimNamespace),
        "release_namespace" => Ok(AccessAction::ReleaseNamespace),
        "reset_quota" => Ok(AccessAction::ResetQuota),
        other => Err(CoreError::Database(format!(
            "invalid access action in db: '{other}'"
        ))),
    }
}

pub struct PgPackageRepository {
    pub(super) pool: PgPool,
}

/// Connection pool sizing, taken from `DatabaseConfig` (`crates/config/src/schema/server.rs`).
pub struct PoolOptions {
    pub max_connections: u32,
    pub min_connections: u32,
    pub acquire_timeout_secs: u64,
}

impl PgPackageRepository {
    pub async fn new(database_url: &str, pool_options: PoolOptions) -> Result<Self, CoreError> {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(pool_options.max_connections)
            .min_connections(pool_options.min_connections)
            .acquire_timeout(std::time::Duration::from_secs(
                pool_options.acquire_timeout_secs,
            ))
            .connect(database_url)
            .await
            .db_err()?;
        Ok(Self { pool })
    }

    pub async fn run_migrations(&self) -> Result<(), CoreError> {
        embedded_migrator()
            .run(&self.pool)
            .await
            .map_err(|e| CoreError::Database(format!("migration failed: {e}")))?;
        Ok(())
    }

    pub fn pool(&self) -> PgPool {
        self.pool.clone()
    }
}

#[async_trait]
impl PackageRepository for PgPackageRepository {
    async fn record_access(&self, event: AccessEvent) -> Result<(), CoreError> {
        crud::record_access_impl(&self.pool, event).await
    }

    async fn get_status(&self, pkg: &PackageId) -> Result<PackageStatus, CoreError> {
        crud::get_status_impl(&self.pool, pkg).await
    }

    async fn set_status(&self, pkg: &PackageId, status: PackageStatus) -> Result<(), CoreError> {
        crud::set_status_impl(&self.pool, pkg, status).await
    }

    async fn delete_package(&self, pkg: &PackageId) -> Result<bool, CoreError> {
        crud::delete_package_impl(&self.pool, pkg).await
    }

    async fn list_packages(&self, filter: PackageFilter) -> Result<Vec<PackageSummary>, CoreError> {
        crud::list_packages_impl(&self.pool, filter).await
    }

    async fn count_packages(&self, filter: PackageFilter) -> Result<u64, CoreError> {
        crud::count_packages_impl(&self.pool, filter).await
    }

    async fn list_events(&self, filter: EventFilter) -> Result<Vec<AccessEvent>, CoreError> {
        explore::list_events_impl(&self.pool, filter).await
    }

    async fn count_events(&self, filter: EventFilter) -> Result<u64, CoreError> {
        explore::count_events_impl(&self.pool, filter).await
    }

    async fn purge_events_before(&self, before: DateTime<Utc>) -> Result<u64, CoreError> {
        explore::purge_events_before_impl(&self.pool, before).await
    }

    async fn explore_packages(
        &self,
        filter: ExploreFilter,
    ) -> Result<Vec<ExploreEntry>, CoreError> {
        explore::explore_packages_impl(&self.pool, filter).await
    }

    async fn count_explore_packages(&self, filter: ExploreFilter) -> Result<u64, CoreError> {
        explore::count_explore_packages_impl(&self.pool, filter).await
    }

    async fn registry_explore_stats(
        &self,
        accessible_registries: &[String],
    ) -> Result<Vec<RegistryStat>, CoreError> {
        explore::registry_explore_stats_impl(&self.pool, accessible_registries).await
    }

    async fn registry_package_counts(
        &self,
        registries: &[String],
    ) -> Result<HashMap<String, i64>, CoreError> {
        health::registry_package_counts_impl(&self.pool, registries).await
    }

    async fn registry_event_stats(
        &self,
        registries: &[String],
    ) -> Result<HashMap<String, (Option<DateTime<Utc>>, i64, i64)>, CoreError> {
        health::registry_event_stats_impl(&self.pool, registries).await
    }

    async fn recent_registry_errors(
        &self,
        registry: &str,
        limit: i64,
    ) -> Result<Vec<RecentErrorRecord>, CoreError> {
        health::recent_registry_errors_impl(&self.pool, registry, limit).await
    }
}
