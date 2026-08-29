use std::collections::BTreeMap;
use std::sync::Arc;

use actix_web::{get, web, HttpResponse, Responder};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use batlehub_core::ports::StatsHistoryRepository;

use crate::{error::AppError, extractors::AuthIdentity};

const DEFAULT_WINDOW_DAYS: i64 = 30;
const MAX_WINDOW_DAYS: i64 = 365;

#[derive(Debug, Deserialize, IntoParams)]
pub struct StatsHistoryQuery {
    /// How far back to read, as a day count with an optional `d` suffix
    /// (`30`, `30d`, `7d`). Clamped to 1–365; defaults to 30.
    pub window: Option<String>,
}

/// Parse `30d` / `30` into a day count, clamped.
///
/// Lenient on purpose: this drives a dashboard, and an unparseable window is
/// better answered with the default series than with a 400 that leaves the
/// panel blank.
fn window_days(raw: Option<&str>) -> i64 {
    raw.map(|s| s.trim().trim_end_matches(['d', 'D']))
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(DEFAULT_WINDOW_DAYS)
        .clamp(1, MAX_WINDOW_DAYS)
}

/// One hour of one registry's cache activity.
#[derive(Debug, Serialize, ToSchema)]
pub struct StatsHistoryPointDto {
    pub window_start: DateTime<Utc>,
    /// Hits **in this window**, not a running total — which is the whole point
    /// of the table: a counter since process start cannot survive a deploy.
    pub hits: u64,
    pub misses: u64,
    /// Bytes in storage at the end of the window. A level, not a delta.
    pub cached_bytes: u64,
}

/// One registry's series over the requested window.
#[derive(Debug, Serialize, ToSchema)]
pub struct RegistryStatsHistoryDto {
    pub registry: String,
    pub points: Vec<StatsHistoryPointDto>,
}

