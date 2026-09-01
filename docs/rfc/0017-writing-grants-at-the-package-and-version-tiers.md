# RFC 0017 — Writing grants at the package and version tiers

| Field       | Value                                                        |
| ----------- | ------------------------------------------------------------ |
| Status      | Draft                                                         |
| Short       | Grants editor                                                 |
| Settles     | Who writes a package- or version-tier grant, and what §4.4 filters once they exist |
| Author      | Max Batleforc <maxleriche.60@gmail.com>                       |
| Co-author   | —                                                             |
| Created     | 2026-09-01                                                    |
| Supersedes  | —                                                             |
| Touches     | `crates/core`, `crates/adapters`, `crates/web`, `cli`, `ui`, docs |

---

## 1. Summary

RFC 0015 built the two deepest tiers of the grant hierarchy and left them without
an editor. The `grants` table has a `node_kind` of `package` or `version`
(migration 041), `GrantRepository` can write either, and `chain::stored_nodes`
reads both on every authorization — but the only code in the tree that ever calls
`put_grant` is the ownership projection, which writes **package** rows carrying
exactly three verbs. No caller has ever written a **version** row.

This RFC adds the missing writer: an admin API, CLI and console surface for
grants on a package or a version, and the per-version listing filter that becomes
load-bearing the moment a version row can exist.

The second half is not optional decoration on the first. RFC 0015 §4.4 rule 2
says a caller holding the read on a package but not on every version gets *the
filtered list*, and `filter_listing`/`package_visibility` were written for it and
have never been called — because with no version row to differ from the package
answer there has never been anything to filter. Shipping the writer without the
filter turns two functions that are correct and idle into two functions that are
correct and bypassed.

### Before / after

```text
# today
$ batlehub admin grants set npm @acme/billing --subject group:oidc1:eng \
      --actions releases:read
error: unknown subcommand `grants`

  — a package-tier grant is reachable only as a side effect of
    `batlehub admin owner add`, which writes releases:publish + owners:read +
    owners:write and nothing else. A version-tier grant is unreachable.

# with this RFC
$ batlehub admin grants set npm @acme/billing --subject group:oidc1:eng \
      --actions releases:read,releases:list
$ batlehub admin grants set npm @acme/billing@2.4.0-rc.1 \
      --subject group:oidc1:release-managers --actions releases:read
$ batlehub admin grants list npm @acme/billing
  node                     subject                          actions
  package:@acme/billing    group:oidc1:eng                  releases:read, releases:list
  package:@acme/billing    user:alice                       releases:publish, owners:read, owners:write  (from ownership)
  version:@acme/billing@2.4.0-rc.1  group:oidc1:release-managers  releases:read
```

An `eng` caller listing `@acme/billing` now receives every version **except**
`2.4.0-rc.1`, rather than a version index naming a release candidate they cannot
download.

---

## 2. Motivation

1. **The version tier is built, read on the hot path, and unwritable.** Migration
   041 constrains `node_kind IN ('package','version')`, `ports::version_node_key`
   formats `name@version`, and `chain::stored_nodes` passes the version into
   `grants_for` on every `authorize`. Every one of those runs today against rows
   that cannot exist. A tier the resolver walks and nothing can populate is worse
   than an absent feature: it is a cost paid on every request for a capability
   nobody has.

2. **The package tier has a writer that can express three verbs.** The ownership
   projection (`services/ownership_grants.rs`) puts `releases:publish`,
   `owners:read` and `owners:write`. There is no way to grant `releases:read` on
   one package to one group — the motivating example of RFC 0015 §4.4's own
   opening sentence. The projection code already anticipates the writer that does
   not exist: `project_remove` keeps verbs it did not write, commented *"Something
   else wrote verbs for this subject on this package. Losing an owner is not a
   reason to lose those."* Nothing else writes them.

