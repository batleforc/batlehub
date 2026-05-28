ALTER TABLE local_packages
    ADD COLUMN IF NOT EXISTS visibility TEXT NOT NULL DEFAULT 'public'
        CHECK (visibility IN ('public', 'internal', 'team'));
