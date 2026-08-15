use chrono::{DateTime, Utc};

use super::{
    map_explore_entry, prepare_registries_param, sort_order_for, str_to_action, str_to_role,
    AccessEvent, AccessResult, CoreError, DbResultExt, EventFilter, ExploreEntry, ExploreFilter,
    PackageId, PgPool, RegistryStat, Row, LOCAL_VISIBILITY_PREDICATE,
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

    rows.iter().map(map_access_event).collect()
}

/// Build an [`AccessEvent`] from one `access_events` row.
///
/// Every query feeding this must select the same thirteen columns; sqlx 0.9
/// rejects non-literal SQL, so the list is repeated per query rather than
/// shared through a `const`.
fn map_access_event(r: &sqlx::postgres::PgRow) -> Result<AccessEvent, CoreError> {
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
}

/// One principal's own allowed downloads, newest first.
///
/// The `user_id`, `action` and `outcome` predicates are written here rather
/// than assembled by a caller: this is what `GET /api/v1/me/downloads` reads,
/// and a `where` clause a handler can forget is a handler that returns someone
/// else's history (RFC 0004 §7).
pub(super) async fn list_own_downloads_impl(
    pool: &PgPool,
    user_id: &str,
    since: DateTime<Utc>,
    limit: u64,
) -> Result<Vec<AccessEvent>, CoreError> {
    let rows = sqlx::query(
        r#"
        SELECT
            id, user_id, user_role, registry, package_name, package_version,
            package_artifact, action, outcome, deny_reason, created_at,
            ip_address, user_agent
        FROM access_events
        WHERE user_id = $1
          AND action = 'download'
          AND outcome = 'allowed'
          AND created_at >= $2
          AND package_name IS NOT NULL
        ORDER BY created_at DESC
        LIMIT $3
        "#,
    )
    .bind(user_id)
    .bind(since)
    .bind(limit as i64)
    .fetch_all(pool)
    .await
    .db_err()?;

    rows.iter().map(map_access_event).collect()
}

pub(super) async fn count_events_impl(
    pool: &PgPool,
    filter: EventFilter,
) -> Result<u64, CoreError> {
    let row = sqlx::query(
        r#"
        SELECT COUNT(*) AS total
        FROM access_events
        WHERE ($1::text IS NULL OR registry = $1)
          AND ($2::text IS NULL OR user_id = $2)
          AND ($3::timestamptz IS NULL OR created_at >= $3)
          AND ($4::timestamptz IS NULL OR created_at <= $4)
          AND ($5::boolean = false OR outcome = 'denied')
          AND ($6::text IS NULL OR package_name = $6)
        "#,
    )
    .bind(&filter.registry)
    .bind(&filter.user_id)
    .bind(filter.from)
    .bind(filter.to)
    .bind(filter.denied_only)
    .bind(&filter.package_name)
    .fetch_one(pool)
    .await
    .db_err()?;

    let count: i64 = row.try_get("total").unwrap_or(0);
    Ok(count as u64)
}