3. **§4.4 rule 2's second half has never been reachable, and will become
   reachable silently.** `filter_listing` and `package_visibility` have no
   caller — audited 2026-09-01. Today that is correct rather than a hole: with no
   version row, a caller's `releases:read` verdict is uniform across every
   version, so the package-tier decision is the whole answer. The first
   version-tier grant changes that with no other code change and no error: index
   documents keep listing versions the caller may not read. The download gate
   still refuses them one at a time, so what leaks is the existence and the
   numbers, not the bytes — inside a package the caller is already allowed to
   list. Bounded, and precisely what rule 2 decided against.

4. **RFC 0015 deferred the editor to "phase 4", and phase 4 shipped without
   it.** §12's decision log records: *"No grants editor before phase 4. …
   config-file-first is right for the registry and namespace tiers because those
   grants are reviewable and diffable; the editor is only needed for the package
   and version tiers, which do not exist until phase 4."* Those tiers now exist.
   RFC 0015 is marked Implemented, so the deferral has no document left to sit
   in.

---

## 3. Goals / non-goals

**Goals**

- An operator can grant a subject any verb on one package or one version, and
  take it away, without editing the config file or restarting.
- A version index returns the versions the caller may read, and a count and
  pagination computed on that set — RFC 0015 §4.4 rules 1 and 2.
- A filtered listing is never served from a cache key that does not name the
  grant set (§4.4 rule 3).
- The ownership projection and the new editor write the same rows without
  fighting: adding an owner and granting a verb are two writers on one table.

**Non-goals**

- **Grants at the registry or namespace tier stay in the config file.** §4.3's
  argument holds and this RFC does not reopen it: those grants are broad, and
  broad authorization belongs in something reviewable and diffable. The editor is
  for the two tiers a config file cannot reasonably enumerate.
- **No sealing from the API.** An empty action set is what a *seal* is (§4.3),
  `ck_grants_actions_non_empty` refuses one, and a seal is a config-file
  statement. Removing every verb deletes the row, exactly as `project_remove`
  already does.
- **No new verbs.** This is a writer for the vocabulary RFC 0015 §4.2 defines.
- **No expansion at evaluation time.** `releases:*` expands at write, as §4.2
  requires; the stored row carries the expanded set. That is already what
  `StoredGrant::actions` documents.
- **No UI for the version tier in phase 1.** The console gets the package tier
  first; a per-version grants panel is worth its own review once the API exists.

---

## 4. User-facing design

### 4.1 API

```text
GET    /api/v1/admin/registries/{registry}/grants?package=<name>[&version=<v>]
PUT    /api/v1/admin/registries/{registry}/grants
DELETE /api/v1/admin/registries/{registry}/grants
```

`PUT` body:

```json
{
  "package": "@acme/billing",
  "version": "2.4.0-rc.1",
  "subject": "group:oidc1:release-managers",
  "actions": ["releases:read"]
}
```

`version` absent addresses the package node; present, the version node. The verb
is `grants:write` for the mutations and `grants:read` for the listing — new
entries in RFC 0015 §4.2's control vocabulary, held at the instance tier by
`role:admin` under §10 rule 5, and delegable per registry like every other
control verb.

### 4.2 CLI

```sh
batlehub admin grants list npm @acme/billing
batlehub admin grants set  npm @acme/billing --subject group:oidc1:eng --actions releases:read,releases:list
batlehub admin grants set  npm @acme/billing@2.4.0-rc.1 --subject user:alice --actions releases:read
batlehub admin grants rm   npm @acme/billing --subject group:oidc1:eng
```

`name@version` in the positional argument selects the version node, matching
`version_node_key`'s own spelling so the CLI and the storage key read the same.

### 4.3 Behaviour rules

- **A write replaces that subject's row on that node**, never the node's other
  rows. `put_grant` already documents why: two rows for one subject would make
  the union depend on read order.
- **Removing every verb deletes the row.** There is no empty-set row to write.
- **Ownership verbs are not editable through this surface.** A `PUT` that would
  drop `releases:publish`, `owners:read` or `owners:write` from a subject that
  holds them by ownership is refused with `409`, naming `admin owner rm` as the
  way to do it. Two writers on one row is fine; two writers on one *verb* is a
  race with no winner, and the ownership projection would silently restore what
  the editor removed on the next owner change.
