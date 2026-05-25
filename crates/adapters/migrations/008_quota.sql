-- Per-user quota tracking for local/hybrid registries.
-- Tracks published bytes and package count per (user, registry) pair.
-- Usage is additive; the admin API resets rows to zero.

CREATE TABLE IF NOT EXISTS quota_usage (
    user_id         TEXT        NOT NULL,
    registry        TEXT        NOT NULL,
    bytes_published BIGINT      NOT NULL DEFAULT 0,
    packages_count  INTEGER     NOT NULL DEFAULT 0,
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (user_id, registry)
);

CREATE INDEX IF NOT EXISTS idx_quota_usage_registry ON quota_usage (registry);
