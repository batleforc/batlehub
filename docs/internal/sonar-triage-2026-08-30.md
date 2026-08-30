# SonarCloud triage — 2026-08-30

Companion to [`codeql-triage-2026-08-30.md`](./codeql-triage-2026-08-30.md), for
the SonarCloud sweep of the same date. Most of the batch was real and is fixed in
code. Three findings are false positives and are resolved in SonarCloud with the
reasoning below; one of the three must **never** be "fixed", and this note is the
record of why.

---

## Fixed in code

| Location | Rule | Fix |
| --- | --- | --- |
| `crates/core/src/services/retention/mod.rs` `run` | cognitive complexity 26 | split into `run` / `judge` / `reclaim` |
| `crates/core/src/services/retention/mod.rs` `decide` | 18 | flattened the nested `if let` chains to `is_some_and` / `zip` |
| `crates/config/src/schema/mod.rs` `tiered_policy_warnings` | 27 | split per tier into `registry_policy_warnings` / `namespace_policy_warnings` |
| `crates/core/src/entities/grant.rs` `resolve` | 21 | extracted `add_administrative_floor` |
| `crates/core/src/entities/policy.rs` `resolve` | 19 | extracted `apply_rule_overrides` |
| `crates/core/src/entities/policy.rs` `narrowing_warnings` | 17 | extracted `dropped_constraints` |
| `crates/core/src/services/authz/filter.rs` matrix test | 21 | extracted `compare_hierarchy` from the six nested loops |
| `crates/web/tests/authz_matrix.rs` coverage test | 26 | hoisted `canonical` out of the test body, split out `param_name` |
| `cli/src/cli/admin.rs` `handle_retention` | 16 | extracted `print_retention_lists` / `print_retention_notes` |
| `crates/core/src/entities/permission.rs` | redundant `'static` | dropped the annotation |
| `crates/web/src/handlers/back_office/packages/bulk.rs` | blank line after attribute | removed |
| `perf/k6/scenarios/08_…js`, `09_…js` | prefer optional chain | `data.metrics[n]?.values` |
| `tests/heavy/marketplace.sh` | `'=https'` literal ×6 | a `fetch_https` wrapper — see the second pass below |

None of the complexity refactors changed behaviour: the full workspace suite is
green, and the two refactored tests still assert exactly what they did before
(the `authz_matrix` canonicaliser is byte-identical, just relocated).

---

## Second pass — `tests/heavy/marketplace.sh`, three new HTTPS hotspots

The first pass fixed a duplicated-literal smell by hoisting
`--proto '=https' --proto-redir '=https'` into a `HTTPS_ONLY` array and splicing
`"${HTTPS_ONLY[@]}"` into each `curl`. The next analysis raised three *new*
security hotspots — "Not enforcing HTTPS here might allow for redirections to
insecure websites" — on precisely the three call sites that had just been
de-duplicated, and on none of them before. One rule's fix tripped another: the
hotspot rule reads the flags lexically at the `curl` token, and an array splat
is opaque to it.

Both rules are now satisfied by structure rather than by argument. The flags
live inline on a single `curl` inside a `fetch_https` wrapper, and the call
sites invoke the wrapper:

```bash
fetch_https() {
  curl -fsSL --proto '=https' --proto-redir '=https' "$@"
}
```

One occurrence of the literal, so nothing to de-duplicate; the flags lexically
attached to the only `curl` the rule can see, so nothing to flag; and a call
site can no longer omit half of the pair, which the array could not guarantee
either.

The hotspot also turned up a gap it had not flagged. The two
`$WEEBO_BASE_URL` release downloads (the VSIX and the JetBrains zip) were plain
`curl -fsSL` with no pinning at all — Sonar said nothing because the URL is a
variable. GitHub releases redirect to `objects.githubusercontent.com`, so they
are the same redirect-chain exposure as the IDE tarballs. Both now go through
`fetch_https`. The remaining bare `curl`s in the script address `$BASE`, the
local server over plain HTTP, and are correct as they are.

