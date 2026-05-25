-- Two-phase publish: a row inserted by begin_publish is 'pending' until the
-- artifact is safely stored and commit_publish promotes it to 'published'.
-- Rows that are still 'pending' after a crash are invisible to readers and
-- can be removed by cleanup_pending().
--
-- DEFAULT 'published' keeps every existing row visible without a data migration.
ALTER TABLE local_packages
    ADD COLUMN IF NOT EXISTS status TEXT NOT NULL DEFAULT 'published';

CREATE INDEX IF NOT EXISTS idx_local_packages_pending
    ON local_packages (registry, published_at)
    WHERE status = 'pending';
