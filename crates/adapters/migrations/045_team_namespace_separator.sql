-- RFC 0015 §4.1 — a team-namespace claim records the character it matches on.
--
-- Every matcher in this tree hardcoded `/`: `find_namespace`'s SQL, its
-- in-memory twin, and `LOCAL_VISIBILITY_PREDICATE`. §4.1 carries RFC 0011-bis
-- §4.2's separator table over unchanged and makes it the definition of
-- "namespace" for every ecosystem — `.` for OpenVSX publishers and NuGet ids,
-- `:` for Maven groupIds — so on those a claim on `digital` covered `digital`
-- and nothing else. That is 0011-bis's bug from the other side: there a prefix
-- matched too much, here it matched too little.
--
-- # Stored on the claim, not derived per query
--
-- `LOCAL_VISIBILITY_PREDICATE` runs across many registries in one statement, so
-- deriving the separator from each registry's ecosystem would mean threading a
-- parallel `(registry, separator)` array into SQL and joining it. §6.3 requires
-- that predicate to agree with `check_visibility` **character for character**,
-- and the cheapest way to guarantee that across three implementations is for all
-- three to read one column.
--
-- # The default is what every existing row already matched
--
-- `'/'`, so no claim changes meaning on upgrade (§10). A claim made on a dotted
-- ecosystem before this column existed keeps its old, narrower matching until it
-- is re-claimed — the conservative direction, and not a regression, because it is
-- what the row already did.
ALTER TABLE team_namespaces
    ADD COLUMN IF NOT EXISTS separator TEXT NOT NULL DEFAULT '/';

-- One character. A multi-character "separator" would make `SUBSTRING(n, 1,
-- LENGTH(p) + 1)` compare the wrong slice, and the failure would be a matcher
-- that silently covers the wrong packages rather than an error.
ALTER TABLE team_namespaces
    DROP CONSTRAINT IF EXISTS ck_team_namespace_separator;
ALTER TABLE team_namespaces
    ADD CONSTRAINT ck_team_namespace_separator CHECK (length(separator) = 1);
