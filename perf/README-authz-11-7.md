# RFC 0015 §11.7 — the whole-registry document measurement

[RFC 0015](../docs/rfc/0015-grants-on-the-resource-hierarchy.md) §4.4 decides
that listings **filter** rather than refuse. §11.7 asks what that costs on the
four documents that name every package in a registry, and attaches a decision to
the answer rather than leaving a number in a wiki.

This directory holds both halves. Arms 1 and 2 need no grants and ran at **phase
0b**, before a line of the RFC was implemented; **arm 3** is phase 3's exit
criterion and ran once §4.4's filter and `DocumentCache` shipped. Arm 4 was never
built — §11.7 makes it conditional on arm 3 missing, and it did not.

One consequence for anyone re-running this: arms 2 and 3 are the *same registry*
on either side of phase 3, so **this build can no longer reproduce arm 2**.
`perf-gems` is filtered *and* cached now. The arm-2 column below is a historical
record; `perf/results/authz-{s,m,l}.json` holds those runs and
`perf/results/authz-arm3-m.json` holds arm 3's.

## The arms, and why both already exist

Neither arm is a prototype. Both ship today, on different registry modes, and
the measurement is a comparison of two code paths this server already runs.

| Arm | Registry | Path | What it is |
| --- | --- | --- | --- |
| 1 | `perf-gems-proxy` (proxy) | `ProxyService::multi_package_document` | Unfiltered, shared cache. The upstream document is cached under an **identity-blind** key; only the administrative block set is applied per request. The floor. |
| 2 | `perf-gems` (local), **pre-phase-3** | `LocalRegistryService::get_rubygems_compact_versions` / `…_names` | Filtered, uncached. A loop over every package name calling `load_visible_versions(…, identity)`. The naive correct implementation §11.7 names — not a model of it. |
| 3 | `perf-gems` (local), **post-phase-3** | the same two functions | Filtered, keyed by resolved grant set. §4.4's `Readable` filter in front of the loop, `DocumentCache` in front of the whole document. What ships. |

Both registries are RubyGems and both serve `/versions` and `/names`. The mock
upstream generates its compact index to the same shape `corpus-seed` writes, so
the two arms return **byte-identical documents** at every size. That is the
property that makes the comparison mean anything: same bytes out, two ways of
arriving at them.

Two documents rather than one, because they bracket the cost. `/versions`
carries every live version of every gem; `/names` carries only the names. If the
cost were in rendering, they would differ by their size ratio. They do not (see
below), which locates it in the walk over packages instead.

## Running it

Four steps. `SIZE` is `s`, `m` or `l` and must be **the same for the corpus and
the upstream**, or the two arms describe different documents.

```bash
# 1. Postgres must be up and migrated (any batlehub server run does the migration).
task perf:authz:corpus SIZE=m        # COPY the corpus into local_packages

# 2. In its own shell — the arm-1 upstream, sized to match.
task perf:authz:upstream SIZE=m

# 3. In its own shell.
task perf:authz:server

# 4. Warm arm 1 once (its first request is the upstream fetch), then measure.
curl -s -o /dev/null -H 'Authorization: Bearer perf-user-token' \
  http://localhost:8080/proxy/perf-gems-proxy/versions
task perf:authz:run SIZE=m VUS=3 ITERATIONS=4
```

Results land in `perf/results/authz-<size>.json` and are committed: they are the
evidence behind a design decision, not scratch output.

### Two things that will silently give you the wrong number

- **Restart the server when you change corpus size.** The arm-1 metadata cache
  is in-memory and its TTL is an hour. Reseeding the corpus does not invalidate
  it, so arm 1 keeps serving the *previous* size's document and the comparison
  is between two different documents. The symptom is an arm-1 `bytes` that does
  not match arm 2's.
- **Use `ITERATIONS` for `m` and `l`, not `DURATION`.** A single arm-2 request on
  the L corpus takes minutes; a duration-bounded run finishes with zero complete
  iterations and reports nothing.

## Results, 2026-08-28

8-core CDE, Postgres 17 in a sidecar, filesystem storage, release build.
`--private-fraction 0.10`, so one package in ten is `internal` and the filter has
something to reject.

| Size | Packages | Versions | Document | Arm 1 p99 | Arm 2 p99 | Ratio |
| --- | --- | --- | --- | --- | --- | --- |
| S | 1 000 | 5 000 | `/versions` (78 KB) | 6.2 ms | 1 995 ms | **322×** |
| S | | | `/names` (17 KB) | 6.4 ms | 1 932 ms | **303×** |
| M | 25 000 | 250 000 | `/versions` (2.5 MB) | 54.8 ms | 44 177 ms | **806×** |
| M | | | `/names` (415 KB) | 7.0 ms | 43 564 ms | **6 218×** |
| L | 200 000 | 2 000 000 | `/versions` (20 MB) | 525.8 ms | 240 776 ms | **458×** |
| L | | | `/names` (3.3 MB) | 37.7 ms | 205 126 ms | **5 443×** |

