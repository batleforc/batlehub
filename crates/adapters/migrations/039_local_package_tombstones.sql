-- RFC 0016 §4.4 — a published version coordinate is never occupied twice.
--
-- Delete stops removing the row. The version survives with `deleted_at` set and
-- `status = 'deleted'`; the artifact bytes are dropped. That single row is the
-- tombstone, and it carries four jobs at once: it refuses a re-publish of the
-- coordinate, it records who deleted it and when, it keeps counting as the
-- newest version for RFC 0015's `monotonic` check, and it is what a retention
-- run will later account against.
--
-- `status = 'deleted'` is redundant with `deleted_at IS NOT NULL` on purpose.
-- Every pre-existing reader already filters `status = 'published'`, so a query
-- somewhere that this change fails to reach still excludes tombstones rather
-- than serving a version whose bytes are gone. The explicit `deleted_at IS NULL`
-- predicate goes on the readers as well (RFC 0016 §6.3); this is the floor
-- under it, not a substitute for it.
--
-- No backfill is possible: rows deleted before this migration are gone and
-- their coordinates are, unavoidably, still free. The invariant starts here.
ALTER TABLE local_packages ADD COLUMN IF NOT EXISTS deleted_at TIMESTAMPTZ;
ALTER TABLE local_packages ADD COLUMN IF NOT EXISTS deleted_by TEXT;

-- RFC 0016 §4.5 — tombstone retention compacts, it never collects.
-- Set when the detail columns (index_metadata, checksum, signature, publisher)
-- have been stripped and only the coordinate claim remains. Distinguishes a
-- compacted tombstone from one whose detail was never recorded, which an
-- auditor reading the row needs to be able to tell apart.
ALTER TABLE local_packages ADD COLUMN IF NOT EXISTS detail_compacted_at TIMESTAMPTZ;

-- Compaction nulls `checksum`, so it can no longer be NOT NULL for the table.
-- The constraint that replaces it is narrower and states the actual invariant: a
-- *live* version always has a checksum, and only a tombstone may lack one. Every
-- reader of a live row still gets a non-null value, and one that goes missing is
-- now a constraint violation at write time rather than a decode panic at read.
--
-- `index_metadata` needs no such change: compaction sets it to `'{}'`, which is
-- three bytes and keeps its NOT NULL. `published_at` is not stripped at all —
-- eight bytes do not accumulate, and "how long did this coordinate live" is the
-- first question asked of a tombstone whose metadata is already gone.
ALTER TABLE local_packages ALTER COLUMN checksum DROP NOT NULL;

DO $$
BEGIN
    ALTER TABLE local_packages ADD CONSTRAINT ck_local_packages_live_checksum
        CHECK (deleted_at IS NOT NULL OR checksum IS NOT NULL);
EXCEPTION
    WHEN duplicate_object THEN NULL;
END $$;

-- The publish path looks up a tombstone by exact coordinate on every publish,
-- and the compaction sweep scans by age. Partial on `deleted_at IS NOT NULL`
-- so a live estate with no deletions pays nothing for either.
CREATE INDEX IF NOT EXISTS idx_local_packages_tombstones
    ON local_packages (registry, name, version)
    WHERE deleted_at IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_local_packages_tombstone_age
    ON local_packages (registry, deleted_at)
    WHERE deleted_at IS NOT NULL;
