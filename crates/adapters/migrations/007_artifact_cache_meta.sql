-- Tracks every artifact stored in the proxy cache: used for eviction policy
-- enforcement (TTL, idle-time, max versions per package, LRU size cap) and
-- cache coherence checking.
CREATE TABLE IF NOT EXISTS artifact_cache_meta (
    artifact_key     TEXT PRIMARY KEY,
    registry         TEXT NOT NULL,
    package_name     TEXT NOT NULL,
    version          TEXT NOT NULL,
    size_bytes       BIGINT,
    cached_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_accessed_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_acm_registry_package ON artifact_cache_meta (registry, package_name);
CREATE INDEX IF NOT EXISTS idx_acm_last_accessed    ON artifact_cache_meta (last_accessed_at);
CREATE INDEX IF NOT EXISTS idx_acm_cached_at        ON artifact_cache_meta (registry, cached_at DESC);

-- Content-hash deduplication index: maps a SHA-256 content hash to the
-- single storage key that holds the actual bytes.
CREATE TABLE IF NOT EXISTS artifact_dedup_index (
    content_hash TEXT PRIMARY KEY,
    content_key  TEXT NOT NULL,
    ref_count    INTEGER NOT NULL DEFAULT 1,
    size_bytes   BIGINT
);

-- Maps every logical artifact key to its content hash (many logical → one blob).
CREATE TABLE IF NOT EXISTS artifact_dedup_refs (
    logical_key  TEXT PRIMARY KEY,
    content_hash TEXT NOT NULL REFERENCES artifact_dedup_index (content_hash)
);

CREATE INDEX IF NOT EXISTS idx_dedup_refs_hash ON artifact_dedup_refs (content_hash);
