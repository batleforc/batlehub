-- Tracks packages published directly to BatleHub (private/local registry mode).
-- index_metadata stores the full sparse-index JSON line for Cargo, or the
-- equivalent for other ecosystems (npm version object, etc.).
CREATE TABLE IF NOT EXISTS local_packages (
    id              SERIAL PRIMARY KEY,
    registry        TEXT NOT NULL,
    name            TEXT NOT NULL,
    version         TEXT NOT NULL,
    checksum        TEXT NOT NULL,
    yanked          BOOLEAN NOT NULL DEFAULT FALSE,
    index_metadata  JSONB NOT NULL,
    published_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    published_by    TEXT,
    CONSTRAINT uq_local_package UNIQUE (registry, name, version)
);

CREATE INDEX IF NOT EXISTS idx_local_packages_registry_name
    ON local_packages (registry, name);
