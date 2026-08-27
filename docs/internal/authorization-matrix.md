# The authorization matrix

**Created:** 2026-08-26, as the follow-up the
[security survey](./security-survey-2026-08-26.md) named as its highest-value next step.
**Lives in:** `crates/web/tests/authz_matrix.rs`. Run it with `task authz-matrix`.

---

## The problem it exists to solve

The survey found the same defect in eight places: a local-registry read path that serves package
data without evaluating the registry rule chain, or without checking per-package visibility,
while the equivalent proxy-mode read enforces both. Ten of its thirteen findings were that one
defect.

It was found one ecosystem at a time — maven, nuget, terraform twice, conda, goproxy, jetbrains,
pypi — because a 14-surface sweep slices by handler family, and each slice can only report what
is in front of it. Worse, the class had **already been found and fixed once**, on the OpenVSX
VSIX route, and came back on eight others. The check is applied by convention, and convention
does not survive a new registry adapter.

## Why it is a test and not a document

The first attempt was static: index every handler, walk the call graph, record which
authorization primitives each route reaches. It failed calibration — **six of the eight known
findings came back clean.**

The reason is structural, and it is the single most useful thing in this file:

> A handler with a guarded proxy branch and an unguarded local branch *mentions* `authorize_read`
> either way. "Does this function call the chain?" answers **yes** for exactly the routes that
> are broken.

The property is per-path, not per-function. No amount of grepping fixes that; the only reliable
way to ask is to make the request and look at what comes back. A static matrix would have been
worse than none, because it would have reported a clean bill on the handlers that were already
known to be wrong.

## What each row asserts

Two axes, each isolating one gate by making the other permissive:

| Axis | Registry RBAC | Package visibility | So the only thing that can refuse is… |
| --- | --- | --- | --- |
| **A — rule chain** | anonymous granted **nothing** | `Public` (the default) | the `[registries.rbac]` rule chain |
| **B — visibility** | anonymous granted `releases:read` + `source:read` | `Internal` | `check_visibility` |

Axis B needs the `team_namespace` port wired into the fixture, because `check_visibility` returns
`Ok(())` outright when it is absent — without the port every row would pass vacuously.

### Disclosure, not status

The assertion is **not** "the response was not a 200". A search index answering `200` with an
empty result set has disclosed nothing and is correct; an artifact route answering `200` with the
bytes has disclosed everything. So a row fails when the response *contains* the package: the
fixture's artifact bytes, the package name, or its distinctive version string (`9.8.7`, chosen so
`contains` cannot collide with an unrelated version in a document).

The third signal matters more than it looks. `/info/{gem}` and `@v/list` are keyed by the URL and
their bodies are bare version lists — they disclose the package without ever naming it, and a
name-only check reports them clean.

### Every row has a positive control

The same request as a caller the policy *does* permit must show the package. Without it, a row
passes when the route 404s for an unrelated reason — a seeding mistake, a path typo — and asserts
nothing at all.

The control asserts **disclosure, not status**, for the same reason the main assertion does. An
earlier version checked only `status == 200`, which a `200` with an empty body satisfies; that
made the negative assertion vacuous on precisely the search and index routes most likely to leak.
A row whose control fails is reported as a broken test, not counted as a pass.

## The ratchet

`Expect::KnownGap` pins a route to its *current*, wrong behaviour, so the suite is green on a
tree with open findings. The ratchet turns both ways:

- a `Denied` row that regresses fails,
- a `KnownGap` row that starts refusing **also** fails, with "flip this row to `Expect::Denied`".

So fixing a handler forces its row to be updated in the same change. Deleting a row is the only
way to make a gap disappear quietly, and that is visible in review.

`Expect::NotChecked` records an axis that does not apply, with the reason — a whole-registry
document is not governed by per-package visibility, and a proxy-only route that never reads local
packages cannot be governed by a local package's visibility. Two rows were initially
misclassified as findings for exactly that second reason; stating the exemption is what keeps the
next reader from re-raising them.

## What happened next

Everything in the table below, plus the survey's findings 4–10, was fixed on
2026-08-26 — **structurally**, by moving the registry rule chain into
`LocalRegistryService`'s own read funnels rather than adding a call to each
handler. Twelve rows flipped from `KnownGap` to `Denied` in that one change,
which is the ratchet working as designed: the fix could not land without this
file being updated in the same diff.

