-- Re-serve integrity verification: store a self-computed SHA-256 (hex) of the
-- cached bytes alongside the cache metadata, so a later serve can re-hash the
-- stored bytes and compare. Additive and nullable: pre-existing cache rows have
-- no checksum and are treated as "skip re-verify" until they are next refreshed.
ALTER TABLE artifact_cache_meta
    ADD COLUMN IF NOT EXISTS checksum TEXT;
