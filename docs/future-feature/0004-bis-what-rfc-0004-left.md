# RFC 0004-bis — What RFC 0004 left, and the gates that could not see it

| Field       | Value                                                                 |
| ----------- | --------------------------------------------------------------------- |
| Status      | **Implemented** — all six phases landed; see the implementation notes in §14 |
| Author      | Max Batleforc <maxleriche.60@gmail.com>                                |
| Co-author   | Claude Opus 5 (1M context) <noreply@anthropic.com>                     |
| Created     | 2026-08-13                                                            |
| Supersedes  | —                                                                     |
| Complements | RFC 0004 (admin composition and the API surface) — carries its residue |
| Touches     | `crates/core`, `crates/web`, `ui`, `ui/build`, `DESIGN.md`, docs       |

---

## 1. Summary

RFC 0004 shipped in five phases and did what it said. This RFC is what it
*discovered* and deliberately did not do — plus one category it could not have
predicted, because the pass found it by looking rather than by planning.

Five kinds of residue, and they are not equal:

1. **Three gates report success over conditions they cannot observe.** The i18n
   audit reads `0 untranslated strings` while five English strings render to a
   French operator. The catalogue gate proves every key is translated and never
   asks whether any key is *used* — 94 of 710 appear nowhere in `src/`, and the
   `adminIpBlocks.*` family among them was orphaned by RFC 0004's own merge with
   nothing failing. Eleven of fifteen admin pages still have no component test.
2. **Seven API gaps the UI is currently papering over**, one of which is a
   correctness defect: `POST /api/v1/admin/access-check` answers `allow` for an
   account the adjacent page shows as blocked.
3. **Composition work still owed** — the five Phase 5 items whose verdicts were
   argued but only partly discharged, plus one pattern no per-page reviewer could
   have seen: fourteen fields ask an operator to type an identifier the server
   already knows, and the substring filter that would suggest it has shipped in
   three endpoints since the repository port was written. The one field that
   already suggests reaches upstream uncached on every keystroke, and caches its
   results in a store that survives logout.
4. **`DESIGN.md` findings**, recorded under RFC 0004 §3/R4 and untouched since,
   including one the console has now improvised twice.
5. **`/packages` has diverged from the design proof that was built to prove it**,
   and no gate looks at it. The proof spends the system's loudest element on the
   registry you are looking at; the shipped page spends a much smaller step on
   the word "Packages", and the proof's organising idea — resolution as state —
   exists nowhere in `ui/src`.

The through-line is the first category. RFC 0004 §2.2's argument was that the
console cannot show what the API does not describe; this RFC's is narrower and
sharper: **a gate that cannot observe a condition reports the same green as a
gate that observed it and found nothing**, and four of ours currently do.

§2.7 is the sharpest instance: the console has a checked-in artefact stating
exactly what one page should look like, the page does not look like it, and the
check that would have said so does not run on that route.

**One item should not wait for its phase.** The explore cache (§2.9) is a
module-level map holding viewer-scoped results that `logout()` does not clear
and a client-side route change does not reload, so on a shared browser the next
person to sign in reads the previous viewer's rows for up to five minutes. It is
a small fix to a live confidentiality edge and it should ship ahead of this
schedule; everything else here can be taken in order.

---

## 2. Motivation

### 2.1 A green gate and a French operator reading English

`pnpm run i18n:check` reports `0 untranslated strings across 0 files`. Five
user-visible English strings ship regardless:

```
ui/src/pages/AdminHealth.vue:143                  empty-message="No registries configured."
ui/src/pages/AdminHealth.vue:435                  confirm-label="Clear Cache"
ui/src/pages/AdminHealth.vue:436                  loading-label="Clearing…"
ui/src/pages/AdminNotificationSubscriptions.vue:489-490   confirm-label="Delete" / loading-label="Deleting…"
```

`build/i18n-audit.mjs:50` scans six attributes — `title`, `placeholder`,
`aria-label`, `alt`, `label`, `description` — and component props are not among
them. So a destructive dialog renders « Vider le cache de cargo ? » above a
button reading **Clear Cache**, and the gate is satisfied.

The same blind spot covers string literals assigned to refs:

```
ui/src/pages/AdminBulk.vue:63     parseError.value = "Paste some CSV content first."
ui/src/pages/LoginPage.vue:69     error.value = "Token is valid but grants only anonymous access."
```

This is not a new class of defect. `ui/src/config/adminSections.ts:1-13` exists
because the audit once read zero while the *entire admin navigation* rendered in
English — the labels were in `<script>` and the scan only read templates. The
file's own header records it. The lesson was learned narrowly: the scanner
gained one case rather than a rule about where human-readable text can live.

### 2.2 A gate that proves translation and never asks about use

`catalogues.test.ts` checks key-set parity, empty strings, placeholder
preservation, verbatim domain terms and relative length. It never checks whether
a key is *referenced*. Two consequences, both already realised:

- **`dashboard.allAnswering`, `dashboard.someDegraded` and `dashboard.healthUnknown`
  were translated, correct, and referenced zero times** while `AdminDashboard`
  hardcoded three English sentences. A French operator's 3am alarm arrived in
  English, with the correct French sitting in the catalogue. RFC 0004 Phase 5
  fixed the page; nothing stops it recurring.
- **94 of 710 keys appear nowhere in `src/`.** The `adminIpBlocks.*` family is
  in that set because RFC 0004 Phase 5 merged that page away — and no gate
  noticed. The 21 `adminExploreCache.*` keys were deleted only because the
  author remembered to.

A parity gate answers "is every key translated". Nobody has been asking "is
every key needed", and the answer has been drifting for at least two RFCs.

### 2.3 Eleven of fifteen admin pages have no test

RFC 0004 §4.4 named "the page's own tests as the regression signal" for the
Impeccable pass. For eleven pages that signal did not exist, so the pass had
route-matrix coverage and axe and nothing else:

```
AdminAccessCheck  AdminBetaChannel  AdminConfigReload  AdminDashboard
AdminHealth       AdminInboundEvents  AdminNotificationSubscriptions
AdminPackages     AdminSbom         AdminTeamNamespaces  AdminWarming
```

Phase 5 added tests for the four pages it changed most invasively, and every one
of those tests caught something: the audit-log envelope, the bulk silent
failure, the dashboard's false empty state, and — in a test written for a
different reason — a submit button that did not exist because `Dialog` has no
`footer` slot. That hit rate is the argument.

### 2.4 The tool that is confidently wrong about the page beside it

`crates/web/src/handlers/back_office/access_check.rs:105` calls
`evaluate_and_trace(&policy.rules, &ctx)` and nothing else. It never consults
`UserBlockRepository` or `IpBlockStore` — both of which reject in middleware
*before* any rule evaluation. So an admin who blocks `alice` on
`/admin/security/blocks`, then simulates `alice` on the next tab, is told
**allow**.

RFC 0004 Phase 5 made the UI state its bound rather than imply coverage it does
not have. That is honest, and it is not a fix: the page whose entire purpose is
"would this identity be allowed" gives an answer that the section it lives in
can contradict.

### 2.5 Composition the verdicts scoped and the phase did not reach

Phase 5 delivered fifteen verdicts and executed all fifteen, but three *update*
verdicts were only partly discharged. This is stated plainly rather than
absorbed, because a verdict recorded as done and a verdict done are different
things:

| Page | Verdict evidence | Executed | Outstanding |
| --- | --- | --- | --- |
| `AdminPackages` (708) | table ~1650px intrinsic in a 1134px container; row verbs off-screen at 1440 | form deleted, filters, confirmations, silent failures | the column drop |
| `AdminHealth` (488) | aggregate cache card restates `AdminDashboard`'s from the same `adminStats().aggregate`, minus the trend | delete-artifact control relocated in | remove the duplicated card |
| `AuditLog` | endpoint accepts `registry\|user_id\|from\|to\|denied_only\|page\|per_page`; page sends none | envelope fixed, export URL and filters | the query surface and a pager |

`AdminWarming` renders eleven identical cards with a JetBrains path placeholder
on cargo and npm, which is PRODUCT principle 5 ("registry types are data") going
unenforced.

### 2.6 One live region in the whole console

`ui/src/components/ui/announcer/` exists, is tested, and has **one** consumer.
Exactly one page under `ui/src/pages/` carries `role="status"`, `role="alert"`
or `aria-live`. Every bulk result, every block, every warm and every cache
invalidation is announced to sighted users only — on the surface whose audience
includes the operator who is the sole person able to perform destructive
actions.

