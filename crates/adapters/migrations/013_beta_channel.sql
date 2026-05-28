-- Tracks per-registry beta-channel membership.
-- Members can see and download pre-release versions; non-members only see stable versions.
CREATE TABLE IF NOT EXISTS beta_channel_members (
    id             SERIAL PRIMARY KEY,
    registry       TEXT NOT NULL,
    principal_type TEXT NOT NULL CHECK (principal_type IN ('user', 'group')),
    principal_id   TEXT NOT NULL,
    granted_by     TEXT,
    granted_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT uq_beta_channel_member
        UNIQUE (registry, principal_type, principal_id)
);

CREATE INDEX IF NOT EXISTS idx_beta_channel_registry
    ON beta_channel_members (registry);
