use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::error::CoreError;

/// One registry's cache activity over one closed window.
///
/// `hits` and `misses` are **deltas for the window**, not running totals. That
/// is the whole point of the table (RFC 0004 §2.3): `StatsResponse` counters are
/// `since_startup` and reset with the process, so "is the hit rate improving"
/// has no answer. A row that is bounded to its own hour survives a restart,
/// because nothing later depends on the counter that produced it.
///
/// `cached_bytes` is the opposite — a gauge, read at the end of the window, not
/// accumulated. Bytes in storage is a level, and differencing a level would
/// report an eviction as negative traffic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatsRollupRow {
    pub registry: String,
    /// Start of the window, truncated to the hour (RFC 0004 R9).
    pub window_start: DateTime<Utc>,
    pub hits: u64,
    pub misses: u64,
    pub cached_bytes: u64,
}

/// Persists the periodic cache-statistics rollup behind the dashboard's trend.
///
/// Deliberately not derived from the access log (RFC 0004 §5.2): that is an
/// audit trail with its own retention and purge semantics, and deriving an
/// operational chart from it would mean a `AuditPurge` silently rewrote
/// history on a dashboard.
///
/// The table holds **no principal and no coordinate** — registry, window,
/// counters — so its retention is an operational choice rather than a privacy
/// one (RFC 0004 §7).
#[async_trait]
pub trait StatsHistoryRepository: Send + Sync {
    /// Append one window's rows.
    ///
    /// Merges per `(registry, window_start)` rather than inserting: a writer
    /// that runs twice for the same hour leaves one row, not two.
    ///
    /// It merges by **summing** `hits`/`misses`. Every row is a delta since the
    /// caller's previous tick ([`StatsRollupService`](crate::services::StatsRollupService)),
    /// and several ticks land in each hourly bucket carrying disjoint deltas, so
    /// an implementation that replaced on conflict would keep only the last —
    /// turning a redeploy at 11:20 into an erased 11:00 window rather than the
    /// lost partial tick the rollup's contract promises.
    ///
    /// `cached_bytes` is the opposite kind of number — a level read from
    /// storage, not a delta — and is replaced by the later value.
    async fn append(&self, rows: &[StatsRollupRow]) -> Result<(), CoreError>;

    /// Rows whose window starts within `[from, to)`, oldest first.
    async fn read_window(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<StatsRollupRow>, CoreError>;

    /// Delete rows whose window starts strictly before `cutoff`. Returns the
    /// number deleted.
    async fn prune_before(&self, cutoff: DateTime<Utc>) -> Result<u64, CoreError>;
}