---

### 2.7 A proof of what one page should be, and a page that is not it

`ui/design-proof/index.html` is checked in. Its surface brief names its scope in
the first line: *"the package catalog (`/packages`) — the proving surface for
the console redesign."* It is the artefact RFC 0003 Phase 1 produced to settle
what this world looks like, and `/packages` is the page it settled it on.

Measured side by side at 1440×900, dark rendition, both against real rows:

| | Proof | Shipped `/packages` |
| --- | --- | --- |
| Ground / ink | `oklch(0.07 0.018 18)` / `oklch(0.93 0.018 25)` | identical |
| Faces | JetBrains Mono + Silkscreen | identical |
| Copper in use | yes | yes |
| **Display element** | **`npm1`** — the registry being viewed, **104px** Silkscreen | `Packages` — the page's nav label, **24px** |
| **Ramp steps present** | 12, 13, 15, 16, 20, 24, **104** | 12, 13, 15, 24 |
| **Halftone plate** | present | **absent** |
| **Dot matrix** | present | **absent** |
| Page height | 1640px | 900px |

The tokens are not the problem: ground, ink, both faces and copper all match
exactly. What is missing is the proof's *idea*.

Two differences carry it. The first is what the Display step is spent on: the
proof makes the **registry you are looking at** the largest thing on the screen,
so the page announces its subject; the shipped page spends a much smaller step
on the word "Packages", which is a label on a door. The second is that
**"resolution as state" — DESIGN.md's organising idea, the fine 3×3 matrix for
what is held and verified against the coarse 2×2 for what is not — appears
nowhere in `ui/src`**, on this page or any other. It is the system's signature
and it has never been implemented. A Phase 5 reviewer reached the same
conclusion independently, from a different page.

**No gate looks at this route.** `/packages` is scanned by `ui:design:rendered`,
which runs `impeccable detect` and axe. The type-ramp and display-face check
lives in `build/design-authed.mjs`, whose route list is `/admin/*`, `/me/*` and
`/`. So the one page with a checked-in specification of its own appearance is
the one significant page no ramp check runs against — which is how a 104px
display element became a 24px one without anything failing.

This is not a Phase 5 regression. RFC 0004 §3 names "reworking the catalog or
package pages" as an explicit non-goal, and the proof predates that RFC
entirely. It is drift that nothing was watching for.

---

### 2.8 Fields that name a thing the server already knows

The console asks an operator to type identifiers it could offer. The same
concept is a closed list on one page and a free-text box on another:

| Concept | Offered as a set | Typed blind |
| --- | --- | --- |
| **registry** | `AdminBetaChannel:95`, `AdminTeamNamespaces:88`, `NamespaceUpload:217` — a `Select` over `listRegistries()` | `AdminSbom:101` (`"e.g. crates-io"`), `AccessCheck:72` (`"github"`), `AdminAccessCheck:97` (`"npm"`), `AdminNotificationSubscriptions:366` (`"e.g. my-cargo"`) |

A registry list is a handful of entries, fetched on nearly every page already.
Half the console makes you remember whether the instance calls it `crates-io`,
`cargo` or `my-cargo` — and the placeholder in each of those four fields guesses
a *different* convention, which is what a field looks like when nobody could
check their answer.

`AdminAccessCheck`'s is not even the `Input` primitive — it is a bare `<input>`
with hand-rolled `rounded border border-border bg-background px-3 py-1.5`
classes, so it also does not inherit whatever `Input` does next. Two of these
fields carry a second defect on top of the first.

For the rest, the suggestion source exists and is unused:

| Field | Source that already ships | Where it is typed blind |
| --- | --- | --- |
| package name | `name` on `/api/v1/packages`, `/api/v1/admin/packages`, `/api/v1/explore/packages` is **`name_contains`**, a substring filter, paginated | `AdminAccessCheck:113`, `AccessCheck:76`, `AdminNotificationSubscriptions:383`, `AdminWarming:163`, `DeleteCachedArtifact:125`, every row of `AdminBulk`'s CSV |
| package name, not yet cached | `GET /api/v1/explore/upstream?name=&registry=&limit=` — an upstream search, already bounded by `limit` | same |
| version | `PackageDetailResponse.versions` — once registry and name are known, the set is closed | `AccessCheck:80`, `AdminAccessCheck:123`, `NamespaceUpload:251`, `DeleteCachedArtifact` |
| namespace prefix | `GET /api/v1/admin/registries/{registry}/namespaces` | `AdminTeamNamespaces:203` |
| notification channel | `GET /api/v1/admin/notifications/channels` | already offered — Phase 5 turned it into chips |
| **subject / user id** | **nothing.** `/api/v1/admin/users/blocked` lists only the blocked | `AuditLog:136`, `AdminAccessCheck:168`, `AdminTeamNamespaces:219` (`"e.g. oidc:frontend-team"`), `AdminBetaChannel:216` |

The substring filter is the sharpest instance: `PackageFilter::name_contains`
has shipped since the repository port was written, three endpoints expose it,
and no field in the console uses it to suggest anything.

The subject field is the worst case, because the failure is silent. An operator
filtering the audit log for `alice` when the instance stores `oidc:alice` gets an
empty table — which reads exactly like "this user did nothing", on the surface
whose entire purpose is establishing what someone did. There is no endpoint that
would let the field offer the answer (A8).

**There is no primitive between `Input` and `Select`.** `ui/src/components/ui/`
has both and nothing in between, so the two places that needed one improvised
with `<datalist>` (`RegistryPathForm:46`, `AdminNotificationSubscriptions:421`)
— unstylable, inconsistently keyboard-navigable across browsers, and invisible
to the a11y gate as a listbox. That is §7 item 7's pattern again: a missing
shared component gets invented per page.

**Why no Phase 5 reviewer caught it.** The critiques were grouped by *section*,
deliberately — a *merge* verdict is invisible to a reviewer who can only see one
route. A field-level pattern is invisible the same way one level up: it crosses
every section, so no reviewer ever had the whole set in view. The grouping that
made merges visible made this class invisible, and that is a property of the
method, not an oversight by any of the reviewers.

### 2.9 A cache that outlives the identity it was filled for

`/packages` makes two calls per search. They are cached completely differently,
and neither is right for what §6.2 is about to do to them.

**`explorePackages` is cached correctly, and the store is not.**
`useExploreCache` keys on `registry::page::sort::query` with a 5-minute TTL,
`perPage` is a module constant so its absence from the key is sound, and
`PackageCatalog.vue:191` writes under the `reg/p/s/q` captured at call time
rather than the current refs — so a late response cannot land under the wrong
key. The entries are disciplined.

The store they live in is not. `_store` is a module-level `Map` in
`useExploreCache.ts`, and the server scopes explore results **by viewer on
purpose**: `explore_viewer_for` (`handlers/mod.rs:28`) carries `is_admin`,
`is_authenticated` and `groups`, and `list.rs:104` applies it so a caller cannot
see packages in a private namespace they are not in. The cache key contains none
of those three. `logout()` (`useAuth.ts:157`) clears the tokens and the identity
and never touches `_store`, and `handleLogout()` does `router.push("/login")` —
a client-side navigation, so nothing reloads and the map survives intact. On a
shared browser the next person to sign in reads the previous viewer's
group-scoped results for up to five minutes. `invalidate()` exists and has
exactly one call site: the refresh button at `PackageCatalog.vue:301`.

**`exploreUpstreamSearch` is cached nowhere.** `fetchUpstream:199` never touches
`exploreCache`. Behind it, `explore_upstream_search` (`stats.rs:203`) fans out
with `join_all` across every accessible registry client on every request, and
the adapters' `search_packages` goes straight to the network — `npm.rs` builds
`/-/v1/search?text=…` and calls `.send()` with no cache store, and the `cache`
references in those files belong to `resolve_metadata`. No `Cache-Control`
either; the only one in the whole web crate is `no-store` on the OIDC callback.

So typing `lodash` is five debounced requests, each fanning out to N upstream
registries — 5N third-party calls — and backspacing re-queries from scratch.

**Neither fetch guards its own ordering.** `packages.value = body.items:189` is
unconditional: no sequence token, no `AbortController`. `selectRegistry` and
`onSortChange` are undebounced, so clicking registry A (uncached, slow) then B
(cached, instant) lets A's response overwrite the table while the sidebar shows
B selected. The cache entries stay correct; the display does not.

