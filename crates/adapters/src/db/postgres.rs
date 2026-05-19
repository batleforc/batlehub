use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use proxy_cache_core::{
    entities::{
        AccessAction, AccessEvent, AccessResult, EventFilter, PackageFilter, PackageId,
        PackageStatus, PackageSummary, Role,
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
            .map_err(|e| CoreError::Database(e.to_string()))?;
        Ok(Self { pool })
    }

    pub async fn run_migrations(&self) -> Result<(), CoreError> {
        sqlx::migrate!("./migrations")
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
        .map_err(|e| CoreError::Database(e.to_string()))?;

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
            .map_err(|e| CoreError::Database(e.to_string()))?;
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
        .map_err(|e| CoreError::Database(e.to_string()))?;

        match row {
            None => Ok(PackageStatus::Available),
            Some(r) => {
                let status: String = r.get("status");
                if status == "blocked" {
                    Ok(PackageStatus::Blocked {
                        reason: r.get::<Option<String>, _>("block_reason").unwrap_or_default(),
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
            PackageStatus::Blocked { reason, blocked_by, blocked_at } => {
                ("blocked", Some(reason.clone()), Some(blocked_by.clone()), Some(*blocked_at))
            }
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
        .map_err(|e| CoreError::Database(e.to_string()))?;

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
                COUNT(ae.id) AS access_count,
                MAX(ae.created_at) AS last_accessed,
                (
                    SELECT ae2.user_id
                    FROM access_events ae2
                    WHERE ae2.registry = ps.registry
                      AND ae2.package_name = ps.package_name
                      AND ae2.package_version = ps.package_version
                      AND ae2.outcome = 'allowed'
                    ORDER BY ae2.created_at DESC
                    LIMIT 1
                ) AS last_accessed_by
            FROM package_statuses ps
            LEFT JOIN access_events ae
                ON ae.registry = ps.registry
                AND ae.package_name = ps.package_name
                AND ae.package_version = ps.package_version
            WHERE ($1::text IS NULL OR ps.registry = $1)
              AND ($2::text IS NULL OR ps.package_name ILIKE '%' || $2 || '%')
              AND ($3::boolean = false OR ps.status = 'blocked')
              AND ($6::text IS NULL OR ps.package_name = $6)
            GROUP BY ps.id, ps.registry, ps.package_name, ps.package_version,
                     ps.package_artifact, ps.status, ps.block_reason, ps.blocked_by, ps.blocked_at
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
        .fetch_all(&self.pool)
        .await
        .map_err(|e| CoreError::Database(e.to_string()))?;

        let summaries = rows
            .into_iter()
            .map(|r| {
                let status: String = r.get("status");
                let pkg_status = if status == "blocked" {
                    PackageStatus::Blocked {
                        reason: r.get::<Option<String>, _>("block_reason").unwrap_or_default(),
                        blocked_by: r.get::<Option<String>, _>("blocked_by").unwrap_or_default(),
                        blocked_at: r
                            .get::<Option<DateTime<Utc>>, _>("blocked_at")
                            .unwrap_or_else(Utc::now),
                    }
                } else {
                    PackageStatus::Available
                };

                let count: Option<i64> = r.get("access_count");
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
                    access_count: count.unwrap_or(0) as u64,
                }
            })
            .collect();

        Ok(summaries)
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
        .map_err(|e| CoreError::Database(e.to_string()))?;

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
}
