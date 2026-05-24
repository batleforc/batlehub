ALTER TABLE local_packages
    ADD COLUMN IF NOT EXISTS signature_bytes BYTEA,
    ADD COLUMN IF NOT EXISTS signature_type  TEXT;
