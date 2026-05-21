-- Persistent metadata cache for proxy registries.
-- Used by PgCacheStore when cache.type = "postgres".
-- Metadata is stored as JSONB; expires_at NULL means no TTL.
CREATE TABLE IF NOT EXISTS metadata_cache (
    cache_key  TEXT PRIMARY KEY,
    metadata   JSONB NOT NULL,
    cached_at  TIMESTAMPTZ NOT NULL,
    expires_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_metadata_cache_expires_at ON metadata_cache (expires_at);