- **Grants only widen** (§4.3). Nothing written here can take away what a broader
  tier granted; a "deny" is not expressible and is not becoming expressible.

### 4.4 Validation

`PUT` rejects:

| Condition | Rationale |
| --- | --- |
| Unknown action name | An unrecognised verb silently granting nothing is §13.17's shape: a config that looks applied and is not |
| Unparseable subject spelling | Same; `SubjectMatcher` already has one parser and this uses it |
| Empty `actions` | A seal is a config-file statement (§4.3), and the table constraint refuses it anyway |
| `version` present and the package has no such version | A grant on a coordinate that does not exist is a typo more often than a plan, and the row would resolve for nobody |
| A verb held through ownership | See §4.3 |

Warnings (surfaced on the response and in `explain`):

| Condition | Behaviour |
| --- | --- |
| The subject already holds the verb from a broader tier | Written, and reported as redundant. Grants union, so this is legal and inert — and an operator who wrote it believed it did something |
| A version-tier grant on a yanked or deleted version | Written, and reported. The coordinate is spent; the row will resolve for nothing |

---

## 5. Architecture

### 5.1 Where the write lands, and what already reads it

```mermaid
flowchart TD
    API["PUT /admin/registries/{r}/grants"] --> SVC["GrantAdminService"]
    OWN["admin owner add"] --> PROJ["ownership_grants projection"]
    SVC --> REPO["GrantRepository::put_grant"]
    PROJ --> REPO
    REPO --> TBL[("grants<br/>node_kind = package #124; version")]
    TBL --> READ["chain::stored_nodes<br/>(already live)"]
    READ --> RES["resolve #40;path, subject#41;"]
    RES --> ONE["authorize_read — one coordinate"]
    RES --> MANY["Readable — whole-registry documents"]
    RES --> VER["filter_listing — one package's versions<br/>(built, idle, this RFC wires it)"]
```

The invariant this protects: **there is one writer path and one reader path.**
The editor does not get its own table, its own cache or its own resolution — it
writes the rows the resolver already walks, which is why the authorization half
of this RFC is small. What is new is a writer and one filter.

### 5.2 Where the version filter goes

The package-level half of §4.4 is enforced by `Readable`
(`services/authz/filter.rs`), consulted by `local_registry/read.rs` for every
whole-registry document. The version-level half belongs one layer in, at
`LocalRegistryService::load_visible_versions` — the single funnel every version
index already passes through, beside `filter_unlisted`, `filter_blocked` and
`filter_for_identity`.

That placement is the whole of rule 1's compliance for local documents: the
funnel returns the filtered `Vec<PublishedPackage>`, and every protocol handler
counts and pages what the funnel returned. Nothing downstream can compute a total
over rows the caller may not see, because nothing downstream ever holds them.

```mermaid
flowchart LR
    GV["backend.get_versions"] --> UL["filter_unlisted"]
    UL --> BL["filter_blocked #40;RFC 0006#41;"]
    BL --> ID["filter_for_identity<br/>#40;beta channel#41;"]
    ID --> GR["filter_by_grants<br/>#40;this RFC#41;"]
    GR --> DOC["the protocol document,<br/>counted and paged here"]
```

---

## 6. Detailed design

### 6.1 `crates/core`

- `services/grants_admin.rs` (new) — `GrantAdminService`, the write funnel:
  validation, the ownership-verb refusal, expansion of `releases:*`, and the
  redundancy warning. A service rather than handler code, so the CLI and the API
  cannot disagree about what a legal grant is.
- `services/local_registry/read.rs` — `filter_by_grants`, called from
  `load_visible_versions` after `filter_for_identity`. Resolves once per package
  and asks per version; the rows for both tiers arrive in the `grants_for` call
  `stored_nodes` already makes.
- `services/authz/filter.rs` — `filter_listing` and `package_visibility` acquire
  their caller. The module header's "waiting for a writer" note is deleted in the
  same commit, because a note that outlives its subject is the next reader's
  wrong turn.

