# Finishing RFC 0015's vocabulary — the six unrequested verbs

Not published (`srcExclude`d). Written 2026-08-29, after
`crates/web/tests/vocabulary_dead_ends.rs` turned §11.5's dead-end check from
prose into a gate.

> **Status: done, 2026-08-29.** Phases A, B, C1, C2 and C3 all landed; C4 is
> declined rather than deferred and §4.2 carries the argument. The dead-end list
> is down from six entries to one. What each phase found is written up in the
> RFC itself — §13.14 for A and B, §13.15 for C1 and C3, §13.16 for C2, the
> separator, and the finding that outranks all of them: **all three new verbs
> shipped unreachable**, because no §10 rule produces a verb no legacy config
> means, so every one of those endpoints answered `403` to the administrator
> while its tests passed by granting the verb explicitly. A single positive
> control caught it. This document is kept as the record of the plan, not as a
> live worklist.

## What this is about

Six of the vocabulary's 31 verbs are requested by no route. The test lists them
with reasons; this is the plan to empty that list.

**They are not one kind of work.** The test's own comment splits them and the
split is the whole planning problem:

| Verb | The action it gates | State |
| --- | --- | --- |
| `releases:list` | version documents, protocol indexes, search results | **implemented, ungated** |
| `catalogue:browse` | the console's explore and search surfaces | **implemented, gated by something else** |
| `npm:dist-tags:write` | moving a `dist-tag` | not implemented — and **declined on purpose** |
| `openvsx:namespace:claim` | claiming a publisher namespace | not implemented |
| `terraform:signing-keys:write` | registering a namespace's GPG key | not implemented |
| `jetbrains:channel:assign` | assigning a build to stable/EAP | not implemented |

The first two are **authorization gaps**: an operator can write the grant, the
server ignores it, and the thing it was meant to gate happens anyway. That is the
failure §11.5 exists to catch, and it is live today.

The last four are **absent features**. §4.2 introduces ecosystem verbs as the
vocabulary's extensible tail — *"an ecosystem-specific verb is added as a variant
like any other"* — and never commits to building the actions behind them. A verb
for an action that does not exist grants nothing because there is nothing to
grant. **Nobody is exposed by them**, and the day one of the features ships, the
compiler has its verb waiting.

So the order below is not the order of the table. It is: close the two holes,
then treat each feature as the small design question it turns out to be.

---

## Phase A — `catalogue:browse` (small, contained, do first)

**Why first.** Seven routes, and §10 rule 2's conjunction already reproduces the
legacy gate *exactly* — the translation was corrected against the §11.3 harness
in §13.5 and produced 19 disagreements before it was right. So the verb is known
to mean what the legacy sets mean, which makes this a substitution rather than a
new policy.

**The work.**

1. The seven `/api/v1/explore/*` routes request `catalogue:browse` through
   `require_verb`, scoped to the registry where they name one.
2. `hot_config::compute_access`'s three explore sets stop being the gate. They
   cannot simply be deleted: `explore_accessible_registries_for` returns a *set*
   that the listing uses to scope its query, not just a yes/no. So the set
   becomes "registries this caller holds `catalogue:browse` on", resolved from
   grants.
3. Delete `RbacConfig::explore`'s reader, keeping the field — §10 keeps
   `[registries.rbac]` accepted indefinitely.

**The decision it needs.** None. Rule 2 already settled it.

**The risk.** The set-not-boolean shape above. Getting it wrong scopes the
catalogue to the wrong registries, which is finding 2's blast radius. The
mitigation is that `explore.rs`'s existing "an empty accessible set is
**nothing**, not no restriction" guard stays exactly where it is.

**Tests.** Two per direction, in `explore.rs`: a caller who holds the verb on one
registry sees that registry's catalogue and no other; a caller who holds it
nowhere gets the empty document that `denied_everywhere` already returns.
Plus `no_stale_exceptions` deletes the entry for free.

**Size.** ~1 day. Behaviour-preserving for every translated config, which the
§11.3 harness can be extended to assert.

---

## Phase B — `releases:list` (the big one, and the one with a real argument)

**The problem is not the wiring, it is the inventory.** 76 sites pass
`Action::ReleasesRead` and some fraction of them are listings. §10 rule 4 exists
precisely because the split is not clean:

> Handlers pass `RELEASES_READ` for most listing documents — the npm packument,
> the NuGet flat index, Composer metadata — while the cargo sparse index goes out
> under `SOURCE_READ`. Both of today's verbs therefore authorise some listing.

So this cannot be a search-and-replace. It needs the same treatment §11.1 gave
the write surface: **an inventory, checked against the router in both
directions.**

**The work.**

1. Extend `authz_matrix.rs`'s route inventory with a third classification —
   `Listing` / `Artifact` / `Neither` — for every registered proxy route. The
   completeness gate that already fails on an unclassified route does the
   enforcement.
2. Every route classified `Listing` requests `releases:list`.
3. `authorize_listing` (4 call sites) takes the verb as its default rather than
   receiving `ReleasesRead` from the caller — that funnel exists *because* a
   listing names no version, so it is the natural home.

**The decision it needs.** What counts as a listing at the boundaries — a
package's own `/versions` document plainly does, a single version's metadata
plainly does not, and the ones in between (npm packument, NuGet registration
index, conda `repodata.json`) need the call made once and recorded, not made 76
times by whoever is editing.

**Why it is safe despite the size.** §10 rule 4 gives `releases:list` to anyone
holding `releases:read` **or** `source:read`, so no translated config loses a
document. The estates that change are the ones that wrote a grants block
distinguishing the two — which is the point of the verb.

