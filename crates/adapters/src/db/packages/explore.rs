use chrono::{DateTime, Utc};

use super::{
    map_explore_entry, prepare_registries_param, sort_order_for, str_to_action, str_to_role,
    AccessEvent, AccessResult, CoreError, DbResultExt, EventFilter, ExploreEntry, ExploreFilter,
    PackageId, PgPool, RegistryStat, Row,
};

pub(super) async fn list_events_impl(
    pool: &PgPool,
    filter: EventFilter,
) -> Result<Vec<AccessEvent>, CoreError> {
    let rows = sqlx::query(
        r#"
        SELECT
            id, user_id, user_role, registry, package_name, package_version,
            package_artifact, action, outcome, deny_reason, created_at,
            ip_address, user_agent
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
    .fetch_all(pool)
    .await
    .db_err()?;

    rows.into_iter()
        .map(|r| {
            let outcome: String = r.get("outcome");
            // Account-wide/network-wide events store NULL in all three coordinate
            // columns (see migration 030); only build a `PackageId` when the row
            // actually has one.
            let registry: Option<String> = r.get("registry");
            let package_name: Option<String> = r.get("package_name");
            let package_version: Option<String> = r.get("package_version");
            let package_artifact: Option<String> = r.get("package_artifact");
            let package_id = match (registry, package_name, package_version) {
                (Some(registry), Some(name), Some(version)) => Some(PackageId {
                    registry,
                    name,
                    version,
                    artifact: package_artifact,
                }),
                _ => None,
            };
            Ok(AccessEvent {
                id: r.get("id"),
                user_id: r.get("user_id"),
                user_role: str_to_role(r.get::<&str, _>("user_role"))?,
                package_id,
                action: str_to_action(r.get::<&str, _>("action"))?,
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
                    "allowed" => AccessResult::Allowed,
                    other => {
                        return Err(CoreError::Database(format!(
                            "invalid access outcome in db: '{other}'"
                        )))
                    }
                },
                timestamp: r.get("created_at"),
                ip_address: r.get("ip_address"),
                user_agent: r.get("user_agent"),
            })
        })
        .collect()
}

pub(super) async fn explore_packages_impl(
    pool: &PgPool,
    filter: ExploreFilter,
) -> Result<Vec<ExploreEntry>, CoreError> {
    let order = sort_order_for(&filter.sort_by);
    let registries = prepare_registries_param(&filter.registries);

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

    let rows = sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(&filter.registry)
        .bind(&filter.name_contains)
        .bind(registries)
        .bind(filter.limit as i64)
        .bind(filter.offset as i64)
        .fetch_all(pool)
        .await
        .db_err()?;

    Ok(rows.into_iter().map(map_explore_entry).collect())
}

pub(super) async fn count_explore_packages_impl(
    pool: &PgPool,
    filter: ExploreFilter,
) -> Result<u64, CoreError> {
    let registries = prepare_registries_param(&filter.registries);

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
    .fetch_one(pool)
    .await
    .db_err()?;

    let count: i64 = row.try_get("total").unwrap_or(0);
    Ok(count as u64)
}

pub(super) async fn registry_explore_stats_impl(
    pool: &PgPool,
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
    .fetch_all(pool)
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

pub(super) async fn purge_events_before_impl(
    pool: &PgPool,
    before: DateTime<Utc>,
) -> Result<u64, CoreError> {
    let result = sqlx::query("DELETE FROM access_events WHERE created_at < $1")
        .bind(before)
        .execute(pool)
        .await
        .db_err()?;
    Ok(result.rows_affected())
}
