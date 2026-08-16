-- Periodic cache-statistics rollup behind the admin dashboard's trend
-- (RFC 0004 §2.3, §6.3).
--
-- `hits`/`misses` are the window's own deltas, not running totals: the live
-- counters in `StatsResponse` are `since_startup` and reset with the process,
-- so a bounded per-hour row is what survives a restart. `cached_bytes` is a
-- gauge read at the end of the window, not a delta.
--
-- Deliberately separate from `access_events` (RFC 0004 §5.2): that table is an
-- audit trail with its own purge semantics, and deriving an operational chart
-- from it would let an audit purge silently rewrite a dashboard.
--
-- The table holds no principal and no coordinate, so its retention is an
-- operational question rather than a privacy one.
CREATE TABLE IF NOT EXISTS stats_history (
    registry     TEXT        NOT NULL,
    window_start TIMESTAMPTZ NOT NULL,
    hits         BIGINT      NOT NULL DEFAULT 0,
    misses       BIGINT      NOT NULL DEFAULT 0,
    cached_bytes BIGINT      NOT NULL DEFAULT 0,
    PRIMARY KEY (registry, window_start)
);

-- Reads are always "the last N days", across every registry.
CREATE INDEX IF NOT EXISTS idx_stats_history_window
    ON stats_history (window_start);