pub(super) async fn explore_packages_impl(
    pool: &PgPool,
    filter: ExploreFilter,
) -> Result<Vec<ExploreEntry>, CoreError> {
    let order = sort_order_for(&filter.sort_by);
    let registries = prepare_registries_param(&filter.registries);
    let groups = filter.viewer.normalised_groups();
    let visibility = LOCAL_VISIBILITY_PREDICATE;

    let sql = format!(
        r#"
        WITH proxied AS (
            SELECT
                ps.registry,
                ps.package_name,
                COUNT(DISTINCT ps.package_version)::bigint AS version_count,
                BOOL_OR(ps.status = 'blocked') AS has_blocked,
                -- Only a local publish can be yanked; a proxied artifact is
                -- withdrawn upstream, which this instance learns about as an
                -- absent version rather than as a flag.
                false AS has_yanked,
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
                -- A yank used to be reported in this column, which made an
                -- owner withdrawing a version indistinguishable from an
                -- operator blocking one. Two different facts, two different
                -- remedies; they now travel separately.
                false AS has_blocked,
                BOOL_OR(lp.yanked) AS has_yanked,
                false AS has_proxied,
                true AS has_local
            FROM local_packages lp
            WHERE lp.status = 'published'
              AND ($1::text IS NULL OR lp.registry = $1)
              AND ($2::text IS NULL OR lp.name ILIKE '%' || $2 || '%')
              AND ($3::text[] IS NULL OR lp.registry = ANY($3::text[]))
              {visibility}
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
                BOOL_OR(has_yanked) AS has_yanked,
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
            agg.has_yanked,
            agg.has_proxied,
            agg.has_local,
            COALESCE(ae.total_downloads, 0)::bigint AS total_downloads,
            ae.last_accessed,
            COALESCE(cm.cached_versions, 0)::bigint AS cached_versions,
            cm.cached_bytes,
            cm.last_fetched_at,
            nv.newest_version,
            nv.newest_published_at
        FROM agg
        LEFT JOIN LATERAL (
            SELECT COUNT(*)::bigint AS total_downloads, MAX(created_at) AS last_accessed
            FROM access_events
            WHERE registry = agg.registry AND package_name = agg.package_name
        ) ae ON true
        -- What this instance actually holds. `agg` is built from
        -- `package_statuses` and `local_packages`, both of which record that a
        -- version is *known*; only `artifact_cache_meta` records that its bytes
        -- are here. The difference between those two is exactly the Pending
        -- state, so it needs its own join rather than an inference from
        -- `version_count`.
        --
        -- SUM over a column that is NULL for every row is NULL, not 0, which is
        -- what carries "size unknown" through to the DTO: artifacts cached
        -- before 004 added `size_bytes` have no number, and reporting 0 for
        -- them would claim we hold empty files.
        -- COUNT(DISTINCT version), not COUNT(*): `artifact_cache_meta` is keyed
        -- on `artifact_key`, which for multi-file registries is one row *per
        -- file*. A Maven version with pom + jar + sources + javadoc is four
        -- rows and one version, and COUNT(*) reported it as four cached
        -- versions of a package whose `version_count` is 1.
        LEFT JOIN LATERAL (
            SELECT COUNT(DISTINCT acm.version)::bigint AS cached_versions,
                   SUM(acm.size_bytes)::bigint AS cached_bytes,
                   MAX(acm.cached_at) AS last_fetched_at
            FROM artifact_cache_meta acm
            WHERE acm.registry = agg.registry AND acm.package_name = agg.package_name
        ) cm ON true
        -- The version we most recently obtained, from either source, ordered by
        -- the timestamp that means "we got it": `cached_at` for a proxied
        -- artifact, `published_at` for a local one. NOT `MAX(version)` — version
        -- strings sort lexically, where '1.10.0' precedes '1.9.0', and no
        -- semantic ordering is available in SQL here.
        --
        -- `published_at` is carried out alongside because the release-age gate
        -- needs it for this specific version; it is NULL for proxied artifacts,
        -- which is a case the gate already defines behaviour for.
        LEFT JOIN LATERAL (
            SELECT v.version AS newest_version, v.published_at AS newest_published_at
            FROM (
                SELECT acm.version, NULL::timestamptz AS published_at, acm.cached_at AS obtained_at
                FROM artifact_cache_meta acm
                WHERE acm.registry = agg.registry AND acm.package_name = agg.package_name
                UNION ALL
                -- The same `{visibility}` predicate the `local_pkgs` CTE
                -- applies. A row can reach `agg` through `package_statuses`
                -- alone (`record_access` writes one on any allowed download or
                -- metadata read, including on the local path), so without it
                -- this join hands an anonymous caller the newest version string
                -- and publish date of a `team`-visibility package that its own
                -- owner happened to pull once.
                SELECT lp.version, lp.published_at, lp.published_at AS obtained_at
                FROM local_packages lp
                WHERE lp.registry = agg.registry AND lp.name = agg.package_name
                  AND lp.status = 'published'
                  {visibility}
            ) v
            ORDER BY v.obtained_at DESC NULLS LAST
            LIMIT 1
        ) nv ON true
        ORDER BY {order}
        LIMIT $7 OFFSET $8
        "#
    );

    // $4/$5/$6 are the viewer parameters consumed by LOCAL_VISIBILITY_PREDICATE.
    // They sit before limit/offset so the same fragment can be spliced into this
    // query and into `count_explore_packages_impl` (which has no limit/offset)
    // without renumbering — the two must apply an identical predicate or the
    // reported total would not match the rows returned.
    let rows = sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(&filter.registry)
        .bind(&filter.name_contains)
        .bind(registries)
        .bind(filter.viewer.is_admin)
        .bind(filter.viewer.is_authenticated)
        .bind(&groups)
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
    let groups = filter.viewer.normalised_groups();
    let visibility = LOCAL_VISIBILITY_PREDICATE;

    // Must apply exactly the same visibility predicate as
    // `explore_packages_impl`, or the pagination total would count rows the page
    // itself filters out — the UI would show "37 packages" over an empty page.
    let sql = format!(
        r#"
        WITH proxied AS (
            SELECT registry, package_name
            FROM package_statuses
            WHERE ($1::text IS NULL OR registry = $1)
              AND ($2::text IS NULL OR package_name ILIKE '%' || $2 || '%')
              AND ($3::text[] IS NULL OR registry = ANY($3::text[]))
        ),
        local_pkgs AS (
            SELECT lp.registry, lp.name AS package_name
            FROM local_packages lp
            WHERE lp.status = 'published'
              AND ($1::text IS NULL OR lp.registry = $1)
              AND ($2::text IS NULL OR lp.name ILIKE '%' || $2 || '%')
              AND ($3::text[] IS NULL OR lp.registry = ANY($3::text[]))
              {visibility}
        )
        SELECT COUNT(*) AS total FROM (
            SELECT registry, package_name FROM proxied
            UNION
            SELECT registry, package_name FROM local_pkgs
        ) combined
        "#
    );

    let row = sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(&filter.registry)
        .bind(&filter.name_contains)
        .bind(registries)
        .bind(filter.viewer.is_admin)
        .bind(filter.viewer.is_authenticated)
        .bind(&groups)
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
        ),
        -- SUM over all-NULL sizes is NULL, which is how "we hold artifacts but
        -- recorded no sizes for them" stays distinguishable from "we hold
        -- nothing". A registry with no cached artifacts at all has no row here
        -- and the LEFT JOIN leaves it NULL too — both are honestly unknown.
        cached_sizes AS (
            SELECT registry, SUM(size_bytes)::bigint AS cached_bytes
            FROM artifact_cache_meta
            WHERE ($1::text[] IS NULL OR registry = ANY($1::text[]))
            GROUP BY registry
        )
        SELECT
            pc.registry,
            pc.package_count,
            COALESCE(dc.total_downloads, 0) AS total_downloads,
            cs.cached_bytes
        FROM pkg_counts pc
        LEFT JOIN download_counts dc ON dc.registry = pc.registry
        LEFT JOIN cached_sizes cs ON cs.registry = pc.registry
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
            let cached_bytes: Option<i64> = r.get("cached_bytes");
            RegistryStat {
                registry: r.get("registry"),
                package_count: pkg_count as u64,
                total_downloads: downloads as u64,
                cached_bytes: cached_bytes.map(|b| b.max(0) as u64),
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

/// Distinct audit subjects, most-recently-active first (RFC 0004-bis A8).
///
/// Ordered by last activity rather than alphabetically: a suggesting field is
/// answering "who has been doing things here", and the operator is almost
/// always looking for someone recent. `ILIKE` because a subject typed as
/// `alice` should find `oidc:alice` — matching only a prefix would reproduce
/// the very failure this endpoint exists to fix.
pub(super) async fn distinct_event_subjects_impl(
    pool: &PgPool,
    contains: Option<&str>,
    limit: u64,
) -> Result<Vec<String>, CoreError> {
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT user_id FROM access_events \
         WHERE user_id IS NOT NULL AND ($1::text IS NULL OR user_id ILIKE '%' || $1 || '%') \
         GROUP BY user_id ORDER BY MAX(created_at) DESC LIMIT $2",
    )
    .bind(contains)
    .bind(limit as i64)
    .fetch_all(pool)
    .await
    .db_err()?;
    Ok(rows.into_iter().map(|(id,)| id).collect())
}