**Tests.** The inventory gate is the main one. Plus one row in `authz_matrix.rs`
per direction: a caller with `releases:list` and not `releases:read` gets the
index and is refused the artifact; the inverse gets the artifact and an empty
index. That pair is §4.4's opening sentence, finally assertable end to end.

**Size.** ~3–4 days, most of it the inventory and its review.

---

## Phase C — the four ecosystem features

Each is a feature with a design question, and **none of them is blocked on
authorization** — the verb is already in the enum, already rejected on the wrong
registry type, and already grantable. Sequence them by whether the question is
answered.

### C1 — `terraform:signing-keys:write` (cleanest, do first)

The read side already has the slot: `eco_terraform.rs` emits
`"signing_keys": {"gpg_public_keys": []}` as a placeholder. Terraform verifies a
provider's SHASUMS signature against the keys served here, so an empty list means
**no client can verify a locally published provider** — this is a real feature
gap with a security consequence, not just a missing verb.

- **Store.** A `signing_keys` table keyed `(registry, namespace)` holding
  ASCII-armoured public keys and their key IDs.
- **Write.** `PUT /api/v1/admin/registries/{r}/namespaces/{ns}/signing-keys`,
  gated on `terraform:signing-keys:write`, scoped to the registry.
- **Read.** `eco_terraform.rs` fills the slot from the store.
- **Decision needed.** Whether a namespace with no key may publish a provider at
  all. Refusing is the coherent answer and is a behaviour change; allowing keeps
  today's behaviour and leaves the hole. **Argue it before building.**
- **Size.** ~2 days plus the decision.

### C2 — `openvsx:namespace:claim`

`vsx/api.rs` hardcodes `"verified": false` with a comment saying BatleHub has no
namespace-ownership model. It has one — `team_namespaces` — and the claim maps
onto it almost exactly: registry, prefix, owning group.

- **Blocker, and it is not small.** `team_namespaces` matching is hardcoded to
  `/` in `LOCAL_VISIBILITY_PREDICATE`'s SQL (`prefix || '/'`) and in
  `find_namespace`. OpenVSX namespaces are dotted (`publisher.extension`), and
  §4.1's separator table says so. **The separator has to become per-registry in
  the team-namespace store before this can be built**, and that predicate is the
  one §6.3 warns must agree with `check_visibility` character for character.
- **Decision needed.** Whether a claim is self-service (any authenticated caller
  claims an unclaimed namespace, first-come) or administrative. §4.3's delegation
  bounds argue for the second; the OpenVSX protocol assumes the first.
- **Size.** ~2 days for the claim, preceded by the separator work (~2 days), and
  that work touches a security-critical predicate — so it wants its own review
  and its own red-checked tests rather than riding along.

### C3 — `jetbrains:channel:assign`

Channel is read from `index_metadata["channel"]` at publish
(`eco_jetbrains.rs:47`), so it is set once and never moved.

- **Decision needed, and it is the interesting one.** Assigning a build to a
  channel post-publish means **mutating `index_metadata` on a published
  version**, which collides directly with §4.5's `immutable`. Is a channel move a
  "replacement"? §13.6 already settled that immutability is a question about
  *bytes* rather than a coordinate — which suggests a channel move is **not** a
  replacement, since no byte changes. That reading needs stating in the RFC
  before it is implemented, not after.
- **Work, once decided.** A `PUT …/plugins/{id}/{version}/channel` gated on the
  verb, writing one `index_metadata` field; the read path already selects on
  `channel`.
- **Size.** ~1 day after the decision. The decision is the work.

### C4 — `npm:dist-tags:write` (last, and possibly never)

**The 501 is a considered refusal, not a stub.** `cli.rs` explains it: dist-tags
are *derived* from the published version set and recomputed on every read, so
RFC 0006's block-repair moves `latest` the instant a version is blocked. A stored
tag would be overwritten by the next request, and `npm dist-tag ls` would report
something the client never set.

- **Decision needed.** Introducing stored tags means deciding what happens when a
  stored tag points at a version that is later blocked, yanked or deleted. The
  options are: repair it silently (which is what derivation does, and makes the
  stored value a lie), serve the blocked version (which defeats RFC 0006), or
  refuse the read (which breaks `npm install`). **None of these is obviously
  right, which is why the current answer is to decline the write.**
- **Recommendation.** Leave it. Record the reasoning in §4.2 beside the verb, and
  keep the exception entry — an exception with a good reason is a better outcome
  than a feature built to empty a list. If a real estate asks for it, it is an
  RFC of its own, not a phase of this one.
- **Size.** ~3 days if the decision goes the other way; the decision is most of
  the cost.

---

## Suggested order and what it costs

| | Item | Kind | Size | Blocked on |
| 1 | **A — `catalogue:browse`** | authorization gap | ~1d | nothing |
| 2 | **B — `releases:list`** | authorization gap | ~3–4d | an inventory decision |
| 3 | **C1 — terraform signing keys** | feature + security gap | ~2d | one decision |
| 4 | **C3 — jetbrains channel** | feature | ~1d | one RFC decision |
| 5 | **C2 — openvsx namespace** | feature | ~2d + ~2d | the team-namespace separator |
| — | **C4 — npm dist-tags** | recommend not doing | — | a decision with no good answer |

**Phases A and B are the ones that matter.** They close live gaps where a grant an
operator writes does nothing. Everything in C is a feature this server does not
have, and shipping the verb before the feature is the order §4.2 describes rather
than a debt it left.

## What to update when each lands

- `DELIBERATELY_UNREQUESTED` in `vocabulary_dead_ends.rs` — `no_stale_exceptions`
  fails until the entry is deleted, so this cannot be forgotten.
- §13.13's list of what the dead-end test says about the vocabulary.
- The RFC's status header, which currently names all six.
