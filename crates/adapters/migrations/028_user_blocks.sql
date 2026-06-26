-- Tracks permanently blocked user accounts (by user_id string).
-- The auth middleware checks this table on every authenticated request.
CREATE TABLE IF NOT EXISTS user_blocks (
    user_id    TEXT        PRIMARY KEY,
    blocked_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    blocked_by TEXT        NOT NULL,
    reason     TEXT
);
