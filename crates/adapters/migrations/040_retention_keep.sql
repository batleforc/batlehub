-- RFC 0016 §4.1 — the version-tier retention pin.
--
-- `retention_keep = TRUE` means a retention run never reclaims this version,
-- whatever the registry's policy says. It is the escape every automatic policy
-- needs: the release an LTS customer runs, which the pull statistics will
-- eventually stop defending.
--
-- **It is a keep, never a reclaim.** There is deliberately no column, and no
-- value of this one, that makes retention *more* aggressive for a single
-- version — a policy that deletes should not be reachable one version at a time.
--
-- Why here and not in RFC 0015's `policy` table, which §4.1 names as the home
-- for package- and version-tier retention: this is not a tiered policy at all.
-- It is a per-version boolean that sits beside `yanked`, `deprecated` and
-- `unlisted` — the three flags that already say "this particular version is
-- special" — and it is set through the same admin surface they are. The tiered
-- part of §4.1, where a namespace narrows a registry and a package narrows a
-- namespace, is what needs the policy table, and that is still phase 3 of
-- RFC 0015 rather than of this one.
ALTER TABLE local_packages ADD COLUMN IF NOT EXISTS retention_keep BOOLEAN NOT NULL DEFAULT FALSE;

-- A retention sweep asks "which versions of this package are pinned" per
-- package, and pins are rare. Partial, so an estate with none pays nothing.
CREATE INDEX IF NOT EXISTS idx_local_packages_retention_keep
    ON local_packages (registry, name)
    WHERE retention_keep;
