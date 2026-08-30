-- RFC 0015 §6.3 — policy at the package and version tiers.
--
-- The twin of `041_grants.sql`, for the other five policies. Same reasoning for
-- why it exists at all (§4.1: a registry with 200 000 packages will not
-- enumerate them in TOML), same `node_kind` + `node_key` shape, same reason for
-- that shape — a `package` column that were NULL for version rows would make
-- every query say `WHERE (package = $1 OR package IS NULL)`, which is the
-- vacuous-predicate shape survey finding 2 came in on.
--
-- # One table, not one per policy
--
-- §6.3 is explicit: *"carrying every policy kind for the package and version
-- tiers — not a table per feature, and not one per tier."* The alternative
-- multiplies with the feature list rather than with the model: five policies
-- over two tiers is ten tables, nine of which are empty for any given
-- coordinate, and a resolver composing a node would join all of them to
-- discover that.
--
-- Composition walks a node at a time (`PolicyPath::resolve`), so what storage
-- owes it is *the node*, whole, in one read.
--
-- # Why the policies are JSONB and `visibility` is not
--
-- `visibility` is a scalar with four values, it is read on the hot path, and it
-- is the one policy here that narrows an audience — so it is a column with a
-- CHECK, greppable in a query plan and constrained by the database.
--
-- `versioning`, `quota` and `rules` are JSONB because they are *documents whose
-- shape is the config file's*. Columns for their fields would be a second
-- schema for `VersioningPolicy` and `QuotaConfig`, kept in step with the TOML
-- ones by hand, and a field added to one and forgotten in the other is a policy
-- an operator can write in a config file and not through the API. The rules
-- block is not even a fixed shape — it is one document per gate, keyed by the
-- gate's own name.
--
-- The cost is that the database cannot constrain their contents.
-- `StoredPolicy::validate` does, at the port, which is where §4.1's tier rules
-- are stated once: the naming half of `versioning` is rejected at version tier
-- (the name already exists), and `quota` stops at the package tier (a
-- per-version quota limits a thing published exactly once).
--
-- # There is no seal here either
--
-- Same as `grants`, and for a smaller reason: nothing in this table takes
-- access away. Every policy it stores constrains a *resource* rather than
-- granting a *subject*, and the one that narrows an audience —
-- `visibility = 'private'` — is a scalar with §4.3's administrative floor above
-- it.
CREATE TABLE IF NOT EXISTS policy (
    id                    BIGSERIAL PRIMARY KEY,
    registry              TEXT NOT NULL,
    -- 'package' or 'version', as in `grants`. A text discriminant with a CHECK
    -- rather than an enum type, so adding a tier is a code change that reaches
    -- the resolver rather than a migration that extends a type.
    node_kind             TEXT NOT NULL,
    -- package  →  the package name       e.g. '@acme/billing/cards'
    -- version  →  'package@version'      e.g. '@acme/billing/cards@1.4.2'
    node_key              TEXT NOT NULL,

    -- RFC 0015 §4.5's narrowing dimension. NULL means **inherit**, which is not
    -- the same as 'public': a row that stored the default rather than leaving it
    -- absent would be an override with a default value, and would stop the tier
    -- above from applying.
    visibility            TEXT,
    prerelease_visibility TEXT,

    -- Composes wholesale (§4.1): a deeper block replaces its parent's entirely
    -- rather than merging field by field, which is the only way "this one
    -- package follows a different release convention" is expressible.
    versioning            JSONB,
    quota                 JSONB,
    -- Composes **per gate**, unlike the two above — each gate is independently
    -- configured, and a wholesale override would force redeclaring `cve_gate`
    -- to change `release_age`, making a forgotten one a silently disabled gate.
    -- Stored as an array of `{gate, settings}` so the per-gate merge is a merge
    -- of rows rather than of keys.
    rules                 JSONB NOT NULL DEFAULT '[]'::jsonb,

    set_by                TEXT,
    set_at                TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT ck_policy_node_kind CHECK (node_kind IN ('package', 'version')),
    CONSTRAINT ck_policy_visibility
        CHECK (visibility IS NULL
               OR visibility IN ('public', 'internal', 'team', 'private')),
    CONSTRAINT ck_policy_prerelease_visibility
        CHECK (prerelease_visibility IS NULL
               OR prerelease_visibility IN ('public', 'internal', 'team', 'private')),
    -- One row per node. Unlike `grants` — where repeating a subject is a union
    -- in the model — a node has exactly one policy, and a second row would be a
    -- second answer to "what applies here" with no rule for choosing.
    CONSTRAINT uq_policy_node UNIQUE (registry, node_kind, node_key)
);

-- The resolution query: "what policy is written on the package and version
-- nodes for this coordinate". Both tiers in one index because both are fetched
-- in one call, for the same reason `idx_grants_lookup` gives — a second round
-- trip spends the p99 budget §11.7 fixed for resolution.
CREATE INDEX IF NOT EXISTS idx_policy_lookup
    ON policy (registry, node_key);
