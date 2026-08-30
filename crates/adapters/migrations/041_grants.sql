-- RFC 0015 §6.3 — grants at the package and version tiers.
--
-- The registry and namespace tiers live in the config file, are reviewed like
-- any other change, and are already built at load (`server/src/grants.rs`).
-- These two cannot: §4.1, "a registry with 200 000 packages will not enumerate
-- them in TOML, let alone their two million versions". So they are written
-- through the admin API and stored here.
--
-- # Why `node_kind` + `node_key` rather than a column per tier
--
-- One row shape for both tiers, because resolution treats them the same way —
-- a version node is a fourth level, not a special case (§4.1). A `package`
-- column that is NULL for version rows would make every query say
-- `WHERE (package = $1 OR package IS NULL)`, which is the shape survey
-- finding 2 came in on: a predicate that is *vacuous* rather than absent.
--
-- `node_key` is the coordinate the tier names:
--
--   package  →  the package name          e.g. '@acme/billing/cards'
--   version  →  'package@version'         e.g. '@acme/billing/cards@1.4.2'
--
-- Not two columns, because the pair is only ever read whole: resolution asks
-- "what is written on this exact node", never "every version row of this
-- package regardless of which". One key, one lookup, no partial match to get
-- wrong.
--
-- # There is deliberately no seal here
--
-- §4.3: sealing is "a config-file construct, and only a config-file construct",
-- expressible at the registry and namespace tiers alone. It is the one thing in
-- the model that takes access away, and a delegate holding `owners:write` may
-- write package and version rows — so a seal representable in this table would
-- let them lock the registry owner out of a package, which is revocation
-- reintroduced one tier below the model built to exclude it.
--
-- That is enforced by the schema rather than by a check: an empty grant map is
-- what a seal *is*, and a row here always carries a subject and its permissions.
-- A package-tier seal is not a rejected request but an unwritable one, which is
-- what §7 asks for and what `crates/core`'s tests assert.
CREATE TABLE IF NOT EXISTS grants (
    id          BIGSERIAL PRIMARY KEY,
    registry    TEXT NOT NULL,
    -- 'package' or 'version'. A text discriminant rather than an enum type:
    -- the tiers are RFC 0015 §4.1's and adding one is a code change that has to
    -- reach the resolver anyway, so a CHECK keeps the two in step without a
    -- migration to extend a type.
    node_kind   TEXT NOT NULL,
    node_key    TEXT NOT NULL,
    -- The grant's left-hand side, in its wire spelling: '*', 'role:user',
    -- 'group:oidc1:eng', 'group:*:eng', 'group::eng', 'user:alice',
    -- 'token:release-bot'. Stored as written so `explain` can report the form
    -- that matched — naming the tier alone leaves an operator searching the
    -- block for the row.
    subject     TEXT NOT NULL,
    -- The resolved verbs, already expanded. §4.2: expansion happens at load,
    -- "never at evaluation time, so an expansion is a fact about the loaded
    -- model rather than something implied at each decision". A row holding
    -- 'releases:*' would move that decision back onto every request.
    actions     TEXT[] NOT NULL,
    granted_by  TEXT,
    granted_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT ck_grants_node_kind CHECK (node_kind IN ('package', 'version')),
    -- Non-empty, because an empty array is a seal by another spelling and seals
    -- are unwritable here. Without this the one construct §4.3 confines to the
    -- config file becomes expressible through the admin API by accident.
    CONSTRAINT ck_grants_actions_non_empty CHECK (cardinality(actions) > 0),
    -- One row per subject per node: repeating a subject is a union in the
    -- model, and two rows would make the union depend on which was read first.
    CONSTRAINT uq_grants_node_subject UNIQUE (registry, node_kind, node_key, subject)
);

-- The resolution query, and the only one on the hot path: "what is written on
-- the package and version nodes for this coordinate". Both tiers in one index
-- because both are fetched together — a read that took two round trips would
-- double the cost §11.7 measures with a 2 ms p99 budget.
CREATE INDEX IF NOT EXISTS idx_grants_lookup
    ON grants (registry, node_key);