**Why this is in this RFC rather than a bug report.** Today one search box does
this. §6.2 puts a suggesting field on fourteen more, every one of them hitting
`name_contains` or `explore/upstream` per keystroke. Whatever the cache does
wrong now gets multiplied by fourteen the moment the field sweep lands, which
makes it a prerequisite rather than an adjacent defect.

`PackageCatalog.vue` has no component test. `useExploreCache.test.ts` covers the
composable in isolation — TTL, key independence, `invalidate` — and never
exercises it against the page, which is why a store that survives logout passes
a suite specifically written for it.

---

## 3. Goals / non-goals

**Goals**

- No gate reports success over a condition it cannot observe. Each of the three
  either gains the observation or loses the claim.
- The access-check simulator is correct, or it does not answer.
- The six remaining API gaps are closed or explicitly declined, each with a row.
- The three partly-discharged Phase 5 verdicts are finished against their own
  recorded evidence.
- Every admin page has a component test that would fail if the page stopped
  answering its question.
- A field that names something the instance knows about offers it while you
  type, and the ones that cannot say so instead of failing silently.
- No cached response outlives the identity it was fetched for, and no field that
  suggests reaches a third-party registry once per keystroke.
- The ramp and display-face check runs on every rendered route, including the
  public ones — so a page drifting from its own specification fails rather than
  waits to be noticed by eye.

**Non-goals**

- **Re-opening the Phase 5 verdicts.** They were reached against rendered pages
  with the evidence §4.4 required. This RFC finishes them; it does not relitigate
  them.
- **The `DESIGN.md` migration.** §7 records the findings and proposes the order
  to take them in, but retiring `Card` across 29 files is its own RFC with its
  own gates, and mixing it into composition work makes a regression
  unattributable — the same argument RFC 0004 §8 made.
- ~~**New product surface.** Nothing here adds a page or a feature. Every item is
  a thing that already exists and is wrong, missing, or unobserved.~~
  **Amended.** §1–§12 hold to this: every item in them is a thing that exists and
  is wrong, missing, or unobserved. **§13 is an addendum that deliberately breaks
  it**, carrying four product gaps found by reading the tree against the roadmap
  rather than by reviewing a page. They are here because they were found while
  this RFC was open and nothing else was tracking them; the cost is that this
  document now covers two kinds of work, and §13 says which of its items are
  specified-only so the distinction survives.
- **Re-cutting `/packages` to match the proof.** §4.4 makes the divergence
  *visible*; closing it is a design task, not a patch. The proof was produced by
  Impeccable in RFC 0003 Phase 1, and implementing "resolution as state" is a
  world-level decision about a component every surface will inherit. Retrofitting
  it by hand from a screenshot is how a specimen becomes a pastiche. See O3.

---

## 4. The gates

### 4.1 The i18n audit learns a rule, not another case

Two changes, and the second is the one that matters:

1. `HUMAN_ATTRS` gains the component props that carry human text —
   `empty-message`, `confirm-label`, `loading-label`, `title`, `description`,
   `item-noun`, `scope`, `action`, `value-text`, `placeholder`.
2. The `<script>` scan learns assignment: a string literal assigned to a `ref`,
   or passed to a function, that `isTranslatable()` accepts. The classifier
   already exists in `ui/build/i18n-shared.mjs` and already returns `true` for
   every example in §2.1 — the audit simply never asks it about these positions.

The second is the rule. Human-readable text is text that reaches a human,
wherever it is written, and the scanner's job is to find it rather than to
enumerate the places it has been found before.

### 4.2 The catalogue gate asks whether a key is used

A new case in `catalogues.test.ts`: every key in `en.json` appears somewhere
under `ui/src`. It fails today with 94 keys, so it lands with the cleanup — the
same commit deletes the dead ones and turns the count into an invariant.

Two exemptions, both narrow and both declared in the test rather than inferred:
keys resolved dynamically from a data table (`adminSections.ts`, `navigation.ts`)
are already proven by the existing navigation case, and a documented allowlist
covers anything a future dynamic lookup needs. An allowlist that grows without
comment is the failure mode; the test requires a reason string per entry.

### 4.3 Every page gets the test that would have caught its own defect

Not coverage for its own sake. One test file per page, and the assertions are
derived from the page's stated question:

| Page | The assertion its absence allowed |
| --- | --- |
| `AdminHealth` | an errors table that reflows without pushing the document |
| `AdminConfigReload` | a diff carrying only `access_config_changed` still renders a decision surface; an expired pending disables Apply |
| `AdminWarming` | a warm failure names the registry |
| `AdminTeamNamespaces` / `AdminBetaChannel` | "none for this registry" never renders while loading |
| `AdminSbom` | `from > to` is refused at the edge |
| `AdminNotificationSubscriptions` | an event type outside `ALL_EVENT_TYPES` is not silently re-saved |
| `AdminInboundEvents` | an unsigned event is distinguishable from a signed one |
| `AdminAccessCheck` | the query prefills; the SDK is called rather than `window.fetch` |
| `AdminPackages` | select-all → bulk block states its count before acting |
| `AdminDashboard` | (exists) |

---

### 4.4 The ramp check runs on every rendered route

`build/design-authed.mjs` measures the type ramp and the display face, and its
route list is authenticated-only. `Taskfile.yml`'s `ui:design:rendered` covers
the public routes — `/`, `/login`, `/packages`, `/setup`, `/tools/*` — but runs
only `impeccable detect` and axe, neither of which knows what a ramp is.

The two lists are merged: one gate, every rendered route, both viewports, with
the ramp and display-face assertions applied to all of them. The authenticated
half already seeds a token; the public half simply does not, which is a flag
rather than a second script.

It will fail on landing for `/packages` (§2.7), and that failure is the point —
it converts an artefact nobody was comparing against into a check that runs. How
the failure is *resolved* is O3's question, not this one's: the gate may
legitimately be satisfied by re-cutting the page, or by the proof being
superseded, but not by nobody knowing they disagree.

---

## 5. The API gaps

| # | Gap | Shape of the fix | Cost |
| --- | --- | --- | --- |
| A1 | **Access-check ignores account and IP blocks** (`access_check.rs:105`) | consult `UserBlockRepository` and `IpBlockStore` before the rules loop; return `blocked_by: "account" \| "ip" \| null` | handler + DTO field |
| A2 | `RegistryHealthDto` lacks `beta_channel_enabled` and `mode` | two fields on an existing response | field |
| A3 | `WarmResponse` returns counts only, never which package failed | a `failures[]` array, as the bulk endpoints already have | field |
| A4 | `PendingReloadSnapshot` omits `warnings`, which `PendingReload` already holds | one field | field |
| A5 | No revert path for config | a `POST …/config/history/{id}/restore` | endpoint |
| A6 | `count_packages_in_namespace` exists on the port, unexposed | a count on the namespace list response | field |
| A7 | No UI for audit retention purge (`DELETE …/audit-log?before=`) | UI only — the endpoint exists | UI |
| A9 | **`/api/v1/me/downloads` cannot say a pull is now blocked.** `MyDownloadDto` carries registry, name, version, artifact and a timestamp, so `RecentPullsWidget` renders a blocked pull identically to any other — on `/`, the one surface a non-admin opens. Reported from a live instance, not review: an operator blocked an artifact, saw it flagged in the catalog, and found the home page silent | a `blocked` field, set from one `blocked_only` scan rather than a lookup per row | field |
| A8 | **No way to list known subjects.** `/api/v1/admin/users/blocked` returns only the blocked, so four subject fields (§2.8) can neither suggest nor validate | a `GET /api/v1/admin/subjects?q=` over the identities the audit log and ownership tables have actually seen | endpoint |

**A1 is the only correctness defect**; the rest are absences. Six of the nine
are a field on a response that already exists, which is why they are one RFC
rather than nine.

A9 was added after the rest shipped, from a user report rather than a reading of
the tree — which is the point of it: §2.2's "the console cannot show what the
API does not describe" was argued about admin pages, and the surface it was
actually true of was the home page.

A8 is the only gap here whose absence is currently *invisible*: a subject field
with no source does not error, it returns an empty result set that reads as an
answer. It is scoped to what the instance has seen — audit subjects and
namespace owners — rather than a user directory, which this product does not
have and should not grow one of here.

A1 also carries a design decision the implementer must not skip: a simulated
request has no client IP, so IP-block simulation needs either an explicit
`client_ip` input or an honest statement that it covers account blocks only.
Answering "allow" because no address was supplied would reproduce the defect one
level down.

