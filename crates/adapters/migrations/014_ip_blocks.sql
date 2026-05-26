-- Tracks per-IP violation counts and active blocks for fail2ban-style IP blocking.
CREATE TABLE IF NOT EXISTS ip_violation_counters (
    ip           TEXT   NOT NULL,
    window_start BIGINT NOT NULL,
    count        BIGINT NOT NULL DEFAULT 1,
    PRIMARY KEY (ip, window_start)
);

CREATE INDEX IF NOT EXISTS idx_ip_violation_ip
    ON ip_violation_counters (ip);

CREATE TABLE IF NOT EXISTS ip_blocks (
    ip         TEXT   PRIMARY KEY,
    blocked_at BIGINT NOT NULL,
    unblock_at BIGINT NOT NULL,
    reason     TEXT   NOT NULL DEFAULT ''
);

CREATE INDEX IF NOT EXISTS idx_ip_blocks_unblock_at
    ON ip_blocks (unblock_at);