### 6.2 `crates/adapters`

- Nothing. `PgGrantRepository` implements every method this needs, and migration
  041's constraints already refuse an empty action set and a duplicate
  `(registry, node_kind, node_key, subject)`.

### 6.3 `crates/web`

- `handlers/back_office/governance/grants.rs` (new) — the three routes, guarded
  by `require_verb(GrantsWrite | GrantsRead)`.
- `handlers/back_office/authz_explain.rs` — no change needed; `explain` already
  reports package- and version-tier provenance because it resolves through the
  same path.

### 6.4 `cli`

- `cli/src/cli/admin.rs` — `admin grants list|set|rm`.

**Deliberately untouched**, so reviewers do not go looking:

- `server/src/grants.rs` — builds the registry and namespace tiers from config.
  Those tiers are a non-goal here.
- `services/ownership_grants.rs` — the projection stays exactly as it is. It is a
  second writer on the same table by design, and §4.3's `409` is what keeps the
  two from overwriting each other.
- The proxy read path — a proxied version document is filtered by
  `ProxyService::version_document` for blocks; grants at the version tier apply
  to locally published versions, and a proxied coordinate has no local row to
  hold a grant against.

---

## 7. Security considerations

- **The new surface is authenticated and admin-gated.** `grants:write` is a
  control verb, held at the instance tier by `role:admin` under §10 rule 5, and
  delegable per registry exactly like `cache:evict` or `audit:purge`.
- **A grant cannot narrow.** Everything writable here widens (§4.3), so the worst
  a wrong write does is grant access, not silently revoke it. That is the
  direction an audit trail can catch after the fact, which is why every mutation
  records one.
- **The filter closes a disclosure rather than opening one.** Before this RFC a
  version-tier grant is unwritable, so no listing under-filters. The filter and
  the writer must land in the same release: the writer alone is the disclosure
  described in §2.3.
- **Rule 3 is a hard requirement, not a follow-up.** Any cache in front of a
  filtered listing is keyed by `GrantSet::cache_key` or it is not cached. Survey
  finding 11 is the precedent already paid for: the search cache held merged
  local hits under an identity-blind key and replayed one caller's private
  results to the next.
- **Every mutation is audited.** New `AccessAction` variants `GrantWrite` and
  `GrantRevoke`, carrying the coordinate and the subject — the question after an
  incident is "who could read this and since when", and a grant write is the only
  event that answers it.

---

## 8. Alternatives considered

| Alternative | Why rejected |
| --- | --- |
| Config-file grants at the package and version tiers | §4.3 already rejected it for these tiers, and the estate sizes §11.7 measures make it absurd: a config file enumerating grants for 200 000 packages is not reviewable, which was the entire argument for keeping the broad tiers in config |
| Extend the ownership API instead of a new surface | Ownership means three specific verbs on one package. Widening it to arbitrary verbs makes "owner" mean nothing, and the cargo owners API is a view over it (§5.1) — it would start reporting subjects that are not owners |
| Ship the writer now, the filter later | §2.3: the gap is silent, and the release between them is a release that lists versions the caller may not read. The filter is idle code today and becomes the thing that makes the writer safe |
| Filter in the handlers rather than in the funnel | 76 call sites, each free to compute a total before filtering — rule 1's failure mode exactly. The funnel is the reason `load_visible_versions` exists |
| Deny-grants at the version tier instead of filtering | A deny in a model that only unions is a second composition rule pointing the other way, and §4.5 gives the account of why visibility (which narrows) and grants (which widen) stay separate mechanisms |

---

## 9. Rollout and compatibility

- **Default behaviour when nothing is configured**: unchanged. No version-tier
  row exists in any estate today, so the filter removes nothing until an operator
  writes the first grant.
- **Config migration**: none. `CURRENT_CONFIG_VERSION` does not move — this adds
  no config key.
- **Schema migration**: none. Migration 041's table already carries both tiers.
- **Operator prerequisites**: none beyond an admin token.
- **Rollback**: the rows persist. Reverting the code leaves version-tier rows in
  the table that the resolver still reads and no filter acts on — which is the
  §2.3 disclosure. A rollback that matters therefore deletes
  `WHERE node_kind = 'version'`, and the release note has to say so.

