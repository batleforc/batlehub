-- Access audit log: every proxy request, allowed or denied.
CREATE TABLE IF NOT EXISTS access_events (
    id               UUID PRIMARY KEY,
    user_id          TEXT,
    user_role        TEXT NOT NULL DEFAULT 'anonymous',
    registry         TEXT NOT NULL,
    package_name     TEXT NOT NULL,
    package_version  TEXT NOT NULL,
    package_artifact TEXT,
    action           TEXT NOT NULL,   -- download | view_metadata | block | unblock
    outcome          TEXT NOT NULL,   -- allowed | denied
    deny_reason      TEXT,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_access_events_registry        ON access_events (registry);
CREATE INDEX IF NOT EXISTS idx_access_events_user_id         ON access_events (user_id);
CREATE INDEX IF NOT EXISTS idx_access_events_created_at      ON access_events (created_at DESC);
CREATE INDEX IF NOT EXISTS idx_access_events_outcome         ON access_events (outcome);

-- Administrative package status (available / blocked).
CREATE TABLE IF NOT EXISTS package_statuses (
    id               UUID PRIMARY KEY,
    registry         TEXT NOT NULL,
    package_name     TEXT NOT NULL,
    package_version  TEXT NOT NULL,
    package_artifact TEXT,
    status           TEXT NOT NULL DEFAULT 'available',  -- available | blocked
    block_reason     TEXT,
    blocked_by       TEXT,
    blocked_at       TIMESTAMPTZ,
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_package_statuses_registry ON package_statuses (registry);
CREATE INDEX IF NOT EXISTS idx_package_statuses_status   ON package_statuses (status);

-- Unique per (registry, name, version, artifact).
-- Expressed as a functional index so COALESCE can treat NULL artifact as ''.
CREATE UNIQUE INDEX IF NOT EXISTS uq_package_status
    ON package_statuses (registry, package_name, package_version, COALESCE(package_artifact, ''));
