-- Fixed-window request counters for per-registry rate limiting.
-- Each row tracks the request count for a single (key, window_start) pair.
-- Old rows are pruned on write to prevent unbounded growth.
CREATE TABLE IF NOT EXISTS rate_limit_counters (
    key          TEXT   NOT NULL,
    window_start BIGINT NOT NULL,  -- Unix timestamp of the window's start (seconds)
    count        BIGINT NOT NULL DEFAULT 1,
    PRIMARY KEY (key, window_start)
);

CREATE INDEX IF NOT EXISTS idx_rate_limit_key ON rate_limit_counters (key);
