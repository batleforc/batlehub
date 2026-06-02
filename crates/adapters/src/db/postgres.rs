use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row};
use crate::db::DbResultExt;

use crate::migrations::embedded_migrator;
use uuid::Uuid;

use batlehub_core::{
    entities::{
        AccessAction, AccessEvent, AccessResult, EventFilter, ExploreEntry, ExploreFilter,
        ExploreSortBy, PackageFilter, PackageId, PackageSource, PackageStatus, PackageSummary,
        RegistryStat, Role,
    },
    error::CoreError,
    ports::PackageRepository,
};

pub struct PgPackageRepository {
    pub(super) pool: PgPool,
}

impl PgPackageRepository {
    pub async fn new(database_url: &str) -> Result<Self, CoreError> {
        let pool = PgPool::connect(database_url)
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

// ── Helper conversions ────────────────────────────────────────────────────────

fn role_to_str(role: &Role) -> &'static str {
    match role {
        Role::Anonymous => "anonymous",
        Role::User => "user",
        Role::Admin => "admin",
    }
}

fn str_to_role(s: &str) -> Role {
    match s {
        "admin" => Role::Admin,
        "user" => Role::User,
        _ => Role::Anonymous,
    }
}

fn action_to_str(action: &AccessAction) -> &'static str {
    match action {
        AccessAction::Download => "download",
        AccessAction::ViewMetadata => "view_metadata",
        AccessAction::Block => "block",
        AccessAction::Unblock => "unblock",
    }
}

fn str_to_action(s: &str) -> AccessAction {
    match s {
        "download" => AccessAction::Download,
        "view_metadata" => AccessAction::ViewMetadata,
        "block" => AccessAction::Block,
        "unblock" => AccessAction::Unblock,
        _ => AccessAction::Download,
    }
}

// ── PackageRepository impl ────────────────────────────────────────────────────

