-- Tracks which storage backend holds each artifact.
-- Populated on first store; missing rows mean "use the default backend" (pre-migration artifacts).
CREATE TABLE IF NOT EXISTS artifact_storage (
    storage_key   TEXT PRIMARY KEY,
    backend_name  TEXT NOT NULL,
    stored_at     TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_artifact_storage_backend ON artifact_storage (backend_name);