---

## 6. Composition still owed

### 6.1 Finishing the Phase 5 verdicts

Each of these is an *update* verdict already argued in Phase 5, with its
before/after recorded. Nothing here needs a new review.

- **`AdminPackages` — the column drop.** Make the name cell the link to the
  package page and the version cell the link to the artifact query, which
  deletes the unlabelled nav column; fold `artifact` into the name cell; move
  `last_accessed_by` to the detail page. Six columns fit 1134px with room, and
  the row verbs come back on screen at the console's standard width.
- **`AdminHealth` — delete the aggregate card.** It reads the same
  `adminStats().aggregate` the dashboard states as a sentence, renders it as
  four tiles without the trend, and its Refresh button does not refresh it
  (`useApi` destructured without `reload`). Removing it also removes the page's
  second fetch.
- **`AuditLog` — use the query surface.** `denied_only` is the single most-used
  audit filter and has no control at all; client-side filtering of page 0 is a
  correctness bug rather than a limitation, because it silently answers "no
  denials" for anything past the newest hundred rows.
- **`AdminWarming` — registry-type-aware fields.** One table of warmable
  registries, the help sentence written once instead of twenty-two times, and
  package/path fields chosen by `registry_type` — joinable client-side from
  data the page already loads.
- **Live regions.** `Announcer` gains its consumers: bulk results, block and
  unblock, warm, cache invalidation, config apply.

---

### 6.2 The field sweep

One primitive, then one pass over every field, with a verdict per field rather
than a blanket upgrade. The verdicts are already determined by §2.8's evidence:

| Verdict | Fields | Why |
| --- | --- | --- |
| **`Select`** | the four free-text registry fields | The set is closed, small, and already fetched. A combobox here would be a search box over eight items — more machinery for a worse answer. `AdminNotificationSubscriptions`' is *optional* ("leave blank for all registries"), so it needs an explicit **All registries** option — a `Select` that cannot express "blank" would silently narrow every subscription relying on it. |
| **Combobox, local source** | version (after registry + name), namespace prefix | Closed set, known once its parent field is set, and small enough to load whole. The field is disabled with a stated reason until its parent is answered. |
| **Combobox, server source** | package name in all six places | Debounced against `name_contains`, which is paginated and already ships. |
| **Combobox, server source, once A8 lands** | the four subject fields | Until then they stay free text — but they say so, rather than returning an empty table that reads as an answer. |
| **Stays free text** | reason / CVE, token name, banner message, the bulk CSV textarea | No set exists. A suggestion here would be an invention. |

The primitive lands first and once, in `ui/src/components/ui/combobox/`, and it
is a real component: `role="combobox"`, `aria-expanded`, `aria-controls`, an
owned listbox, arrow-key and Home/End navigation, `aria-activedescendant`, Escape
to revert, and the `Announcer` from §2.6 reporting the result count — which is
the second consumer that live region has been waiting for. `<datalist>` is not
that, which is why the two pages that used it are converted rather than left.

It obeys the world: no card, no shadow, the ground and ink of the surface it
sits on, the same 1px border as `Input`, and it enters `DESIGN.md`'s component
list rather than existing only in the tree.

Three behaviours are not optional, because each is how this pattern usually
fails:

- **A suggestion never blocks a submission.** Every one of these fields must
  still accept a value the server has not seen — warming a package that is not
  cached yet is the *point* of `AdminWarming`, and a combobox that only accepts
  what it can already offer would break it.
- **No result is a stated answer**, not an empty popup: "nothing cached matches
  `lodahs`" is the message that catches the typo. This is the same defect as
  §2.4 and the audit-log envelope — a blank that reads as a fact.
- **The upstream source is opt-in per keystroke-free action.** `explore/upstream`
  reaches a third-party registry; typing must not. Local matches appear as you
  type, and searching upstream is an explicit affordance below them (O4).

---

### 6.3 The cache the field sweep depends on

Three fixes, ordered by consequence. They land **before** §6.2, because the
sweep multiplies each of them by fourteen.

1. **The cache is scoped to the identity it was filled for.** The store clears
   when the identity changes — not only on `logout()`, but on any token or
   `identity` transition, so an anonymous visitor who signs in does not keep
   reading the anonymous view. Clearing on the transition is preferred to adding
   the viewer to the key: a key carrying `is_admin`, `is_authenticated` and
   `groups` would still hold the old viewer's rows in memory, and the whole
   point is that they stop being readable.
2. **The upstream search gets a cache**, keyed `name::registry`, with a TTL well
   under the listing's five minutes — this is third-party freshness, not our
   own data — and it is the *only* thing that makes a suggesting field
   acceptable on a path that reaches npm. It sits in the client — one minute,
   per operator — because a server-side cache changes who the upstream sees the
   query from and makes one operator's typo everyone's cached answer (O4).
3. **Both fetches guard their ordering** with a sequence number: a response
   whose sequence is not the latest is written to the cache and *not* to the
   display. Writing it to the cache is deliberate — the response is correct for
   its own key, it is only stale for the screen.

