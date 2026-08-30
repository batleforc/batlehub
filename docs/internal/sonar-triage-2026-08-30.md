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
| `tests/heavy/marketplace.sh` | `'=https'` literal ×6 | hoisted to a `HTTPS_ONLY` array constant |

None of the complexity refactors changed behaviour: the full workspace suite is
green, and the two refactored tests still assert exactly what they did before
(the `authz_matrix` canonicaliser is byte-identical, just relocated).

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

## FP 2 — `ui/src/components/ui/table/Table.vue:27`, "tabindex should only be declared on interactive elements"

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

## Stance

Same line as the CodeQL note. A finding about a fact (a CVE, a real WCAG failure)
is not ours to dismiss. A finding that is an incorrect inference about this code
is, provided the reasoning is written down where the next person meets it — and
provided we did not quietly widen "incorrect inference" to mean "inconvenient".
Nine of the twelve complexity findings in this batch were correct and were fixed;
that ratio is the check on this stance, not the stance itself.