**Action:** no hotspot to resolve in SonarCloud — the next analysis clears all
three. The lesson is the transferable part: **when a fix for one rule moves an
argument away from the call it guards, re-check the rules that read that call.**
De-duplication that hides a security flag from the analyser also hides it from
the reader.

---

## FP 1 — `039_local_package_tombstones.sql:24`, "Remove this commented out code"

**Do not fix this. Editing this file breaks running deployments.**

The flagged lines are the rollback runbook inside the migration's header
comment:

```sql
-- A rollback must first
--   DELETE FROM local_packages WHERE deleted_at IS NOT NULL;
-- and only then drop the columns and the CHECK below.
```

That is documentation of a manual procedure, not disabled code. But the reason
this is more than a taste question is `crates/adapters/src/migrations.rs`:
migrations are embedded with `Migration::new()`, which **computes a SHA-384
checksum over the SQL text**, and sqlx validates that checksum against the
`_sqlx_migrations` row on every startup. Changing so much as one character of a
comment in an already-applied migration changes the checksum and makes the
migrator refuse to run with a `VersionMismatch` — on every instance that has
already applied 039, which is all of them.

Migrations are append-only for exactly this reason. The rule is worth stating in
general: **SonarCloud findings inside `crates/adapters/migrations/` are resolved,
never fixed**, unless the migration has not yet shipped.

**Action:** resolve as "won't fix" with this reasoning.

---

## FP 2 — `ui/src/components/ui/table/Table.vue`, the `<section>` scroll container, "tabindex should only be declared on interactive elements"

The element is the table's scroll container. It carries `tabindex="0"` because a
region that scrolls but cannot be reached by keyboard is unreachable content for
anyone not using a pointer — axe reports precisely that as
`scrollable-region-focusable`, and RFC 0003 §4.7 makes tables keyboard-operable
explicitly.

So the two scanners disagree, and axe is right: WCAG 2.1.1 requires the scrollable
region be operable by keyboard, and `tabindex="0"` on the container is the
standard remedy. Sonar's rule has no exception for scroll containers. Removing the
attribute to satisfy it would introduce a real, testable accessibility failure to
silence a heuristic one.

The element is a `<section>` with an accessible name, so it is not an anonymous
focus stop: it carries `role="region"` natively and announces as something worth
stopping on.

**Action:** resolve as "false positive". The code comment above the element has
been extended to name this rule, so the next person to read it does not
re-litigate it.

---

## FP 3 — `ui/design-proof/index.html:167`, "Use `<address>` or `<details>` or `<fieldset>` … instead of the group role"

The only occurrence of `role="group"` in the file is **inside a CSS comment**,
and the comment exists to explain that the code deliberately uses `<fieldset>`
instead:

```
/* `.pop` and `.segmented` are <fieldset>s — the native grouping element, rather
   than a <div role="group">. A fieldset carries three UA defaults that have to
   go for it to lay out like the div did: … */
```

The analyser matched prose. The fix Sonar is asking for is already implemented,
and the comment Sonar is flagging is the documentation of that fix.

**Action:** resolve as "false positive".

---

## Third pass — PR 138, and why the three FPs moved into `sonar-project.properties`

The PR-138 analysis (`feat/rework-role`, 60 open issues on new code) raised all
three of the false positives above **again**. Resolving an issue in the
SonarCloud UI resolves *that* issue on *that* branch; a pull request analyses its
own new code, and a file the branch touched produces new issue keys that carry no
resolution. `Table.vue` and `039_…sql` are both in this branch's diff, so both
came back — with the "resolved as false positive" note in the triage doc and, for
the table, in a code comment directly above the element.

A per-issue resolution is therefore the wrong instrument for a finding that is
*structurally* wrong about this code: it has to be re-done on every branch that
touches the file, by whoever happens to be reading the gate that day, and it
fails open when they don't. All three are now `sonar.issue.ignore.multicriteria`
entries with the reasoning inline, scoped as narrowly as the finding is:

