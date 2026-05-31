CREATE TABLE IF NOT EXISTS config_changes (
    id           UUID        PRIMARY KEY,
    triggered_by TEXT        NOT NULL,
    triggered_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    -- "applied" | "rejected"
    status       TEXT        NOT NULL DEFAULT 'applied',
    -- {"added_registries": [...], "removed_registries": [...], "changed_registries": [...], ...}
    diff         JSONB       NOT NULL DEFAULT '{}',
    summary      TEXT        NOT NULL DEFAULT '',
    error_msg    TEXT
);

CREATE INDEX IF NOT EXISTS idx_config_changes_triggered_at
    ON config_changes (triggered_at DESC);
