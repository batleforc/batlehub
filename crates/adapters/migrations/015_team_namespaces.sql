CREATE TABLE IF NOT EXISTS team_namespaces (
    id         SERIAL PRIMARY KEY,
    registry   TEXT NOT NULL,
    prefix     TEXT NOT NULL,
    group_id   TEXT NOT NULL,
    claimed_by TEXT,
    claimed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT uq_team_namespace UNIQUE (registry, prefix)
);

CREATE INDEX IF NOT EXISTS idx_team_namespaces_registry ON team_namespaces (registry);