---

## 10. Test plan

- **Unit** (`crates/core/src/services/grants_admin.rs`): expansion of
  `releases:*` at write; the ownership-verb `409`; empty-actions refusal; unknown
  subject and unknown action refusals; the redundancy warning fires on a
  broader-tier duplicate and not otherwise.
- **Unit** (`crates/core/src/services/authz/filter.rs`): `filter_listing` and
  `package_visibility` keep their existing tests and gain a caller — the tests
  that have been asserting an idle function start guarding a live one.
- **Unit** (`crates/core/src/services/local_registry/read.rs`): a version-tier
  grant on one version of three yields one version to a caller whose package
  grant does not cover it; no version-tier row yields all three, byte for byte
  what the funnel returns today.
- **Integration** (`crates/web/tests/authz_matrix.rs`): the matrix gains a
  version-tier row. This is the strongest signal in the plan — §13.17 records
  that a test granting its own verb cannot tell a correct denial from a denial
  for the wrong reason, and the matrix is what caught 44 such routes.
- **Integration** (`crates/web/tests/grants_*.rs`): the three routes, the verb
  gate, and `explain` reporting a version-tier grant's provenance.
- **Integration**: a filtered version index served twice to two callers with
  different grant sets returns two different documents — rule 3, asserted rather
  than assumed.
- **Existing suites that must pass unchanged**: `authz_matrix.rs` on every route
  that has no version-tier grant (the filter must be inert when the table is
  empty), `local_npm_registry.rs` and its siblings for the untouched protocol
  documents, and `pg_grants.rs` for the storage layer this adds nothing to.

---

## 11. Decisions and open questions

### Resolved

| # | Question | Decision |
| --- | --- | --- |
| 1 | Config file or API for these tiers? | **API.** §4.3 settled it for the broad tiers and gave the reason the deep ones differ: a config file cannot enumerate 200 000 packages, and these grants are written per package by people who are not editing the deployment |
| 2 | Does the ownership projection stay? | **Yes, unchanged.** It is the cargo owners API's storage (§5.1). Two writers on one table is the design; the `409` on ownership verbs is what keeps them from fighting over one row |
| 3 | Writer and filter in one release? | **Yes.** The gap between them is a release that under-filters, silently (§2.3) |

### Still open

1. **Does the version filter apply to proxied documents?** A version-tier grant
   names a local coordinate, and `ProxyService::version_document` filters
   upstream documents for blocks. A hybrid registry can hold a local `1.2.0`
   beside an upstream `1.2.1`; a grant on the local one is meaningful and a grant
   on the upstream one has no row to hang from. Recommendation: local versions
   only in phase 1, stated in the docs rather than left to be discovered, because
   the alternative is a grant that appears to apply and does not.

2. **Should `grants:read` be separate from `audit:read`?** Both answer "who could
   see this". Recommendation: separate, matching the `audit:read`/`audit:purge`
   split — reading who holds a verb is not reading what they did with it, and one
   of the two is a much larger disclosure.

3. **Does the console get the version tier in phase 3, or later?** The package
   tier is a table of subjects; the version tier is that table per version, and a
   package with 400 versions makes it a different design problem. Recommendation:
   defer, and let the CLI carry the version tier until someone asks for the panel.

---

## 12. Implementation phases

| Phase | Content |
| --- | --- |
| 1 | `grants:read` / `grants:write` in the vocabulary, `GrantAdminService`, the three routes, the audit actions. Useful alone: it makes the package tier writable with the full verb set, which is §2.2 and needs no filter |
| 2 | `filter_by_grants` in `load_visible_versions`, wiring `filter_listing` and `package_visibility`. **Must ship no later than the release that allows a version-tier write** — phase 1 refuses `node_kind = version` until this lands, which is the interlock rather than a note in a changelog |
| 3 | CLI `admin grants list\|set\|rm`, then the console's package-tier panel |