The L rows are two samples at one VU — an order of magnitude, not a
distribution. S is 88 iterations and M is 12.

L's `/versions` ratio (458×) is *lower* than M's (806×) because arm 1 got worse,
not because arm 2 got better: shipping a 20 MB document costs 526 ms however it
was produced. `/names` keeps the M shape at 5 443×, because its document stays
small while the walk behind it does not.

Single-shot arm-2 latency against package count — 0.88 s at 1 000, 27.1 s at
25 000, 210.3 s at 200 000 — is **linear in the number of packages**, not in the
depth of the hierarchy. §11.7 names that distinction and calls the second
outcome a redesign.

`/names` is the sharper result and the one worth reading twice. It is a sixth
the size of `/versions` and costs the *same* to build, because the cost is the
per-package round trip, not the bytes. Any fix that makes the document smaller —
compression, pagination, a leaner line format — buys nothing at all.

The mechanism is exact: `get_rubygems_compact_versions` calls
`list_package_names` once, then `load_visible_versions` per name, which is
`backend.get_versions(registry, name)`. **One query per package** — 25 001 at M,
200 001 at L.

## Arm 3, and what it settles

Same corpus, same harness, same registries — `perf-gems` after phase 3.

| Document | Arm 1 p99 | Arm 3 p99 | Ratio | (arm 2 was) |
| --- | --- | --- | --- | --- |
| `/versions` (2.5 MB) | 84.4 ms | 86.0 ms | **1.0×** | 44 177 ms |
| `/names` (415 KB) | 40.4 ms | 37.3 ms | **0.9×** | 43 564 ms |

**Threshold: within 20 % of arm 1 at size M. It passes.** The two are
indistinguishable, and `/names` comes out marginally faster — noise, not a win.
Arm 1's own p99 is higher here than in the phase 0b table (84 ms against 55 ms)
because the two arms now contend for the same server rather than one of them
spending the run blocked on 44-second requests.

The premise underneath arm 3 was checked directly rather than assumed. Its whole
viability rests on *callers sharing a set sharing an entry*, so: one caller
warmed `/versions` (28.4 s cold, 22.9 ms warm), and a **different** caller
resolving to the same grant set was served in 41 ms. A hit. That is §11.7's
"number of distinct grant sets exercised", confirmed on the cheapest case.

Invalidation is by **generation**, not TTL — a per-registry counter bumped by
every publish, yank and unyank, read *before* the document is built so a publish
landing mid-render invalidates the result instead of being stamped onto bytes
that predate it. A TTL alone would have reintroduced conda's `repodata.json.zst`
bug, where a key a publish did not change served pre-publish bytes indefinitely.

## The second number: what resolution costs

§11.7 is one question with two numbers, and this is the other one — the
*gating* one, since "failing the resolution threshold sends the storage design
back before phase 4 builds the `policy` table on it".

```bash
task perf:authz:corpus SIZE=m       # seeds package-tier grants too
task perf:authz:upstream SIZE=m     # separate shell
task perf:authz:server              # separate shell
task perf:authz:resolution SIZE=m
```

A single-coordinate read on the smallest document the registry serves
(`/info/{gem}`), so latency is dominated by authorization rather than
serialisation. Two arms: a package carrying a package-tier grant row, and its
neighbour with none — the second being the common case, and the one a corpus
with no grants would mistake for the whole story.

| Corpus | rows | arm | p50 | p99 |
| --- | --- | --- | --- | --- |
| M | 250 000 | granted | 7.86 ms | 27.40 ms |
| M | 250 000 | ungranted | 7.77 ms | 27.65 ms |
| S | 5 000 | granted | 7.66 ms | 33.45 ms |
| S | 5 000 | ungranted | 7.64 ms | 32.54 ms |

**Threshold: 2 ms added at p99 on M. It passes.** The granted/ungranted delta is
−0.25 ms — within noise, and negative, which is what noise looks like.

**And it does not scale with the estate.** p50 is 7.66 ms at 5 000 rows and
7.86 ms at 250 000: flat across a 250× difference, with the p99 spread going the
wrong way for a size effect. §11.7 asks whether the cost is "bounded by the
hierarchy's depth (four) or by the estate's size"; it is depth.

That is the opposite of the document result above, and the contrast is the point:
a document costs one query *per package*, while resolving one coordinate is one
query however many packages exist.

## What it decides

§11.7's phase 0b branch: *"If arm 2 at size M is close to arm 1, the grant-set
cache key is an optimisation and phase 3 can ship without it. If it is an order
of magnitude worse, the cache key is load-bearing and phase 3 has to be designed
around it from the first commit."*

806× at M is nearly three orders of magnitude. **The cache key is load-bearing.**
See RFC 0015 §13.2 for what that changed about phase 3.

And §11.7's phase 3 branch, now that arm 3 has run: *"Arm 3 passes. Filtering
applies everywhere, the grant-set cache key ships with phase 3, and open question
5 closes."* All three happened. RFC 0015 §13.5 has the entry.
