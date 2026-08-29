-- RFC 0015 §4.2 — the GPG keys a Terraform namespace's providers are signed with.
--
-- Terraform verifies a provider's `SHASUMS` signature against the keys the
-- registry serves in its download response, and this server served a hardcoded
-- `{"gpg_public_keys": []}`. An empty list does not mean "unsigned, proceed": it
-- tells the client there is nothing to verify against, so no locally published
-- provider could be verified by anybody. The verb `terraform:signing-keys:write`
-- existed for this and had no store behind it.
--
-- # Keyed by namespace
--
-- A publisher signs every provider under their namespace with one key, which is
-- how the protocol is shaped and how §4.2 words the action ("a namespace's
-- providers"). It is also RFC 0015 §4.1's namespace tier, so the grant that
-- delegates this names exactly the scope the key covers.
--
-- # `key_id` is the identity, and the uniqueness is per namespace
--
-- Re-registering the same id replaces the armour, which is what a key rotation
-- that keeps its id looks like. Two namespaces may register the same id
-- independently — they are different publishers as far as this server knows, and
-- collapsing them would let one namespace's rotation silently change another's
-- verification material.
CREATE TABLE IF NOT EXISTS provider_signing_keys (
    id          BIGSERIAL PRIMARY KEY,
    registry    TEXT NOT NULL,
    namespace   TEXT NOT NULL,
    -- Terraform's field names, carried rather than renamed: these values are
    -- serialised straight into the protocol response.
    key_id      TEXT NOT NULL,
    ascii_armor TEXT NOT NULL,
    trust_signature TEXT,
    source      TEXT,
    source_url  TEXT,
    set_by      TEXT,
    set_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT ck_signing_key_id_not_empty CHECK (length(trim(key_id)) > 0),
    CONSTRAINT uq_signing_key UNIQUE (registry, namespace, key_id)
);

-- The read is "every key for this namespace", on the provider download path.
CREATE INDEX IF NOT EXISTS idx_provider_signing_keys_lookup
    ON provider_signing_keys (registry, namespace);
