use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Datelike, Duration, TimeZone, Timelike, Utc};
use tokio::sync::Mutex;

use crate::{
    error::CoreError,
    ports::{StatsHistoryRepository, StatsRollupRow},
    services::ProxyMetrics,
};

/// Truncate a timestamp to the start of its hour.
///
/// The interval is fixed at one hour rather than configured (RFC 0004 R9): it
/// is the resolution the data is *kept* at, and a deployment wanting daily
/// figures aggregates on read. Two instances with different intervals would
/// have incomparable histories for no gain.
pub fn hour_start(at: DateTime<Utc>) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(at.year(), at.month(), at.day(), at.hour(), 0, 0)
        .single()
        .unwrap_or(at)
}

/// Turns the live, since-startup counters into a durable per-hour series.
///
/// `ProxyMetrics` counts monotonically from process start, so the value at any
/// instant is meaningless across a restart — which is exactly the defect
/// RFC 0004 §2.3 describes. This writes the **difference** since the previous
/// tick, so each row is a closed observation of its own window and nothing
/// later depends on the counter that produced it.
///
/// A restart therefore costs at most the current partial hour: the baseline
/// resets to zero along with the counters, and the next tick writes what
/// accumulated since the process came up. It never produces a negative or
/// absurd delta, which a naive "store the counter" scheme would on every
/// restart.
pub struct StatsRollupService {
    metrics: Arc<ProxyMetrics>,
    repo: Arc<dyn StatsHistoryRepository>,
    /// Counter values at the previous tick, per registry.
    baseline: Mutex<HashMap<String, (u64, u64)>>,
    /// How many days of history to keep. `0` disables pruning entirely.
    retention_days: u32,
}

impl StatsRollupService {
    pub fn new(
        metrics: Arc<ProxyMetrics>,
        repo: Arc<dyn StatsHistoryRepository>,
        retention_days: u32,
    ) -> Self {
        Self {
            metrics,
            repo,
            baseline: Mutex::new(HashMap::new()),
            retention_days,
        }
    }

    /// [`tick`](Self::tick) at the current instant.
    ///
    /// The caller-supplied-`now` form exists for tests; a scheduler has no
    /// reason to name the time itself, and threading a clock through the server
    /// binary just to call this would put `chrono` in its dependency list for
    /// one expression.
    pub async fn tick_now(
        &self,
        cached_bytes_by_registry: &HashMap<String, u64>,
    ) -> Result<usize, CoreError> {
        self.tick(Utc::now(), cached_bytes_by_registry).await
    }

