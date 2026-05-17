-- Personal API tokens created by OIDC-authenticated users.
-- Only the SHA-256 hash of the raw token is stored.
-- revoked_at IS NULL means the token is active.
CREATE TABLE IF NOT EXISTS user_tokens (
    id           UUID PRIMARY KEY,
    user_id      TEXT NOT NULL,
    name         TEXT NOT NULL,
    token_hash   TEXT NOT NULL UNIQUE,
    role         TEXT NOT NULL,
    expires_at   TIMESTAMPTZ NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    revoked_at   TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_user_tokens_user_id ON user_tokens (user_id);
CREATE INDEX IF NOT EXISTS idx_user_tokens_hash    ON user_tokens (token_hash);
-- Token names must be unique per user
CREATE UNIQUE INDEX IF NOT EXISTS uq_user_token_name ON user_tokens (user_id, name);
