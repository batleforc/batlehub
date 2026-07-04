use std::collections::HashMap;

use chrono::{DateTime, Utc};

use super::{CoreError, DbResultExt, PgPool, RecentErrorRecord, Row};

/// Distinct package counts per registry, for the admin health dashboard.
/// Mirrors the raw query formerly inlined in
/// `crates/web/src/handlers/back_office/health/system.rs`.
pub(super) async fn registry_package_counts_impl(
    pool: &PgPool,
    registries: &[String],
) -> Result<HashMap<String, i64>, CoreError> {
    let rows = sqlx::query(
        "SELECT registry, COUNT(DISTINCT package_name) AS cnt
         FROM package_statuses
         WHERE registry = ANY($1)
         GROUP BY registry",
    )
    .bind(registries)
    .fetch_all(pool)
    .await
    .db_err()?;

    Ok(rows
        .into_iter()
        .map(|r| (r.get::<String, _>("registry"), r.get::<i64, _>("cnt")))
        .collect())
}

/// Per-registry download stats (last pull time, pulls in the last hour/day)
/// for the admin health dashboard.
pub(super) async fn registry_event_stats_impl(
    pool: &PgPool,
    registries: &[String],
) -> Result<HashMap<String, (Option<DateTime<Utc>>, i64, i64)>, CoreError> {
    let rows = sqlx::query(
        r#"SELECT
               registry,
               MAX(created_at) FILTER (WHERE action = 'download' AND outcome = 'allowed')
                   AS last_pull_at,
               COUNT(*) FILTER (
                   WHERE action = 'download' AND outcome = 'allowed'
                   AND created_at > NOW() - INTERVAL '1 hour'
               ) AS pulls_last_hour,
               COUNT(*) FILTER (
                   WHERE action = 'download' AND outcome = 'allowed'
                   AND created_at > NOW() - INTERVAL '1 day'
               ) AS pulls_last_day
           FROM access_events
           WHERE registry = ANY($1)
           GROUP BY registry"#,
    )
    .bind(registries)
    .fetch_all(pool)
    .await
    .db_err()?;

    Ok(rows
        .into_iter()
        .map(|r| {
            (
                r.get::<String, _>("registry"),
                (
                    r.try_get("last_pull_at").unwrap_or(None),
                    r.try_get("pulls_last_hour").unwrap_or(0),
                    r.try_get("pulls_last_day").unwrap_or(0),
                ),
            )
        })
        .collect())
}

/// Most recent denied/error access events for a single registry (last 24h),
/// newest first, capped at `limit`.
pub(super) async fn recent_registry_errors_impl(
    pool: &PgPool,
    registry: &str,
    limit: i64,
) -> Result<Vec<RecentErrorRecord>, CoreError> {
    let rows = sqlx::query(
        r#"SELECT created_at, user_id, package_name, package_version, outcome, deny_reason
           FROM access_events
           WHERE registry = $1 AND outcome IN ('denied', 'error')
           AND created_at > NOW() - INTERVAL '24 hours'
           ORDER BY created_at DESC LIMIT $2"#,
    )
    .bind(registry)
    .bind(limit)
    .fetch_all(pool)
    .await
    .db_err()?;

    Ok(rows
        .into_iter()
        .map(|r| RecentErrorRecord {
            created_at: r.get("created_at"),
            user_id: r.get("user_id"),
            package_name: r.get("package_name"),
            package_version: r.get("package_version"),
            outcome: r.get::<String, _>("outcome"),
            deny_reason: r.get::<Option<String>, _>("deny_reason"),
        })
        .collect())
}