    /// Write one window's rows and prune anything past retention.
    ///
    /// `cached_bytes_by_registry` is supplied by the caller rather than read
    /// here: it comes from the storage backend, which `core` has no business
    /// reaching into from a rollup.
    pub async fn tick(
        &self,
        now: DateTime<Utc>,
        cached_bytes_by_registry: &HashMap<String, u64>,
    ) -> Result<usize, CoreError> {
        let window_start = hour_start(now);
        let mut baseline = self.baseline.lock().await;

        let mut rows = Vec::new();
        for (registry, counters) in self.metrics.all() {
            let (hits_now, misses_now) = (counters.hits(), counters.misses());
            let (hits_prev, misses_prev) = baseline.get(registry).copied().unwrap_or((0, 0));

            // `saturating_sub` rather than `-`: a counter that went backwards
            // means the process restarted between ticks, and 0 is the honest
            // answer for a window we cannot account for.
            let hits = hits_now.saturating_sub(hits_prev);
            let misses = misses_now.saturating_sub(misses_prev);
            baseline.insert(registry.clone(), (hits_now, misses_now));

            rows.push(StatsRollupRow {
                registry: registry.clone(),
                window_start,
                hits,
                misses,
                cached_bytes: cached_bytes_by_registry.get(registry).copied().unwrap_or(0),
            });
        }
        drop(baseline);

        if rows.is_empty() {
            return Ok(0);
        }
        self.repo.append(&rows).await?;

        if self.retention_days > 0 {
            let cutoff = now - Duration::days(i64::from(self.retention_days));
            self.repo.prune_before(cutoff).await?;
        }
        Ok(rows.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::Mutex as StdMutex;

    #[derive(Default)]
    struct MemHistory {
        rows: StdMutex<Vec<StatsRollupRow>>,
        pruned_before: StdMutex<Option<DateTime<Utc>>>,
    }

    #[async_trait]
    impl StatsHistoryRepository for MemHistory {
        async fn append(&self, rows: &[StatsRollupRow]) -> Result<(), CoreError> {
            let mut guard = self.rows.lock().unwrap();
            for row in rows {
                guard.retain(|r| {
                    !(r.registry == row.registry && r.window_start == row.window_start)
                });
                guard.push(row.clone());
            }
            Ok(())
        }
        async fn read_window(
            &self,
            from: DateTime<Utc>,
            to: DateTime<Utc>,
        ) -> Result<Vec<StatsRollupRow>, CoreError> {
            Ok(self
                .rows
                .lock()
                .unwrap()
                .iter()
                .filter(|r| r.window_start >= from && r.window_start < to)
                .cloned()
                .collect())
        }
        async fn prune_before(&self, cutoff: DateTime<Utc>) -> Result<u64, CoreError> {
            *self.pruned_before.lock().unwrap() = Some(cutoff);
            let mut guard = self.rows.lock().unwrap();
            let before = guard.len();
            guard.retain(|r| r.window_start >= cutoff);
            Ok((before - guard.len()) as u64)
        }
    }

    fn svc(retention_days: u32) -> (Arc<ProxyMetrics>, Arc<MemHistory>, StatsRollupService) {
        let metrics = Arc::new(ProxyMetrics::new(&["npm".to_owned()]));
        let repo = Arc::new(MemHistory::default());
        let svc = StatsRollupService::new(
            Arc::clone(&metrics),
            Arc::clone(&repo) as Arc<dyn StatsHistoryRepository>,
            retention_days,
        );
        (metrics, repo, svc)
    }

    fn at(hour: u32, minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 12, hour, minute, 0).unwrap()
    }

    #[test]
    fn hour_start_truncates_minutes_and_seconds() {
        assert_eq!(hour_start(at(13, 47)), at(13, 0));
        assert_eq!(hour_start(at(13, 0)), at(13, 0));
    }

    #[tokio::test]
    async fn first_tick_records_everything_since_startup() {
        let (metrics, repo, svc) = svc(0);
        for _ in 0..7 {
            metrics.record_artifact_hit("npm");
        }
        metrics.record_artifact_miss("npm");

        svc.tick(at(10, 30), &HashMap::new()).await.unwrap();

        let rows = repo.rows.lock().unwrap().clone();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].hits, 7);
        assert_eq!(rows[0].misses, 1);
        assert_eq!(rows[0].window_start, at(10, 0), "truncated to the hour");
    }

    /// The property the whole table exists for: a row is a window's own
    /// traffic, not a running total, so the series stays readable across a
    /// restart.
    #[tokio::test]
    async fn each_row_holds_only_its_own_window() {
        let (metrics, repo, svc) = svc(0);

        for _ in 0..10 {
            metrics.record_artifact_hit("npm");
        }
        svc.tick(at(10, 0), &HashMap::new()).await.unwrap();

        // Three more hits in the next hour: the second row says 3, not 13.
        for _ in 0..3 {
            metrics.record_artifact_hit("npm");
        }
        svc.tick(at(11, 0), &HashMap::new()).await.unwrap();

        let rows = repo.rows.lock().unwrap().clone();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].hits, 10);
        assert_eq!(rows[1].hits, 3, "the window's delta, not the counter");
    }

    #[tokio::test]
    async fn a_quiet_window_records_zero_rather_than_nothing() {
        let (metrics, repo, svc) = svc(0);
        metrics.record_artifact_hit("npm");
        svc.tick(at(10, 0), &HashMap::new()).await.unwrap();
        svc.tick(at(11, 0), &HashMap::new()).await.unwrap();

        let rows = repo.rows.lock().unwrap().clone();
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows[1].hits, 0,
            "an hour with no traffic is a fact, and a gap in the chart is not"
        );
    }

    /// A restart resets the counters. Without `saturating_sub` the next tick
    /// would underflow into an enormous delta and poison the chart.
    #[tokio::test]
    async fn a_counter_going_backwards_records_zero_not_an_underflow() {
        let (metrics, repo, svc) = svc(0);
        for _ in 0..5 {
            metrics.record_artifact_hit("npm");
        }
        svc.tick(at(10, 0), &HashMap::new()).await.unwrap();

        // Simulate the process coming back with fresh counters.
        let fresh = Arc::new(ProxyMetrics::new(&["npm".to_owned()]));
        fresh.record_artifact_hit("npm");
        let restarted = StatsRollupService {
            metrics: fresh,
            repo: Arc::clone(&repo) as Arc<dyn StatsHistoryRepository>,
            baseline: Mutex::new(HashMap::from([("npm".to_owned(), (5, 0))])),
            retention_days: 0,
        };
        restarted.tick(at(11, 0), &HashMap::new()).await.unwrap();

        let rows = repo.rows.lock().unwrap().clone();
        assert_eq!(rows[1].hits, 0, "1 - 5 is not 18446744073709551612");
    }

    #[tokio::test]
    async fn cached_bytes_is_a_level_not_a_delta() {
        let (_, repo, svc) = svc(0);
        let bytes = HashMap::from([("npm".to_owned(), 4_096_u64)]);
        svc.tick(at(10, 0), &bytes).await.unwrap();
        svc.tick(at(11, 0), &bytes).await.unwrap();

        let rows = repo.rows.lock().unwrap().clone();
        assert_eq!(rows[0].cached_bytes, 4_096);
        assert_eq!(
            rows[1].cached_bytes, 4_096,
            "storage did not grow by 4 KiB in the second hour — it *is* 4 KiB"
        );
    }

    #[tokio::test]
    async fn ticking_twice_in_one_window_does_not_double_count() {
        let (metrics, repo, svc) = svc(0);
        for _ in 0..4 {
            metrics.record_artifact_hit("npm");
        }
        svc.tick(at(10, 5), &HashMap::new()).await.unwrap();
        svc.tick(at(10, 55), &HashMap::new()).await.unwrap();

        let rows = repo.rows.lock().unwrap().clone();
        assert_eq!(rows.len(), 1, "same window, upserted");
        assert_eq!(rows[0].hits, 0, "the second tick saw no new traffic");
    }

    #[tokio::test]
    async fn retention_prunes_only_past_the_window() {
        let (_, repo, svc) = svc(30);
        svc.tick(at(10, 0), &HashMap::new()).await.unwrap();
        let cutoff = repo.pruned_before.lock().unwrap().unwrap();
        assert_eq!(cutoff, at(10, 0) - Duration::days(30));
    }

    #[tokio::test]
    async fn retention_of_zero_never_prunes() {
        let (_, repo, svc) = svc(0);
        svc.tick(at(10, 0), &HashMap::new()).await.unwrap();
        assert!(
            repo.pruned_before.lock().unwrap().is_none(),
            "0 disables pruning rather than pruning everything"
        );
    }
}