Two things this file learned in the process are worth keeping:

- **The fixture was measuring the wrong object.** `make_local_svc` built the
  local service with its own empty `HotConfig` while production shares one `Arc`
  between both services, so before that was fixed no row here could have observed
  a local read being authorised at all — the policy a row set and the policy the
  local path read were different objects. A matrix whose fixture cannot express
  the gate reports green for the same reason a static analysis does.
- **A broken positive control hid a vacuous row.** The goproxy `@v/list` row
  seeded no `index_metadata`, and `get_go_version_list` reads each version from
  `index_metadata["Version"]` — so the list came back empty *for everyone*,
  including the permitted caller. The control caught it, which is the argument
  for controls; without one the row would have read as a pass.

**No row is `KnownGap` any more.** The two search rows were the last, and finding
11 closed them the same day. The variant is kept — with an `allow(dead_code)`,
because that is what an empty ratchet costs — since the next finding should be
pinnable on the day it is found rather than after a debate about how to keep the
suite green.

## What it found

Calibration first: the matrix independently re-found the survey's conda, jetbrains, goproxy,
pypi and terraform chain gaps, and its nuget visibility gap — which is the evidence that it
measures the right thing.

Then, new — none of these were named by any of the fourteen survey surfaces:

| Route | Axis | Note |
| --- | --- | --- |
| `cargo /registry/{path}` — **the sparse index** | chain | `serve_local_index` (`cargo/index.rs:123`) calls `get_index` with no chain. `proxy_upstream_index` directly below it carries a comment recording that this *exact* gap — "a private cargo registry's crate names and versions were readable by anyone who could reach the port" — was why the proxy path moved onto `ProxyService`. Closed there, left open here. This is the primary read path for `cargo build`. |
| `rubygems /info/{gem}` | chain | `gem_compact_info` local branch returns at `compact.rs:323` with no chain; the fall-through at `:331` runs it |
| `rubygems /versions` | chain | `serve_compact` returns the local document at `compact.rs:136`; the proxy path at `:142` runs the chain |
| `rubygems /names` | chain | same `serve_compact` branch. `/names` is deliberately unfiltered for *blocking*, which is a different question from whether an RBAC-denied caller may read it at all |

The pattern in every one: the proxy path is guarded, the local path is not. All
four became survey findings 15 and 16, and all four are now `Expect::Denied`.

It also found four routes no finding names at all, by holding rows to `Denied`
that the tree did not yet honour: `terraform /v1/modules/…/versions`,
`jetbrains /plugins/list`, `jetbrains updatePlugins.xml` and
`composer /packages.json` — the last of which took the caller identity and
dropped it while listing every package in the registry.

One row that looked like a finding was not, and is recorded as
`Expect::NotChecked` rather than deleted: `generic /generic/{path}` is a path
mirror whose coordinate is the synthetic `repo/_`, so it never reads the local
package whose visibility axis B sets. The `200` was the upstream's file and the
fixture URL merely contained the package name — which is also a reminder that
`disclosed()`'s name-matching is a heuristic, and that the exemption has to be
stated or it gets re-raised.

## Coverage, stated honestly

The matrix does not cover every route, and a row that does not exist is not a passing row. What
is covered is recorded in the table itself; what is not is:

- **Write routes.** Publish, yank, deprecate and delete are governed by `enforce_publish_policy`
  and ownership, not by the read chain. A separate matrix, not this one.
- **Back-office and front-office routes.** `require_admin` and `accessible_registries_for` are a
  different gate with a different shape.
- **The unlisted/yanked axis.** The survey asked for three axes; two are implemented. Unlisted
  filtering is exercised by the per-ecosystem suites but not systematically here.
- **Routes with `control: false`.** Their fixture cannot produce a permitted-caller hit — Maven's
  multi-file artifact key, a rubygems gemspec, generated repo metadata — so their axis assertions
  are weaker than the rest. Marked in the table rather than dropped.
- **The forge clients** (`github`, `gitlab`, `forgejo`) and several protocol/discovery documents.

Adding an ecosystem means adding rows. Until someone does, that ecosystem's read routes are
unverified — which is the honest state, and the reason this section exists rather than a claim of
completeness.