| Rule | Scope | Why it cannot be fixed in code |
| --- | --- | --- |
| `plsql:S125` | `crates/adapters/migrations/**` | editing an applied migration changes its SHA-384 and stops every deployment (FP 1) |
| `Web:S6845` | `ui/src/components/ui/table/Table.vue` | removing `tabindex` is a real WCAG 2.1.1 failure (FP 2) |
| `shell:S5332` | `tests/heavy/**` | the harness *is* a plain-HTTP server on 127.0.0.1 (below) |

FP 3, `ui/design-proof/index.html`, is the one that was fixable and is fixed
rather than ignored: the rule was matching element syntax inside a CSS comment,
so the comment now says the same thing in prose. Nothing about the page changed.

### The new one — `shell:S5332` ×2 in `tests/heavy/authz.sh`

"Make sure that using clear-text protocols is safe here", on the two
`create_env` calls that carry the conda channel URL. It is safe here, and not
incidentally: every heavy suite boots a server on `127.0.0.1` and drives a real
package manager at it over plain HTTP. The credentials in those URLs are per-run
literals the suite generates for itself and that authorise nothing outside the
throwaway registry the run creates.

Making them HTTPS would delete what is being measured. The suites document
working *around* client refusals of plain HTTP rather than avoiding it —
`allowInsecureConnections` for NuGet, `secure-http: false` for Composer, and the
`terraform` target's TLS-terminating tap, which exists only because Terraform
will not talk to a plain-`http:` registry at all.

Scope note carried into the properties file: `tests/heavy/**` only. Product code
reaching an upstream over `http://` is a real finding.

---

## Fixed in code — PR 138

Fifty-seven of the sixty. Fifty-six were in `tests/heavy/authz.sh`, which is new
in this branch and is the first shell file large enough for the `shelldre` rules
to have much to say about.

| Rule | Count | Fix |
| --- | --- | --- |
| `shelldre:S7682` functions should end with an explicit return | 30 | a trailing `return` on every function |
| `shelldre:S7679` parameters should be named locals | 15 | `local method="$1" path="$3"` and friends, at nine call-takers |
| `shelldre:S1192` duplicated literal | 4 | `HDR_JSON`, `WIRE_403`, `IP_BLOCKS`, `WHO_DENIED` |
| `shelldre:S1481` unused local | 4 | three were used *inside a nested function* and moved into it; `simple` was genuinely dead |
| `shelldre:S131` `case` without `*)` | 1 | an explicit fall-through arm on the verb exception list |
| `rust:S3776` cognitive complexity 19 | 1 | `feed_readable`'s four `if flag {"1"} else {"0"}` became a `bit()` helper |

Two of the S7682 fixes are **not** `return 0`, and the distinction is the only
part of this batch that could have broken a test silently. `create_env` (conda)
and `bundle_install` (rubygems) are the functions whose *exit status is the
assertion*: the denial arm reads `$?` under `set +e` and the positive control
hangs a `||` off the call. A blanket `return 0` there would have made both arms
pass unconditionally — a heavy suite that reports green while asserting nothing,
which is the exact failure mode the suite's own header argues against. Both got
`return $?` and a comment saying why. `ruby_run` is a third: its status answers
the `gem list -i bundler` probe.

The S1481 findings are worth a line of their own because three of the four were
*not* dead code. `host_key`, `host` and `vsce_version` were each declared in a
phase and read inside a function nested in that phase — visible at run time,
invisible to the analyser. Moving the declaration into the function that reads it
satisfies the rule and is better code: the value is now next to its only use.
`simple` in `phase_pypi` was the real one, dead since the phase started building
its index URLs with credentials in the userinfo.

`rust:S3776` did not change what is hashed. `bit()` emits the same `"1"`/`"0"`
strings in the same order, so every digest is byte-identical; `cargo test -p
batlehub-core --lib authz` is green.

---

## Stance

Same line as the CodeQL note. A finding about a fact (a CVE, a real WCAG failure)
is not ours to dismiss. A finding that is an incorrect inference about this code
is, provided the reasoning is written down where the next person meets it — and
provided we did not quietly widen "incorrect inference" to mean "inconvenient".
Nine of the twelve complexity findings in this batch were correct and were fixed;
that ratio is the check on this stance, not the stance itself.