#[async_trait]
impl PackageRepository for PgPackageRepository {
    async fn record_access(&self, event: AccessEvent) -> Result<(), CoreError> {
        let (outcome, deny_reason): (&str, Option<String>) = match &event.result {
            AccessResult::Allowed => ("allowed", None),
            AccessResult::Denied { reason } => ("denied", Some(reason.clone())),
            AccessResult::ProxyError { reason } => ("error", Some(reason.clone())),
        };

        sqlx::query(
            r#"
            INSERT INTO access_events
                (id, user_id, user_role, registry, package_name, package_version,
                 package_artifact, action, outcome, deny_reason, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
            "#,
        )
        .bind(event.id)
        .bind(&event.user_id)
        .bind(role_to_str(&event.user_role))
        .bind(&event.package_id.registry)
        .bind(&event.package_id.name)
        .bind(&event.package_id.version)
        .bind(&event.package_id.artifact)
        .bind(action_to_str(&event.action))
        .bind(outcome)
        .bind(deny_reason)
        .bind(event.timestamp)
        .execute(&self.pool)
        .await
        .db_err()?;

        // Ensure the package appears in list_packages by creating an 'available' status
        // row on first access. DO NOTHING preserves any existing blocked status.
        if matches!(event.result, AccessResult::Allowed) {
            sqlx::query(
                r#"
                INSERT INTO package_statuses
                    (id, registry, package_name, package_version, package_artifact,
                     status, updated_at)
                VALUES ($1, $2, $3, $4, $5, 'available', NOW())
                ON CONFLICT (registry, package_name, package_version, COALESCE(package_artifact, ''))
                DO NOTHING
                "#,
            )
            .bind(Uuid::new_v4())
            .bind(&event.package_id.registry)
            .bind(&event.package_id.name)
            .bind(&event.package_id.version)
            .bind(&event.package_id.artifact)
            .execute(&self.pool)
            .await
            .db_err()?;
        }

        Ok(())
    }

    async fn get_status(&self, pkg: &PackageId) -> Result<PackageStatus, CoreError> {
        let row = sqlx::query(
            r#"
            SELECT status, block_reason, blocked_by, blocked_at
            FROM package_statuses
            WHERE registry = $1 AND package_name = $2 AND package_version = $3
              AND (package_artifact IS NOT DISTINCT FROM $4)
            "#,
        )
        .bind(&pkg.registry)
        .bind(&pkg.name)
        .bind(&pkg.version)
        .bind(&pkg.artifact)
        .fetch_optional(&self.pool)
        .await
        .db_err()?;

        match row {
            None => Ok(PackageStatus::Available),
            Some(r) => {
                let status: String = r.get("status");
                if status == "blocked" {
                    Ok(PackageStatus::Blocked {
                        reason: r
                            .get::<Option<String>, _>("block_reason")
                            .unwrap_or_default(),
                        blocked_by: r.get::<Option<String>, _>("blocked_by").unwrap_or_default(),
                        blocked_at: r
                            .get::<Option<DateTime<Utc>>, _>("blocked_at")
                            .unwrap_or_else(Utc::now),
                    })
                } else {
                    Ok(PackageStatus::Available)
                }
            }
        }
    }

    async fn set_status(&self, pkg: &PackageId, status: PackageStatus) -> Result<(), CoreError> {
        let (status_str, reason, blocked_by, blocked_at): (
            &str,
            Option<String>,
            Option<String>,
            Option<DateTime<Utc>>,
        ) = match &status {
            PackageStatus::Available => ("available", None, None, None),
            PackageStatus::Blocked {
                reason,
                blocked_by,
                blocked_at,
            } => (
                "blocked",
                Some(reason.clone()),
                Some(blocked_by.clone()),
                Some(*blocked_at),
            ),
        };

        sqlx::query(
            r#"
            INSERT INTO package_statuses
                (id, registry, package_name, package_version, package_artifact,
                 status, block_reason, blocked_by, blocked_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, NOW())
            ON CONFLICT (registry, package_name, package_version, COALESCE(package_artifact, ''))
            DO UPDATE SET
                status = EXCLUDED.status,
                block_reason = EXCLUDED.block_reason,
                blocked_by = EXCLUDED.blocked_by,
                blocked_at = EXCLUDED.blocked_at,
                updated_at = NOW()
            "#,
        )
        .bind(Uuid::new_v4())
        .bind(&pkg.registry)
        .bind(&pkg.name)
        .bind(&pkg.version)
        .bind(&pkg.artifact)
        .bind(status_str)
        .bind(reason)
        .bind(blocked_by)
        .bind(blocked_at)
        .execute(&self.pool)
        .await
        .db_err()?;

        Ok(())
    }

    async fn list_packages(&self, filter: PackageFilter) -> Result<Vec<PackageSummary>, CoreError> {
        let rows = sqlx::query(
            r#"
            SELECT
                ps.id,
                ps.registry,
                ps.package_name,
                ps.package_version,
                ps.package_artifact,
                ps.status,
                ps.block_reason,
                ps.blocked_by,
                ps.blocked_at,
                COALESCE(ae_counts.access_count, 0) AS access_count,
                ae_counts.last_accessed,
                ae_user.last_accessed_by
            FROM package_statuses ps
            LEFT JOIN LATERAL (
                SELECT COUNT(*) AS access_count, MAX(ae.created_at) AS last_accessed
                FROM access_events ae
                WHERE ae.registry       = ps.registry
                  AND ae.package_name   = ps.package_name
                  AND ae.package_version = ps.package_version
            ) ae_counts ON true
            LEFT JOIN LATERAL (
                SELECT ae2.user_id AS last_accessed_by
                FROM access_events ae2
                WHERE ae2.registry       = ps.registry
                  AND ae2.package_name   = ps.package_name
                  AND ae2.package_version = ps.package_version
                  AND ae2.outcome = 'allowed'
                ORDER BY ae2.created_at DESC
                LIMIT 1
            ) ae_user ON true
            WHERE ($1::text IS NULL OR ps.registry = $1)
              AND ($2::text IS NULL OR ps.package_name ILIKE '%' || $2 || '%')
              AND ($3::boolean = false OR ps.status = 'blocked')
              AND ($6::text IS NULL OR ps.package_name = $6)
              AND ($7::text[] IS NULL OR ps.registry = ANY($7::text[]))
            ORDER BY ps.registry, ps.package_name, ps.package_version
            LIMIT $4 OFFSET $5
            "#,
        )
        .bind(&filter.registry)
        .bind(&filter.name_contains)
        .bind(filter.blocked_only)
        .bind(filter.limit as i64)
        .bind(filter.offset as i64)
        .bind(&filter.name_exact)
        .bind(if filter.registries.is_empty() {
            None
        } else {
            Some(filter.registries.clone())
        })
        .fetch_all(&self.pool)
        .await
        .db_err()?;

        let summaries = rows
            .into_iter()
            .map(|r| {
                let status: String = r.get("status");
                let pkg_status = if status == "blocked" {
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
                };

                PackageSummary {
                    id: r.get("id"),
                    package_id: PackageId {
                        registry: r.get("registry"),
                        name: r.get("package_name"),
                        version: r.get("package_version"),
                        artifact: r.get("package_artifact"),
                    },
                    status: pkg_status,
                    last_accessed: r.get("last_accessed"),
                    last_accessed_by: r.get("last_accessed_by"),
                    access_count: r.get::<i64, _>("access_count") as u64,
                }
            })
            .collect();

        Ok(summaries)
    }

    async fn count_packages(&self, filter: PackageFilter) -> Result<u64, CoreError> {
        let row = sqlx::query(
            r#"
            SELECT COUNT(*) AS total
            FROM package_statuses ps
            WHERE ($1::text IS NULL OR ps.registry = $1)
              AND ($2::text IS NULL OR ps.package_name ILIKE '%' || $2 || '%')
              AND ($3::boolean = false OR ps.status = 'blocked')
              AND ($4::text IS NULL OR ps.package_name = $4)
              AND ($5::text[] IS NULL OR ps.registry = ANY($5::text[]))
            "#,
        )
        .bind(&filter.registry)
        .bind(&filter.name_contains)
        .bind(filter.blocked_only)
        .bind(&filter.name_exact)
        .bind(if filter.registries.is_empty() {
            None
        } else {
            Some(filter.registries)
        })
        .fetch_one(&self.pool)
        .await
        .db_err()?;

        let count: i64 = row.try_get("total").unwrap_or(0);
        Ok(count as u64)
    }

    async fn list_events(&self, filter: EventFilter) -> Result<Vec<AccessEvent>, CoreError> {
        let rows = sqlx::query(
            r#"
            SELECT
                id, user_id, user_role, registry, package_name, package_version,
                package_artifact, action, outcome, deny_reason, created_at
            FROM access_events
            WHERE ($1::text IS NULL OR registry = $1)
              AND ($2::text IS NULL OR user_id = $2)
              AND ($3::timestamptz IS NULL OR created_at >= $3)
              AND ($4::timestamptz IS NULL OR created_at <= $4)
              AND ($5::boolean = false OR outcome = 'denied')
              AND ($8::text IS NULL OR package_name = $8)
            ORDER BY created_at DESC
            LIMIT $6 OFFSET $7
            "#,
        )
        .bind(&filter.registry)
        .bind(&filter.user_id)
        .bind(filter.from)
        .bind(filter.to)
        .bind(filter.denied_only)
        .bind(filter.limit as i64)
        .bind(filter.offset as i64)
        .bind(&filter.package_name)
        .fetch_all(&self.pool)
        .await
        .db_err()?;

        let events = rows
            .into_iter()
            .map(|r| {
                let outcome: String = r.get("outcome");
                AccessEvent {
                    id: r.get("id"),
                    user_id: r.get("user_id"),
                    user_role: str_to_role(r.get::<&str, _>("user_role")),
                    package_id: PackageId {
                        registry: r.get("registry"),
                        name: r.get("package_name"),
                        version: r.get("package_version"),
                        artifact: r.get("package_artifact"),
                    },
                    action: str_to_action(r.get::<&str, _>("action")),
                    result: match outcome.as_str() {
                        "denied" => AccessResult::Denied {
                            reason: r
                                .get::<Option<String>, _>("deny_reason")
                                .unwrap_or_default(),
                        },
                        "error" => AccessResult::ProxyError {
                            reason: r
                                .get::<Option<String>, _>("deny_reason")
                                .unwrap_or_default(),
                        },
                        _ => AccessResult::Allowed,
                    },
                    timestamp: r.get("created_at"),
                }
            })
            .collect();

        Ok(events)
    }

    async fn explore_packages(
        &self,
        filter: ExploreFilter,
    ) -> Result<Vec<ExploreEntry>, CoreError> {
        let order = match filter.sort_by {
            ExploreSortBy::Name => "package_name ASC",
            ExploreSortBy::Downloads => "total_downloads DESC NULLS LAST",
            ExploreSortBy::Recent => "last_accessed DESC NULLS LAST",
        };
        let registries = if filter.registries.is_empty() {
            None
        } else {
            Some(filter.registries.clone())
        };

        let sql = format!(
            r#"
            WITH proxied AS (
                SELECT
                    ps.registry,
                    ps.package_name,
                    COUNT(DISTINCT ps.package_version)::bigint AS version_count,
                    BOOL_OR(ps.status = 'blocked') AS has_blocked,
                    true AS has_proxied,
                    false AS has_local
                FROM package_statuses ps
                WHERE ($1::text IS NULL OR ps.registry = $1)
                  AND ($2::text IS NULL OR ps.package_name ILIKE '%' || $2 || '%')
                  AND ($3::text[] IS NULL OR ps.registry = ANY($3::text[]))
                GROUP BY ps.registry, ps.package_name
            ),
            local_pkgs AS (
                SELECT
                    lp.registry,
                    lp.name AS package_name,
                    COUNT(DISTINCT lp.version)::bigint AS version_count,
                    BOOL_OR(lp.yanked) AS has_blocked,
                    false AS has_proxied,
                    true AS has_local
                FROM local_packages lp
                WHERE lp.status = 'published'
                  AND ($1::text IS NULL OR lp.registry = $1)
                  AND ($2::text IS NULL OR lp.name ILIKE '%' || $2 || '%')
                  AND ($3::text[] IS NULL OR lp.registry = ANY($3::text[]))
                GROUP BY lp.registry, lp.name
            ),
            combined AS (
                SELECT * FROM proxied
                UNION ALL
                SELECT * FROM local_pkgs
            ),
            agg AS (
                SELECT
                    registry,
                    package_name,
                    SUM(version_count)::bigint AS version_count,
                    BOOL_OR(has_blocked) AS has_blocked,
                    BOOL_OR(has_proxied) AS has_proxied,
                    BOOL_OR(has_local) AS has_local
                FROM combined
                GROUP BY registry, package_name
            )
            SELECT
                agg.registry,
                agg.package_name,
                agg.version_count,
                agg.has_blocked,
                agg.has_proxied,
                agg.has_local,
                COALESCE(ae.total_downloads, 0)::bigint AS total_downloads,
                ae.last_accessed
            FROM agg
            LEFT JOIN LATERAL (
                SELECT COUNT(*)::bigint AS total_downloads, MAX(created_at) AS last_accessed
                FROM access_events
                WHERE registry = agg.registry AND package_name = agg.package_name
            ) ae ON true
            ORDER BY {order}
            LIMIT $4 OFFSET $5
            "#
        );

        let rows = sqlx::query(&sql)
            .bind(&filter.registry)
            .bind(&filter.name_contains)
            .bind(registries)
            .bind(filter.limit as i64)
            .bind(filter.offset as i64)
            .fetch_all(&self.pool)
            .await
            .db_err()?;

        let entries = rows
            .into_iter()
            .map(|r| {
                let has_proxied: bool = r.get("has_proxied");
                let has_local: bool = r.get("has_local");
                let source = match (has_proxied, has_local) {
                    (true, true) => PackageSource::Both,
                    (false, true) => PackageSource::Local,
                    _ => PackageSource::Proxied,
                };
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
            })
            .collect();

        Ok(entries)
    }

    async fn count_explore_packages(&self, filter: ExploreFilter) -> Result<u64, CoreError> {
        let registries = if filter.registries.is_empty() {
            None
        } else {
            Some(filter.registries.clone())
        };

        let row = sqlx::query(
            r#"
            WITH proxied AS (
                SELECT registry, package_name
                FROM package_statuses
                WHERE ($1::text IS NULL OR registry = $1)
                  AND ($2::text IS NULL OR package_name ILIKE '%' || $2 || '%')
                  AND ($3::text[] IS NULL OR registry = ANY($3::text[]))
            ),
            local_pkgs AS (
                SELECT registry, name AS package_name
                FROM local_packages
                WHERE status = 'published'
                  AND ($1::text IS NULL OR registry = $1)
                  AND ($2::text IS NULL OR name ILIKE '%' || $2 || '%')
                  AND ($3::text[] IS NULL OR registry = ANY($3::text[]))
            )
            SELECT COUNT(*) AS total FROM (
                SELECT registry, package_name FROM proxied
                UNION
                SELECT registry, package_name FROM local_pkgs
            ) combined
            "#,
        )
        .bind(&filter.registry)
        .bind(&filter.name_contains)
        .bind(registries)
        .fetch_one(&self.pool)
        .await
        .db_err()?;

        let count: i64 = row.try_get("total").unwrap_or(0);
        Ok(count as u64)
    }

    async fn registry_explore_stats(
        &self,
        accessible_registries: &[String],
    ) -> Result<Vec<RegistryStat>, CoreError> {
        let registries = if accessible_registries.is_empty() {
            None
        } else {
            Some(accessible_registries.to_vec())
        };

        let rows = sqlx::query(
            r#"
            WITH pkg_counts AS (
                SELECT registry, COUNT(DISTINCT package_name)::bigint AS package_count
                FROM (
                    SELECT registry, package_name FROM package_statuses
                    WHERE ($1::text[] IS NULL OR registry = ANY($1::text[]))
                    UNION
                    SELECT registry, name AS package_name FROM local_packages
                    WHERE status = 'published'
                      AND ($1::text[] IS NULL OR registry = ANY($1::text[]))
                ) combined
                GROUP BY registry
            ),
            download_counts AS (
                SELECT registry, COUNT(*)::bigint AS total_downloads
                FROM access_events
                WHERE ($1::text[] IS NULL OR registry = ANY($1::text[]))
                GROUP BY registry
            )
            SELECT
                pc.registry,
                pc.package_count,
                COALESCE(dc.total_downloads, 0) AS total_downloads
            FROM pkg_counts pc
            LEFT JOIN download_counts dc ON dc.registry = pc.registry
            ORDER BY pc.package_count DESC
            "#,
        )
        .bind(registries)
        .fetch_all(&self.pool)
        .await
        .db_err()?;

        let stats = rows
            .into_iter()
            .map(|r| {
                let pkg_count: i64 = r.get("package_count");
                let downloads: i64 = r.get("total_downloads");
                RegistryStat {
                    registry: r.get("registry"),
                    package_count: pkg_count as u64,
                    total_downloads: downloads as u64,
                }
            })
            .collect();

        Ok(stats)
    }
}
