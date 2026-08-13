# RFC 0004-bis — What RFC 0004 left, and the gates that could not see it

| Field       | Value                                                                 |
| ----------- | --------------------------------------------------------------------- |
| Status      | Draft                                                                 |
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

Four kinds of residue, and they are not equal:

1. **Three gates report success over conditions they cannot observe.** The i18n
   audit reads `0 untranslated strings` while five English strings render to a
   French operator. The catalogue gate proves every key is translated and never
   asks whether any key is *used* — 94 of 710 appear nowhere in `src/`, and the
   `adminIpBlocks.*` family among them was orphaned by RFC 0004's own merge with
   nothing failing. Eleven of fifteen admin pages still have no component test.
2. **Seven API gaps the UI is currently papering over**, one of which is a
   correctness defect: `POST /api/v1/admin/access-check` answers `allow` for an
   account the adjacent page shows as blocked.
3. **Composition work the Phase 5 verdicts identified and scoped but did not
   finish** — five items, each with its verdict already argued.
4. **`DESIGN.md` findings**, recorded under RFC 0004 §3/R4 and untouched since,
   including one the console has now improvised twice.

The through-line is the first category. RFC 0004 §2.2's argument was that the
console cannot show what the API does not describe; this RFC's is narrower and
sharper: **a gate that cannot observe a condition reports the same green as a
gate that observed it and found nothing**, and three of ours currently do.

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

**Non-goals**

- **Re-opening the Phase 5 verdicts.** They were reached against rendered pages
  with the evidence §4.4 required. This RFC finishes them; it does not relitigate
  them.
- **The `DESIGN.md` migration.** §7 records the findings and proposes the order
  to take them in, but retiring `Card` across 29 files is its own RFC with its
  own gates, and mixing it into composition work makes a regression
  unattributable — the same argument RFC 0004 §8 made.
- **New product surface.** Nothing here adds a page or a feature. Every item is
  a thing that already exists and is wrong, missing, or unobserved.

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

**A1 is the only correctness defect**; the rest are absences. Five of the seven
are a field on a response that already exists, which is why they are one RFC
rather than seven.

A1 also carries a design decision the implementer must not skip: a simulated
request has no client IP, so IP-block simulation needs either an explicit
`client_ip` input or an honest statement that it covers account blocks only.
Answering "allow" because no address was supplied would reproduce the defect one
level down.

---

## 6. Finishing the Phase 5 verdicts

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

## 7. `DESIGN.md` findings

Recorded under RFC 0004 §3/R4 and still not acted on. Listed in the order they
should be taken, which is not the order of severity — the first is a decision,
the rest are consequences of it.

1. **The world has no token for "degraded but not refused."** `--copper` is
   specified as *pending or held, never good*, with an enumerated job list that
   does not include a metric that has got worse. The dashboard's falling hit
   rate and the quota meter's warning state have now **independently improvised
   the same missing job**, and one of those improvisations is pinned by a test
   (`AdminDashboardTrend.test.ts` asserts `text-copper`). Two surfaces reaching
   for the same absent thing is the shape of a token the system needs; The One
   Synthetic Rule forbids solving it locally, which is precisely why it has been
   solved locally twice.
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
7. **"Resolution as State" is unimplemented.** DESIGN.md's organising idea — the
   3×3 dot matrix for what is held and verified, the coarse 2×2 for what is not
   — exists nowhere in `ui/src`. It should land once, as one shared component,
   rather than being invented per page.

---

## 8. Alternatives considered

| Alternative | Why rejected |
| --- | --- |
| Fold this into RFC 0004 as new phases | RFC 0004 is shipped and its decision log is closed. Its §11 says discoveries during implementation become new rows, not new scope — and seven of these are rows it already carries. A shipped RFC that keeps growing stops being reviewable. |
| Fix the five English strings and move on | It leaves the scanner unable to see the *class*, and the class has already produced two incidents: the admin navigation in §2.1 and these five. The rule is the deliverable; the strings are the symptom. |
| Delete the 94 unused keys without the gate | It restores the invariant for exactly as long as nobody removes another page. RFC 0004's own merge created most of them, in a commit that passed every gate. |
| Make the access-check simulator consult blocks in the UI | The UI would have to fetch two more endpoints and re-implement middleware ordering in TypeScript. The decision belongs where enforcement is, or the two disagree the first time either changes. |
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
- **§6 changes no route.** All five items are within pages that keep their
  paths, so no `LEGACY_REDIRECTS` entry and no change to
  `EXPECTED_COMBINATIONS`.
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
- **Each new page test asserts the page's question**, per the table in §4.3, not
  its markup. A test that pins a class name re-breaks on the `DESIGN.md`
  migration and teaches nothing.
- **The RFC 0004 gates stay green throughout** — 48 authenticated
  route/role/viewport combinations, the rendered detector at both viewports,
  `task ui:design`, and the contract gate. `EXPECTED_COMBINATIONS` does not move
  in this RFC; if it does, something in §6 changed a route and that is a review
  question.

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
| B7 | Does the `DESIGN.md` migration belong here? | **No — but the copper token decision might.** Retiring `Card` is mechanical and enormous; the missing "degraded" token is a decision the system has now improvised around twice, and every further surface that needs it improvises again. The token decision may be taken in this RFC's window; the migration may not. |

### Still open

| # | Question | Why it is open |
| --- | --- | --- |
| O1 | Does the "degraded but not refused" job get a new token, or an existing one gains a job? | The One Synthetic Rule caps the palette, and the honest options — widen copper's job list, or admit a fifth condition needs the dot pattern rather than a hue — are a `DESIGN.md` decision, not an implementation one. Two surfaces are waiting on the answer. |
| O2 | Is `AdminSbom` in the right section at all? | Phase 5 moved it from Observability to Operations because it observes nothing. If a future advisories surface lands under Security & Access, it may belong there instead — and that surface is RFC 0002's, not this one's. |

---

## 12. Implementation phases

Each phase leaves the tree green: `cargo test --workspace`,
`cargo clippy -- -D warnings`, `vue-tsc`, `vitest`, `oxlint`, and every RFC 0003
and RFC 0004 gate.

| Phase | Content | Useful on its own? |
| --- | --- | --- |
| 1 | **The gates stop over-reporting.** §4.1 and §4.2, each landing with its own cleanup — the five strings translated, the 94 keys resolved, both checks turned into invariants. | Yes, and it is first for the same reason RFC 0004's contract sweep was: every later phase is measured by these. |
| 2 | **A1 — the access-check simulator tells the truth.** Handler consults both block stores, `blocked_by` discriminator, the absence tests of §10, and the UI's interim bound replaced by the real answer. | Yes. It is the only correctness defect in the RFC. |
| 3 | **The remaining API gaps.** A2–A4, A6, A7, each a field or a small UI on an endpoint that exists; A5 if it survives its own design. | Yes, per gap. |
| 4 | **Finish the Phase 5 verdicts.** The five items in §6, one page per commit, each carrying the verdict evidence it is discharging. | Yes, per page. |
| 5 | **The page tests.** §4.3, one file per page, assertions derived from each page's stated question. | Yes — and it should arguably run alongside phase 4 rather than after it, so each re-cut lands with its regression signal. |

Phase 1 is first and alone. The other four are all measured by gates that
currently cannot see what they claim to, and finishing work under an
unobservant gate is how this RFC's contents accumulated in the first place.