The store's own tests grow the case that would have caught this: fill it as one
identity, change identity, assert a miss. `PackageCatalog.vue` gets the
component test it never had (§4.3's rule, applied to a page outside `/admin`),
asserting that a second search for the same term issues no second request and
that a slow first response does not overwrite a fast second.

---

## 7. `DESIGN.md` findings

Recorded under RFC 0004 §3/R4 and still not acted on. Listed in the order they
should be taken, which is not the order of severity — the first is a decision,
the rest are consequences of it.

1. ~~**The world has no token for "degraded but not refused."**~~ **Resolved —
   copper gained the job (O1).** `--copper` was specified as *pending or held,
   never good*, with an enumerated job list that did not include a metric that
   had got worse. The dashboard's falling hit rate and the quota meter's warning
   state had **independently improvised the same missing job**, and one of those
   improvisations was pinned by a test (`AdminDashboardTrend.test.ts` asserts
   `text-copper`). Two surfaces reaching for the same absent thing is the shape
   of a job the system needs, and The One Synthetic Rule forbids a sixth hue —
   so `DESIGN.md`'s Secondary entry now authorises the job rather than the two
   pages continuing to assume it. Both improvisations are legal as written; no
   code changes. This was the one `DESIGN.md` finding B7 allowed into this RFC's
   window, and it is the only one taken.
2. **`Card` is used in 29 files and `rounded-sm` in 31**, against "There are
   none. The system has no card" and "Zero radius, everywhere". This is the
   migration `ui/src` has not had. It is large, mechanical, and must not be
   mixed with composition work.
3. **No grammar for a row-level action column.** Every admin list needs per-row
   verbs; with no rule they default to crimson fills, which is how one view
   reached eleven of them against a language that allows one.
4. **No spec for a bulk-selection context bar.** With no card, no shadow and no
   dependable fill, nothing in the vocabulary distinguishes a persistent
   selection bar from the sheet — which is why the existing one reached for
   `bg-card shadow-sm`, against the Flat-At-Rest Rule.
5. **Tailwind's shadow utilities are not neutralised**, so `shadow-sm` paints a
   real shadow anywhere someone types it.
6. **`Button` keeps its crimson fill when `:disabled`**, against "crimson never
   appears in a disabled state".
7. ~~**"Resolution as State" is unimplemented.**~~ **Resolved — it shipped as a
   component (§14.6).** DESIGN.md's organising idea — the 3×3 dot matrix for
   what is held and verified, the coarse 2×2 for what is not — existed nowhere
   in `ui/src`. It was found twice independently, by a Phase 5 reviewer and by
   §2.7's direct measurement against the proof. It now lives once, in
   `ui/src/components/ui/resolution/`, transcribed from DESIGN.md's own
   six-state table and consumed by `/packages` and the package detail page —
   rather than being invented per page, which is what produced eleven crimson
   row-verbs and two `<datalist>`s. The Display step (item 9) followed in
   §14.9, which closed O3 and emptied the pin.
8. **No grammar for a text field that suggests.** §6.2 lands one, and the world
   has no entry for it — the same gap as items 3 and 4, found the same way. It
   is written into `DESIGN.md` as part of that work rather than after it.
9. ~~**What the Display step is spent on.**~~ **Resolved for `/packages`
   (§14.9); still open for every other page.** The proof gives it to the registry
   being viewed; every shipped page gave it to the page's own name
   ("Dashboard", "Packages", "Bulk Block"). A Phase 5 reviewer raised the same
   question from the admin side: the loudest element in the system currently
   carries a nav label while the page's actual content — an alarm, a destructive
   count, the subject — sits at 14px. Whether an Operate page should carry the
   Display step at all, and on what, is a question about the world.

---

## 8. Alternatives considered

| Alternative | Why rejected |
| --- | --- |
| Fold this into RFC 0004 as new phases | RFC 0004 is shipped and its decision log is closed. Its §11 says discoveries during implementation become new rows, not new scope — and seven of these are rows it already carries. A shipped RFC that keeps growing stops being reviewable. |
| Fix the five English strings and move on | It leaves the scanner unable to see the *class*, and the class has already produced two incidents: the admin navigation in §2.1 and these five. The rule is the deliverable; the strings are the symptom. |
| Delete the 94 unused keys without the gate | It restores the invariant for exactly as long as nobody removes another page. RFC 0004's own merge created most of them, in a commit that passed every gate. |
| Make the access-check simulator consult blocks in the UI | The UI would have to fetch two more endpoints and re-implement middleware ordering in TypeScript. The decision belongs where enforcement is, or the two disagree the first time either changes. |
| Re-cut `/packages` to match the proof as part of this RFC | The divergence is two decisions — what the Display step is spent on, and whether "resolution as state" becomes a component — and both are world-level, inherited by every surface. RFC 0003 Phase 1 used Impeccable to reach them; reproducing them by eye from a checked-in screenshot is how a specimen becomes a pastiche. This RFC makes the disagreement fail a gate; O3 decides which side changes. |
| Delete the design proof, since the page has moved on | It is the only checked-in statement of what this world looks like, and `DESIGN.md` refers to the decisions it embodies. Deleting the evidence to resolve a disagreement with it is the wrong direction — if the proof is superseded, that is a decision to record, not a file to remove. |
| Put a `<datalist>` on the fields and be done | It is what the two existing cases did, and it is why this is in an RFC: `<datalist>` cannot be styled to the world, its keyboard behaviour differs per browser, it exposes no listbox to assistive tech, and it cannot render a "no match" message — which is the one behaviour §6.2 exists to get. |
| Upgrade every free-text field to a combobox | Four of them have no set to offer (reason, CVE, token name, banner text), and four more have no source until A8. A blanket upgrade would invent suggestions for fields that have none, which is worse than typing. The sweep is per-field with a stated verdict for that reason. |
| Take the `DESIGN.md` migration first | It touches 29 files and would make every subsequent composition regression unattributable. RFC 0004 §8 made this argument for the same reason and it has not weakened. |
| Add tests only to pages that changed | The four pages that gained tests in Phase 5 each had a defect the test caught immediately. There is no reason to believe the eleven untested ones are cleaner — only that nobody has looked. |

---

## 9. Rollout and compatibility

- **The gate changes fail on landing, by design.** §4.1 and §4.2 both surface
  existing conditions, so each lands with its own cleanup in the same commit —
  which is the only moment the count is provably correct.
- **A1 changes an answer.** A simulation that returned `allow` for a blocked
  account will return `deny`. That is the point, and it is a behaviour change to
  announce rather than slip in: anyone who built on the old answer was building
  on a wrong one.
- **A2–A4 and A6 are additive fields.** No response shrinks; the generated
  client picks them up through the existing `task dump-spec` → `task ui:generate`
  path, and the RFC 0004 contract gate already refuses a body-less success.
- **§6 changes no route.** §6.1's five items are within pages that keep their
  paths, and §6.2 replaces inputs in place, so no `LEGACY_REDIRECTS` entry and no
  change to `EXPECTED_COMBINATIONS` from either.
- **§6.2 changes what a field accepts, never what it rejects.** Every converted
  field still takes a value the server has not seen, so no operator workflow that
  worked before stops working — including warming a package that is not cached,
  which is the case the suggestion source cannot cover by definition.
- **A8 is a new endpoint**, admin-scoped, additive, and read-only. The four
  subject fields degrade to their current free-text behaviour if it is absent.
- **§6.3 makes `/packages` re-fetch more often, on purpose.** Clearing the
  explore cache on an identity transition means signing in or out costs one
  extra listing request. That is the correct trade, and it is the only
  user-visible slowdown in this RFC.
- **No migration, no config change, no `CURRENT_CONFIG_VERSION` move.**

---

## 10. Test plan

- **The gates are the test.** §4.1 lands with the five strings translated and a
  case asserting a component prop carrying English fails the audit. §4.2 lands
  with the 94 keys resolved and the reference check as an equality.
- **A1 gets the absence assertion RFC 0004 §7 established**: block an account,
  simulate it, assert `deny` and a `blocked_by` of `"account"` — and assert the
  reverse, that an unblocked account with no matching rule still returns
  `allow`, so the new check cannot become a blanket denial.
- **§4.4 lands with its own failure recorded, not hidden.** The merged gate
  asserts the ramp and display face on every rendered route; `/packages` fails
  it on arrival (§2.7). The failure is pinned as an expected-fail with O3 named
  as its owner, so the gate is green for every *other* route and the one
  unresolved disagreement is a line in the output rather than a silence. It is
  un-pinned by whichever side of O3 moves.
- **§6.3's identity case is the test the store never had**: fill the cache as one
  identity, transition, assert a miss. The existing `useExploreCache.test.ts`
  passes today *because* it only ever tests one viewer — the suite is extended
  rather than trusted.
- **The combobox is tested as a component, once**, against the keyboard contract
  in §6.2 — arrow keys, Home/End, Escape reverting, `aria-activedescendant`
  tracking, and a free-typed value the source never offered surviving submit.
  Per-field tests then assert only which source that field is bound to.
- **Each new page test asserts the page's question**, per the table in §4.3, not
  its markup. A test that pins a class name re-breaks on the `DESIGN.md`
  migration and teaches nothing.
- **The RFC 0004 gates stay green throughout** — 48 authenticated
  route/role/viewport combinations, the rendered detector at both viewports,
  `task ui:design`, and the contract gate. `EXPECTED_COMBINATIONS` moves exactly
  once, in the §4.4 commit that merges the public routes into the same gate, and
  its new value is derived in that commit's message. Any *other* movement means
  something in §6 changed a route, and that is a review question — which is the
  whole point of the constant.

---

## 11. Decisions and open questions

### Resolved

| # | Question | Decision |
| --- | --- | --- |
| B1 | Extend the i18n scanner, or move the five strings? | **Both, and the scanner first.** The strings are a symptom of a scanner that enumerates positions rather than asking whether text reaches a human. This is the second incident of that exact shape; `adminSections.ts`'s own header records the first. |
| B2 | Should the catalogue gate check reference? | **Yes, with a declared allowlist.** A parity gate proves every key is translated and is silent about whether any is needed — which let three correct French sentences sit unused while the page hardcoded English, and let RFC 0004's own merge orphan a family of keys with everything green. |
| B3 | Fix the access-check simulator in the UI or the handler? | **The handler.** Re-implementing middleware ordering in TypeScript makes the two disagree the first time either changes, and the decision belongs where enforcement is. The UI's stated bound (RFC 0004 Phase 5) is the interim, not the answer. |
| B4 | Does A1 need a simulated client IP? | **It must not answer as if it had one.** A simulation with no address either takes an explicit `client_ip` or states that it covers account blocks only. Returning `allow` because no address was supplied reproduces the defect one level down. |
| B5 | Are the three partly-finished Phase 5 verdicts re-reviewed? | **No.** Their evidence was gathered against rendered pages and is recorded. Re-reviewing would invite a different answer to a question already decided, which is how a re-cut becomes a rewrite. |
| B6 | Test every page, or only the ones being changed? | **Every page.** Four pages gained tests in Phase 5 and all four caught a defect immediately — including one (`Dialog` has no `footer` slot) found by a test written for an unrelated reason. The eleven untested pages are not known to be cleaner. |
| B8 | Does this RFC re-cut `/packages` to match its design proof? | **No. It makes the disagreement fail a gate, and stops there.** The two differences are what the Display step is spent on and whether "resolution as state" becomes a real component — both world-level, inherited by every surface. RFC 0003 Phase 1 reached them through Impeccable; reproducing them by eye from a checked-in screenshot is how a specimen becomes a pastiche. A visible red gate is a decision waiting to be taken; a hand-retrofitted page is a decision taken by whoever was nearest. |
| B9 | Is A5 (a config revert path) buildable as specified? | **No — declined, with the row §3 requires.** `config_changes` stores a *diff summary* — added, removed and changed registry names — and never the config itself, so `POST …/history/{id}/restore` cannot be built on it: a list of registry names does not reconstruct a TOML file. Making it possible means storing full config content per change, and `config.toml` carries `upstream_auth` credentials, static bearer tokens and OIDC client secrets — every secret this instance has ever been configured with, in a table any admin can read through an API, retained indefinitely and surviving the rotation that was supposed to end their life. The revert path that exists is the config editor; what it lacks is the *previous* content to paste, and closing that honestly is a question about where config history should live (a git-backed config, an encrypted store), not a field on this response. Recorded at `handlers/back_office/config.rs`. |
| B7 | Does the `DESIGN.md` migration belong here? | **No — but the copper token decision might.** Retiring `Card` is mechanical and enormous; the missing "degraded" token is a decision the system has now improvised around twice, and every further surface that needs it improvises again. The token decision may be taken in this RFC's window; the migration may not. |
| O1 | Does the "degraded but not refused" job get a new token, or an existing one gains a job? | **Copper gains the job**, taken in this RFC's window per B7. `DESIGN.md`'s Secondary entry now authorises *a measured value moving the wrong way but not yet refused*, naming the falling hit rate and the approaching quota, which makes both improvisations legal rather than tolerated. The alternative — a fifth condition carried by the dot pattern instead of a hue — was rejected on subject: the matrix's six states all describe one artifact's resolution, and an instance-wide metric is not one of them, so it would have needed a seventh row that does not fit the table's own subject *and* a component that does not exist (§7 item 7). Copper's negative half is unchanged: never *good*. |
| O3 | When the ramp gate goes red on `/packages`, which side moves? | **The page moved.** Both halves are closed: "resolution as state" shipped as a component (§14.6), and the Display step is now spent on the registry being viewed (§14.9). The deciding fact was that the cost argument was false — the proof is runnable source, and `--t-display` had simply never been mapped to a utility, so the page had been reaching for the largest step that existed. The `EXPECTED_FAIL` pin came out in the same change, on the gate's own instruction: it failed *because* `/packages` had started passing while still pinned. |
| O4 | Does the package-name combobox ever query upstream implicitly, and where does that cache live? | **No, and client-side** — ratifying what §6.2 and §6.3 built. Typing never leaves the instance: `useSuggestions.ts` binds only local sources and `Combobox.vue` renders the upstream search as an explicit affordance below them. The cache is per-operator (`_upstream` in `useExploreCache.ts`, 1-minute TTL against the listing's five). A server-side cache was rejected because it inverts who the upstream sees the query from and makes one operator's typo everyone's cached answer; a per-registry setting re-enabling the implicit form was rejected because it reintroduces exactly the 5N third-party calls §2.9 measured, and would cost a config field and a `CURRENT_CONFIG_VERSION` move this RFC otherwise avoids. |

### Still open

Both are deferred to another RFC rather than undecided here, and each names the
RFC that owns it. Neither blocks anything in §12.

| # | Question | Why it is open |
| --- | --- | --- |
| O2 | Is `AdminSbom` in the right section at all? | Phase 5 moved it from Observability to Operations because it observes nothing. If a future advisories surface lands under Security & Access, it may belong there instead — and that surface is RFC 0002's, not this one's. |

---

## 12. Implementation phases

Each phase leaves the tree green: `cargo test --workspace`,
`cargo clippy -- -D warnings`, `vue-tsc`, `vitest`, `oxlint`, and every RFC 0003
and RFC 0004 gate.

| Phase | Content | Useful on its own? |
| --- | --- | --- |
| 1 | **The gates stop over-reporting.** §4.1, §4.2 and §4.4, each landing with its own cleanup — the five strings translated, the 94 keys resolved, the ramp check merged onto every rendered route, all three turned into invariants. | Yes, and it is first for the same reason RFC 0004's contract sweep was: every later phase is measured by these. |
| 2 | **A1 — the access-check simulator tells the truth.** Handler consults both block stores, `blocked_by` discriminator, the absence tests of §10, and the UI's interim bound replaced by the real answer. | Yes. It is the only correctness defect in the RFC. |
| 3 | **The remaining API gaps.** A2–A4, A6, A7, each a field or a small UI on an endpoint that exists; A5 if it survives its own design; A8, which unblocks the subject fields in phase 4. | Yes, per gap. |
| 4 | **Finish the composition.** §6.1's five verdicts, one page per commit; then §6.3, then §6.2 — the combobox primitive first and alone, then the field sweep, one commit per source. | Yes, per page and per field group. §6.3's first item ships on its own ahead of everything else in this phase; the primitive is useless alone, so it lands with the registry `Select`s and the version field in the same commit. |
| 5 | **The page tests.** §4.3, one file per page, assertions derived from each page's stated question. | Yes — and it should arguably run alongside phase 4 rather than after it, so each re-cut lands with its regression signal. |
| 6 | **§13.1 — the licence gate.** Licence extraction in the five archive extractors, the licence persisted and looked up by coordinate, `LicenseGateRule` and `[registries.license_gate]`, the licence surfaced on the package detail page. §13.2–13.4 are specified only. | Yes, and last: it is the one phase that adds product surface, so it is separable from everything §1–§12 owes. |

§6.3 precedes §6.2 within phase 4, and its first item — the cache not outliving
its identity — is the one thing in this RFC that need not wait for phase 1. It
is a small, self-contained fix to a live confidentiality edge, and it should go
out as soon as someone has the time, ahead of the whole schedule.

§6.2's subject fields depend on A8, which is phase 3 — so the field sweep is
ordered registry → version → package name → subject, and the last group simply
does not land if A8 slips. Everything before it is independent.

Phase 1 is first and alone. The other four are all measured by gates that
currently cannot see what they claim to, and finishing work under an
unobservant gate is how this RFC's contents accumulated in the first place.

§4.4 landed with one known failure — `/packages` against its own proof — pinned
rather than hidden, on the argument that an unclosed gap everyone can see beats
a green gate over a page nobody was comparing to anything. It was the only place
in this RFC where landing a gate did not also land its fix.

It has since been closed (§14.9): the page moved, the pin came out, and the gate
is green on all 60 combinations. The pin earned its keep on the way out — the
gate failed *because* the route had started passing while still pinned, which is
the assertion §4.4 wrote on the theory that a stale pin is its own kind of
silence.

---

## 13. Addendum — four product gaps nothing was tracking

This section breaks §3's "no new product surface" non-goal, and says so there.
It exists because these four were found by reading `crates/` against
`ROADMAP.md` — a different method from the rest of this RFC, which reviewed
pages — and no other document was carrying them.

The roadmap's eleven unchecked items are *known* absences with entries. These
four have no entry anywhere. Only §13.1 is implemented in this RFC's window; the
other three are specified here and built elsewhere, which is stated per item
rather than left to be inferred.

### 13.1 No licence policy rule

`crates/core/src/rules/` holds eight rules — `rbac`, `deny_latest`,
`block_list`, `release_age`, `cve_gate`, `version_gate`, `signed_release`,
`trusted_publisher`. There is no licence rule, and `crates/config` has no
`license_gate` block. An organisation that runs a proxy for compliance reasons
asks "block AGPL" about as often as it asks "block criticals", and today the
answer is that BatleHub cannot express it.

RFC 0002 lists `licence` as a **flag kind** an external source may push. That is
a different mechanism — it requires a SOC or vendor feed to assert something
per version. A licence rule reads what the package *itself* declares, needs no
external source, and is the one an instance with no security vendor can use.

**The data does not exist yet, which is the real cost.** `ArtifactSbom`
(`entities/sbom.rs`) stores `document: serde_json::Value` and nothing else about
content; `services/sbom/generate.rs` emits neither SPDX `licenseDeclared` nor
CycloneDX `licenses`; and grepping `crates/core` and `crates/adapters` for
`licen` finds only unrelated hits in the conda, pacman and rpm adapters plus a
comment on `PackageMetadata::extra`. So this is not "copy `cve_gate.rs`": it is
licence *extraction* first, then the gate.

Extraction has one good home. `ArchiveSbomExtractor`
(`adapters/src/sbom/extractor/`) already opens the archive and parses the
manifest for dependencies, in five ecosystems — cargo, npm, maven, pypi, nuget.
The licence is declared in the same file it already reads (`Cargo.toml`'s
`package.license`, `package.json`'s `license`, `pom.xml`'s `<licenses>`,
`METADATA`'s `License-Expression` and its `Classifier: License ::` fallback,
`.nuspec`'s `<license>`). It costs one more field off a parse that already runs.

**A limit this design cannot remove, stated rather than hidden.** The manifest
is inside the archive, so the licence is only known *after* the artifact has
been downloaded. On the very first request for an uncached package the rule runs
before there is anything to read, and it cannot answer. Two honest options, and
the config exposes both:

- `allow_unknown = true` (default) — the first fetch proceeds, the licence is
  recorded on the way through, and every later request is gated. Deny is
  therefore *eventually* consistent, which is the correct trade for a proxy
  whose job is to serve.
- `allow_unknown = false` — an unknown licence is a denial. Conservative, costs
  a first fetch per package, and is the same shape as
  `integrity.require_metadata`, which already makes exactly this trade for
  checksums.

Pretending otherwise — evaluating against an absent licence and calling it
`allow` — would be the §2.4 defect again, one subsystem over.

**The partial coverage is itself a gate problem, so it is treated as one.**
Five registry types have a parser; sixteen do not. On those sixteen a
`license_gate` is silent at runtime in both directions — with the default it
never denies, and with `allow_unknown = false` it denies everything — and
neither state errors, logs, or fails validation. That is this RFC's own §1
through-line reappearing inside the feature it added: *a rule that cannot
observe a condition reports the same green as one that observed it and found
nothing.*

So the config warning is part of §13.1, not a follow-up.
`AppConfig::warnings()` gains `license-gate.no-extractor` and
`license-gate.denies-everything` — two codes rather than one, because the
consequences are opposite and the second is what an operator will be triaging
under pressure. They surface on the Config Reload page and at
`GET /api/v1/admin/config/warnings` through the existing generic renderer, so
no console change is needed.

The list of covered types lives in `LICENSE_EXTRACTION_TYPES` in
`crates/core/src/ports/sbom.rs` and is read by both the adapter's dispatch and
the warning, with a test in `extractor/mod.rs` refusing the drift. A parser
added without updating the const would warn about a registry type that now
works; a type added without a parser would silence a warning that is still
true. Either way the operator is told something false about their own policy,
which is the failure this RFC exists to stop.

**Not in scope for §13.1**: dependency-licence gating. The extractor reads the
manifest's *own* declaration, so the rule answers "what licence is this
package", not "what licences does its dependency tree carry". The second needs a
resolved graph, which BatleHub does not build.

### 13.2 No instance-to-instance transfer

`docs/configuration.md` §6.16 routes upstream traffic through a corporate HTTP
proxy, which covers a *restricted* network. It does not cover a disconnected
one: there is no export bundle, no import path, and no way to point one BatleHub
at another as a source. `docs/high-availability.md:241`'s `upstream batlehub` is
an nginx block, not instance chaining.

An air-gapped estate is one of the strongest reasons to run a caching proxy at
all, and today the only route in is restoring a backup of an entire instance —
which moves the database, the config and every credential in it, not a set of
artifacts someone approved.

**Specified only.** A bundle format, its signing, the import path and its
interaction with content-addressable dedup are a subsystem, and they belong in
their own RFC with their own phases.

### 13.3 No storage-backend migration

`[storage]` supports filesystem and S3 behind a router
(`adapters/src/storage/router/`), and artifacts record which backend holds them
— `handlers/back_office/packages/detail.rs:63` returns a `storage_backend` that
is null "if not yet cached or pre-migration". But there is no migrate, move or
rebalance operation anywhere in `adapters/src/storage/` or `core/src/services/`.
Change the backend in config and the existing artifacts stay where they are,
reachable only for as long as the old backend remains configured.

**Specified only.** The operation is a walk with resume, partial-failure
tolerance and dedup ref-count awareness — moderate, self-contained, and not
something to bolt onto a console RFC.

### 13.4 No seeding from an existing registry

`batlehub-cli registry suggest` writes *config* for a new instance. Nothing
imports the *contents* of an incumbent Nexus, Artifactory or Verdaccio. That is
the migration path for exactly the users most likely to adopt this, and its
absence means every adoption starts from an empty cache.

**Specified only.** Per-source adapters plus an ingestion path; its own RFC.

### 13.5 Rollout

§13.1 lands as one phase, after phase 5, and touches no route: a new rule, a new
config block, one field on an existing response, and the licence rendered on the
package detail page. `CURRENT_CONFIG_VERSION` does not move — the block is
optional and its absence is the current behaviour.

The three specified-only items get roadmap entries in the same commit as this
addendum, so they are tracked whether or not their RFCs are ever written.

---

## 14. What execution caught

This RFC's argument is that a gate reporting green over a condition it cannot
observe is indistinguishable from one that looked. That argument applies to the
RFC itself, so this section separates what was *verified by running it* from
what was only read. Four findings; none of them came from review.

### 14.1 The licence gate did nothing, and nothing said so

§13.1 was written, tested (2612 unit and integration tests), clippy-clean and
merged into the phase table before it was ever executed against a server. On
first run against a real Postgres and a real npm upstream, the licence came back
`null` for every version — four times in a row, for three different packages.

The parser was fine. `ProxyService::maybe_trigger_sbom`
(`services/proxy/resolve.rs:139`) returns early unless the registry has an
enabled `[registries.sbom]` block, and **the licence is recorded as a side
effect of SBOM generation**. With SBOM off, nothing is extracted, so a
`license_gate` sees an unknown licence for every version however good its
parser — and says nothing about it, because the config is valid and the rule
loads like any other.

That is this RFC's own §1 defect, reproduced inside the feature §13 added. It
was fixed the way §4 fixes gates: `license-gate.sbom-disabled`, whose message
changes according to whether the combination makes the rule inert or makes it
refuse every download. Confirmed live — the running server reported it at
`registries[2].rules[0]`.

**No amount of unit testing would have found this.** Every test supplied its own
extractor or its own repository; none of them went through the call site that
decides whether extraction happens at all.

### 14.2 The chain, once it worked

Recorded because "it compiles" and "it serves a 403" are different claims:

| Step | Observed |
| --- | --- |
| `GET /proxy/npm/ansi-regex/6.2.2/tarball` | 200, artifact cached, licence extracted |
| `GET /api/v1/admin/packages/detail` | `license = "MIT"` |
| `GET /api/v1/explore/packages/npm/ansi-regex` | `license = "MIT"` |
| `GET /api/v1/sbom/export?format=cyclonedx` | `strip-ansi → MIT`, `is-odd → MIT` |
| gate flipped to `deny = ["MIT"]`, re-request | **403** `blocked: licence 'MIT' is on the deny list` |

### 14.3 A coordinate mismatch the licence inherits

A request for `/proxy/npm/{name}` with no version records the package row under
the *requested* version — the literal string `latest` — while the SBOM is
recorded under the *resolved* one (`7.2.0`). The licence lookup is by
coordinate, so it joins nothing for those rows and the version reads unknown.

This is **pre-existing and not introduced here**: `list_vulnerabilities` on both
detail handlers is keyed the same way and has the same hole. It is recorded
rather than fixed, because changing what a `latest` request writes is a change
to the cache's own identity model and belongs in its own RFC.

### 14.4 §4.4's gate, on first execution

The merged ramp/display-face gate was wired, committed and described in this
document before it had run once — the same "wired but unexecuted" state RFC 0003
§13 exists to close. Run against the `che-browser` sidecar over CDP:

```
60 route/role/viewport combination(s) scanned, 0 with unexpected findings, 1 pinned
```

Every route passes at both viewports across anonymous, user and admin — except
`/packages`, which failed exactly as §2.7 measured by hand:

```
⚠ anonymous /packages @1440
    [type] spends 24px on its largest Silkscreen element;
    ui/design-proof/index.html spends 104px at this width, on the registry being viewed
⚠ /packages failed as expected — RFC 0004-bis O3
```

So §2.7's central claim is now machine-measured rather than eyeballed, and the
disagreement is a line in CI output owned by name. The pin is the designed end
state, not an outstanding task: it is un-pinned by whichever side of O3 moves.

One rough edge worth a follow-up: `EXPECTED_COMBINATIONS` is 30 and counts
route/role pairs, while the summary line reports 60 because it counts viewports
too. The guard (`planned < EXPECTED`) is correct and coverage still cannot
silently shrink, but two numbers 2× apart in the same output invite someone to
"fix" the wrong one.

### 14.5 What is still not verified

- **Line coverage.** `task coverage-check` enforces 80% and needs Podman, which
  is absent from the environment this was implemented in. §13.1 added a fair
  amount of Rust; its effect on the number is unmeasured.
- **§13.2–13.4** are specified only, by design, with roadmap entries.
- **O2** belongs to RFC 0002 and is the only question still open. O3 closed
  (§14.9) and `EXPECTED_FAIL` is now empty.

### 14.6 Resolution as State, and what reading the proof changed

§7 item 7 and O3 both rested on a premise this document asserted twice and never
checked: that re-cutting `/packages` means reproducing a **screenshot** by eye,
which §8 and B8 both call "how a specimen becomes a pastiche".

`ui/design-proof/index.html` is not a screenshot. It is 707 lines of runnable,
self-contained source with `fonts/` and `halftone-plate.png` checked in beside
it. The three pieces §2.7 measured as missing cost, in the proof's own code:

| Piece | Actual cost in the proof |
| --- | --- |
| Resolution matrix | ~12 lines of CSS (`.matrix.fine` 3×3 @5px, `.coarse` 2×2 @8px, `currentColor`, `.18` unlit) plus a six-entry state table |
| Display step | `--t-display:56px`, stepping 72/88/104 at 640/880/1140, on one `<h1>` |
| Halftone plate | ~10 lines, PNG already committed, including the `@supports` guard that stops a dropped mask painting a solid copper block |

So the cost argument was wrong, and it had been deterring the work. **The
resolution matrix — the system's signature, absent for two RFCs — is one of the
smallest components in the console.** It landed as
`ui/src/components/ui/resolution/`, transcribed from DESIGN.md's table rather
than from the proof's CSS, with 17 tests asserting against that table: cell
counts, lit counts, which cell is dark for `stale` (the spec says "centre out",
so *which* is the specification and not merely *how many*), the hue per state,
`aria-hidden` on the matrix with the word carrying it for assistive tech, and
`bg-current` so the mark takes its context's ink.

Deliberately **not** in the component: the `resolve` animation. DESIGN.md
forbids it "on load of unchanged content", and a component cannot know whether
it just changed — only the list rendering it can. Putting it there would make it
fire on every render, which is the one thing the spec names.

Consumers: `/packages` (cached → fine 3×3; an upstream hit never fetched →
`pending`, "not yet resolved") and the package detail page (clear → cached,
plus blocked and yanked one-to-one). The words stay each page's own — the
pattern and hue carry the resolution grammar, and "Not yet proxied" tells an
operator more than "Pending" would.

**The gate is unchanged at 60 scanned / 0 unexpected / 1 pinned**, correctly:
§4.4 measures the Display step, which this did not touch. O3 is now one question
rather than two.

### 14.7 §4.1's rule is still narrower than its own statement

Wiring the component surfaced three more untranslated strings, in
`PackageDetailPage.firewallLabel`:

```js
if (fw.status === "blocked") return "Blocked";
if (fw.status === "yanked")  return "Yanked";
return "Clear";
```

`pnpm run i18n:check` reported `0 untranslated strings` over them. §4.1 taught
the scanner component props and `ref` assignments; a string literal **returned
from a function** is neither, so it is the §2.1 class again, one position over —
and §4.1's own text claims the fix was "a rule, not another case".

The three strings are translated. **The scanner still cannot see the position**,
which means this is a known open blind spot rather than a closed finding. It is
recorded here rather than fixed because widening the scan to `return` needs
`isTranslatable` to hold the line against non-prose returns (class names, keys,
route paths), and that deserves its own change with its own false-positive
review — not a patch tacked onto a component landing.

### 14.8 A9, and where §2.2 was actually true

A9 did not come from reading the tree. An operator blocked an artifact
(`jetbrains-ide` / `repo` / `_` / `idea/idea-2026.1.3.tar.gz`), saw the catalog
mark it, and reported that the home page did not.

Every layer below the DTO was already correct.
`GET /api/v1/admin/packages?blocked_only=true` returned it;
`GET /api/v1/explore/packages` returned `has_blocked: true`; `/packages`
rendered it. `GET /api/v1/me/downloads` returned:

```json
{ "registry": "jetbrains-ide", "name": "repo", "version": "_",
  "artifact": "idea/idea-2026.1.3.tar.gz", "downloaded_at": "…" }
```

— five fields, none of which is the answer. `RecentPullsWidget` was not wrong;
it had nothing to render.

**This is §2.2's argument landing on a surface it was never aimed at.** RFC 0004
made that case about admin pages, and this RFC inherited the framing: §5's eight
gaps are all admin or explore reads. The `me` endpoints were treated as finished
because Phase 2 built them with their scoping and their absence tests. They were
finished as *reads of what you did*, and incomplete as *reads of what is true of
it now*.

Two details worth keeping:

- **Keyed on the full coordinate**, `PackageId::cache_key()`, which includes the
  artifact. A path-addressed registry stores its whole tree under one synthetic
  package name (`repo`, version `_`), so keying on name alone would mark every
  JetBrains download blocked the moment one file was.
- **One `blocked_only` query, not one per row**, capped at `MAX_BLOCKED_SCAN`
  with the cap named in the source. A failed lookup reports "not blocked" rather
  than failing the widget — losing the flag is a smaller harm than losing the
  list — and an absent flag on an older client renders nothing rather than
  asserting clean.

It was verified against the reporting instance's own data before being called
done: `"blocked": true`, on that exact artifact.

### 14.9 O3, and a token that was never adopted

`/packages` now matches its proof, the pin is gone, and the gate is green on all
60 combinations. The reason it was tractable at all is §14.6's finding, and the
root cause turned out to be one line.

**`--t-display` was mapped to no utility.** `ui/src/design/tokens.css` declares
it correctly — 56px stepping to 72/88/104 at 640/880/1140 — and `ui/src` had
**zero consumers**. The `@theme` block in `assets/index.css` maps `--text-xs`
through `--text-2xl` onto the ramp and stops there, so the largest utility that
existed was `text-2xl` at 24px, and every page reached for it. 24 *is* a
declared Silkscreen step, so nothing looked wrong to any check that asks whether
a size is on the ramp.

This is RFC 0003 §13 finding 4 again — `--font-family-*` generating no utility,
so the design's text face never painted. **A token that generates no utility is
a token that was never adopted**, and neither the token tests nor a source scan
can see the difference, because both inspect declarations rather than what a
page actually reaches for. `--t-sub` / `--t-px-sm` (16px) was unmapped for the
same reason and is now mapped too.

What landed:

- `--text-display` and `--text-sub` in `@theme`.
- The specimen replaces `PageHeader` on this route only. Its `<h1>` is the
  selected registry, or "All registries" when the facet is on all — a blank
  specimen would be §2.4's defect in a headline. The caption carries type, mode
  and counts on one ruled line; the proof also shows a cached size, which no
  endpoint on this page returns, so it is omitted rather than estimated.
- The halftone plate, `@supports` guard included: without it a dropped mask
  paints solid copper over half the specimen and `--ink-dim` on that composite
  falls to 3.21:1.
- The page's one action moved into the toolbar, where the proof keeps its own,
  leaving the specimen as the route's only `h1`.

**The long-name worry was real and had the wrong cause.** Predicted: a 13-char
registry name at 104px would wrap badly. Measured over CDP at three widths, it
did — `jetbrains-ide` came to **312px of headline, two lines**. The cause was
not the name: the `.display` rule in the proof sets `line-height:.92`,
`letter-spacing:.02em` and `text-transform:uppercase`, and the port had picked
up the body's 1.625 instead. With the proof's own rule the same name is **96px,
one line**, at the full Display step, with no horizontal overflow at 1440, 880
or 390. Setting a display face tight is most of what makes it read as display
rather than as a very large heading — and it is the kind of thing that only a
measurement finds, because both versions are "on the ramp".

**The pin removed itself, in effect.** With the page moved, the gate failed —
not on `/packages`, but on the *pin*: `✗ /packages is pinned in EXPECTED_FAIL
and now passes. One side of the disagreement moved — remove the pin in the
commit that moved it.` That inverse assertion was written in §4.4 on the theory
that a stale pin is its own kind of silence. It is the one part of this RFC that
has now been proven by being triggered rather than by being reasoned about.

`EXPECTED_FAIL` is empty, and §7 item 9 stays open for every page that is not
`/packages`: whether an admin surface carries the Display step at all is still
a world-level question, and this settled it only for the proving surface.
