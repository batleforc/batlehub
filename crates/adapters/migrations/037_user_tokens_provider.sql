-- Qualify personal access tokens by the auth provider that minted them.
--
-- `user_id` alone is a bare string chosen by whichever provider authenticated
-- the caller: an OIDC `sub`, a Kubernetes service account username, or a
-- `user_id` an operator typed into a static `[[auth.tokens]]` entry. Listing and
-- revoking matched on that string only, so any identity that happened to carry
-- the same one could enumerate and destroy another principal's tokens.
--
-- Backfill: every existing row was necessarily created through an OIDC session,
-- because that is the only kind `create_token` has ever accepted. The literal
-- 'oidc' is the historical default provider name and the only value the old
-- check would let through, so it is the correct value for rows that predate this
-- column — a deployment that renamed its provider could not create tokens at all
-- (that was the bug fixed alongside this) and therefore has no rows to migrate.
ALTER TABLE user_tokens
    ADD COLUMN IF NOT EXISTS provider TEXT NOT NULL DEFAULT 'oidc';

-- No default going forward: the caller states which provider it authenticated
-- through, rather than inheriting one silently.
ALTER TABLE user_tokens
    ALTER COLUMN provider DROP DEFAULT;

-- Token names are unique per principal, and a principal is now (provider,
-- user_id) rather than user_id alone. Kept as a unique *index* under the same
-- name it had in 003, because `create_token` maps a violation to a 409 by
-- matching on that name.
DROP INDEX IF EXISTS uq_user_token_name;

CREATE UNIQUE INDEX IF NOT EXISTS uq_user_token_name
    ON user_tokens (provider, user_id, name);