/// The dashboard's trend: this window against the one before it.
///
/// Both halves are computed server-side so the two numbers being compared are
/// always derived the same way. A client differencing two separate requests
/// would be one off-by-one away from comparing 30 days with 29.
#[derive(Debug, Serialize, ToSchema)]
pub struct StatsTrendDto {
    pub hits: u64,
    pub misses: u64,
    /// Hit rate in [0, 1] for the requested window, or `null` when it saw no
    /// traffic at all — which is not the same as a hit rate of zero.
    pub hit_rate: Option<f64>,
    /// The same rate over the equally-long window immediately before it, or
    /// `null` when there is no history that far back yet.
    pub previous_hit_rate: Option<f64>,
    /// `hit_rate - previous_hit_rate`, or `null` when either side is unknown.
    /// Present so a reader is not left subtracting two nullable numbers.
    pub delta: Option<f64>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct StatsHistoryResponse {
    /// The window actually used, after clamping.
    pub window_days: i64,
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    pub trend: StatsTrendDto,
    /// Per registry, oldest point first. Registries with no rows in the window
    /// are absent rather than present-and-empty.
    pub per_registry: Vec<RegistryStatsHistoryDto>,
}

fn hit_rate(hits: u64, misses: u64) -> Option<f64> {
    let total = hits + misses;
    (total > 0).then(|| hits as f64 / total as f64)
}

/// Persisted cache statistics over a window (admin).
///
/// Answers the question `/api/v1/admin/stats` structurally cannot: its counters
/// are `since_startup` and reset with the process, so "is the hit rate
/// improving" has no answer there (RFC 0004 §2.3). This reads the hourly rollup
/// instead, which is bounded per window and survives a deploy.
#[utoipa::path(
    get,
    path = "/api/v1/admin/stats/history",
    tag = "back-office",
    params(StatsHistoryQuery),
    responses(
        (status = 200, description = "Persisted cache statistics for the window, with the trend against the previous one", body = StatsHistoryResponse),
        (status = 403, description = "`stats:read` required"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/api/v1/admin/stats/history")]
pub async fn admin_stats_history(
    identity: AuthIdentity,
    query: web::Query<StatsHistoryQuery>,
    history: web::Data<Arc<dyn StatsHistoryRepository>>,
    proxy_svc: web::Data<Arc<batlehub_core::services::ProxyService>>,
) -> Result<impl Responder, AppError> {
    // RFC 0015 §4.2 — `stats:read` in place of `require_admin`. Computed from the
    // **configured** registries rather than from the rows that came back: a
    // window with no traffic in it has no rows, and deriving the gate from those
    // would refuse a caller who simply asked about a quiet week.
    let days = window_days(query.window.as_deref());
    let to = Utc::now();
    let from = to - Duration::days(days);
    let previous_from = from - Duration::days(days);

    let rows = history
        .read_window(from, to)
        .await
        .map_err(AppError::from)?;
    let previous = history
        .read_window(previous_from, from)
        .await
        .map_err(AppError::from)?;

    // The candidate universe is the **configured hierarchy**, not the rows that
    // came back. A window with no traffic in it has no rows, and a gate derived
    // from those would refuse a caller who simply asked about a quiet week —
    // while a registry that has rows and no hierarchy is one whose numbers
    // nobody has been granted, so it drops out either way (see
    // `registries_with_stats_read`, which fails closed on exactly that).
    let candidates: Vec<String> = {
        let hot = proxy_svc.hot.read().await;
        hot.grants.keys().cloned().collect()
    };
    let permitted =
        super::stats::registries_with_stats_read(&identity, &candidates, &proxy_svc.hot).await?;

    // §4.4 — applied **before** the sums below rather than to the series
    // afterwards. A trend computed over a registry this caller may not read is
    // the same disclosure as a count over packages they may not see, and it is
    // the shape §4.4 calls the one that fails silently: nothing about a summed
    // number says which rows went into it.
    let rows: Vec<_> = rows
        .into_iter()
        .filter(|r| permitted.contains(&r.registry))
        .collect();
    let previous: Vec<_> = previous
        .into_iter()
        .filter(|r| permitted.contains(&r.registry))
        .collect();

    let mut by_registry: BTreeMap<String, Vec<StatsHistoryPointDto>> = BTreeMap::new();
    let (mut hits, mut misses) = (0_u64, 0_u64);
    for row in rows {
        hits = hits.saturating_add(row.hits);
        misses = misses.saturating_add(row.misses);
        by_registry
            .entry(row.registry)
            .or_default()
            .push(StatsHistoryPointDto {
                window_start: row.window_start,
                hits: row.hits,
                misses: row.misses,
                cached_bytes: row.cached_bytes,
            });
    }

    let (mut prev_hits, mut prev_misses) = (0_u64, 0_u64);
    for row in previous {
        prev_hits = prev_hits.saturating_add(row.hits);
        prev_misses = prev_misses.saturating_add(row.misses);
    }

    let current_rate = hit_rate(hits, misses);
    let previous_rate = hit_rate(prev_hits, prev_misses);

    Ok(HttpResponse::Ok().json(StatsHistoryResponse {
        window_days: days,
        from,
        to,
        trend: StatsTrendDto {
            hits,
            misses,
            hit_rate: current_rate,
            previous_hit_rate: previous_rate,
            delta: current_rate.zip(previous_rate).map(|(a, b)| a - b),
        },
        per_registry: by_registry
            .into_iter()
            .map(|(registry, points)| RegistryStatsHistoryDto { registry, points })
            .collect(),
    }))
}

#[cfg(test)]
mod tests {
    use super::{hit_rate, window_days, DEFAULT_WINDOW_DAYS, MAX_WINDOW_DAYS};

    #[test]
    fn window_accepts_a_bare_count_and_a_d_suffix() {
        assert_eq!(window_days(Some("7")), 7);
        assert_eq!(window_days(Some("7d")), 7);
        assert_eq!(window_days(Some(" 30D ")), 30);
    }

    #[test]
    fn window_falls_back_rather_than_failing() {
        assert_eq!(window_days(None), DEFAULT_WINDOW_DAYS);
        assert_eq!(
            window_days(Some("last tuesday")),
            DEFAULT_WINDOW_DAYS,
            "a dashboard is better served the default series than a blank panel"
        );
    }

    #[test]
    fn window_is_clamped_at_both_ends() {
        assert_eq!(window_days(Some("0")), 1);
        assert_eq!(window_days(Some("-5")), 1);
        assert_eq!(window_days(Some("100000")), MAX_WINDOW_DAYS);
    }

    #[test]
    fn no_traffic_is_an_unknown_rate_not_a_zero_one() {
        assert_eq!(hit_rate(0, 0), None);
        assert_eq!(hit_rate(0, 5), Some(0.0));
        assert_eq!(hit_rate(3, 1), Some(0.75));
    }
}
