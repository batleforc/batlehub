CREATE TABLE IF NOT EXISTS artifact_sboms (
    id           UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    artifact_key TEXT        NOT NULL,
    registry     TEXT        NOT NULL,
    package_name TEXT        NOT NULL,
    version      TEXT        NOT NULL,
    format       TEXT        NOT NULL,       -- 'spdx' | 'cyclonedx'
    spec_version TEXT        NOT NULL,       -- '2.3'  | '1.4'
    document     JSONB       NOT NULL,
    source       TEXT        NOT NULL,       -- 'generated' | 'upstream' | 'extracted'
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT uq_sbom UNIQUE (artifact_key, format)
);

CREATE INDEX IF NOT EXISTS idx_artifact_sboms_key      ON artifact_sboms (artifact_key);
CREATE INDEX IF NOT EXISTS idx_artifact_sboms_registry ON artifact_sboms (registry, package_name, version);
CREATE INDEX IF NOT EXISTS idx_artifact_sboms_created  ON artifact_sboms (registry, created_at DESC);
