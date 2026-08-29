-- RFC 0015 §10 rule 9 — ownership rows become package-tier grants.
--
-- > Ownership rows migrate to package-level grants — `releases:publish`,
-- > `owners:read` and `owners:write` on the one package, which is the scope
-- > `OwnershipPort` already has. Registry-wide `owners:write` is rule 5's admin
-- > grant and nothing else; a publisher does not acquire it by publishing.
--
-- §5.1 calls this "the largest simplification": a crate owner *is* a subject
-- holding `releases:publish` and `owners:write` on one package, so the cargo
-- owners API becomes a view over grants rather than a second store.
--
-- # `principal_type` maps to a subject form, and the mapping is not symmetric
--
--   user   →  'user:<id>'
--   group  →  'group::<name>' or 'group:<provider>:<name>'
--
-- A group principal is stored exactly as `is_permitted_by_group` compares it:
-- the identity's group string, which may or may not carry a `provider:` prefix.
-- RFC 0015 §13.5 records why that matters — a bare `eng` and a prefixed
-- `oidc1:eng` are different groups today, and reading a bare one as
-- `group:*:eng` would widen every deployment that uses the bare form. So the
-- mapping preserves the shape rather than normalising it: a principal
-- containing ':' becomes `group:<before>:<after>`, one without becomes
-- `group::<name>` — the RFC 0015 §4.3 spelling for a group with no provider.
--
-- # What is deliberately *not* migrated
--
-- **An unowned package gets no row.** §7: "Ownership migration must not convert
-- 'no owner rows' into 'everyone'. The survey's finding 1 was exactly that
-- reading; the migration writes no grant for an unowned package, and no grant
-- denies." This statement only inserts what exists, so a package with no owners
-- ends with no package-tier grants — which is *absence*, not a grant to
-- everyone.
--
-- **`role` is dropped.** `package_owners.role` is 'admin' or 'maintainer', and
-- nothing in the tree reads it: `can_publish` checks for a row, not for a role.
-- Carrying it into the grant set would invent a distinction the product does not
-- make, and the vocabulary has no verb for it. If one is wanted later it is a
-- new verb, granted deliberately, rather than a column resurrected.
--
-- # Idempotent
--
-- `ON CONFLICT DO NOTHING` on the node/subject key, so a re-run adds nothing and
-- — importantly — does not *overwrite* a grant an operator has since edited
-- through the admin API. A migration that clobbered a later decision would be
-- worse than one that skipped it.
INSERT INTO grants (registry, node_kind, node_key, subject, actions, granted_by)
SELECT
    o.registry,
    'package',
    o.package_name,
    CASE
        WHEN o.principal_type = 'user' THEN 'user:' || o.principal_id
        WHEN POSITION(':' IN o.principal_id) > 0 THEN 'group:' || o.principal_id
        ELSE 'group::' || o.principal_id
    END,
    ARRAY['releases:publish', 'owners:read', 'owners:write'],
    o.granted_by
FROM package_owners o
ON CONFLICT (registry, node_kind, node_key, subject) DO NOTHING;
