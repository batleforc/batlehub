-- Deprecation & unlisting state for locally published packages.
-- `deprecated` keeps a version listed and downloadable but flags it (optionally
-- with a message); `unlisted` hides a version from registry-protocol listings
-- while leaving it downloadable by exact coordinate. Mirrors the `yanked` column.
ALTER TABLE local_packages ADD COLUMN IF NOT EXISTS deprecated BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE local_packages ADD COLUMN IF NOT EXISTS deprecation_message TEXT;
ALTER TABLE local_packages ADD COLUMN IF NOT EXISTS unlisted BOOLEAN NOT NULL DEFAULT FALSE;
