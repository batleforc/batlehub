-- When a personal access token was last presented.
--
-- Two things it makes possible that nothing else does: telling a compromised
-- token from a dormant one after a leak, and telling a user which of their
-- tokens they can safely revoke.
--
-- NULL means "never used since this column existed", which is not the same as
-- "never used" — the distinction matters when reading rows created before this
-- migration, so the column is deliberately nullable rather than defaulted to
-- `created_at`.
ALTER TABLE user_tokens
    ADD COLUMN IF NOT EXISTS last_used_at TIMESTAMPTZ;
