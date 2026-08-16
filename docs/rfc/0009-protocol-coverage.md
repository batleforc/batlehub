# RFC 0009 — Every endpoint the client actually calls

| Field       | Value                                                        |
| ----------- | ------------------------------------------------------------ |
| Status      | **Draft**                                                     |
| Author      | Max Batleforc <maxleriche.60@gmail.com>                       |
| Co-author   | Claude Opus 5 (1M context) <noreply@anthropic.com>            |
| Created     | 2026-08-16                                                    |
| Supersedes  | —                                                             |
| Touches     | `crates/core`, `crates/adapters`, `crates/web`, `server`, `docs` |

---

## 1. Summary

RFC 0006 closed a gap it found on its way past: `openvsx` and
`vscode-marketplace` cached VSIX bytes but served none of the gallery routes an
editor calls, so BatleHub could not be an editor's marketplace at all (§13.1-bis).
The survey that followed — `docs/internal/registry-api-coverage.md` — asked the
same question of the other nineteen registry kinds and found four more of the
same class, plus a long tail.

The headline is not the list. It is that in every case the *tests passed*, the
*docs described the feature as working*, and nobody had written down what the
client actually asks for. `npm audit` is served at
`/-/npm/v1/audit/bulk`; npm calls `/-/npm/v1/security/advisories/bulk`. Four
tests assert the first path. They exercise our route, and npm's route has never
been typed anywhere in this repository.

This RFC fixes all of it, and adds two mechanisms so that the next endpoint we
invent instead of implement fails the build:

1. a **protocol conformance fixture** per ecosystem — the client's literal paths,
   asserted to route — with a ratchet list of the ones not yet served;
2. **generated endpoint reference tables** in `docs/registries/*.md`, from
   `ui/openapi.json`, with a drift check, the way `docs:roadmap` and
   `docs:listing-coverage` already work.

And it states the obligation that every new endpoint inherits: **an endpoint that
names a version is a listing, and RFC 0006 says listings are filtered.** Adding
the RubyGems compact index without filtering it would re-open, for the default
Ruby client, the exact hole RFC 0006 spent eight phases closing.

### Before / after

```text
# today
$ npm audit                       → 404 from BatleHub, 404 from the forward
$ bundle install                  → falls back to specs.4.8.gz, which is NOT filtered
                                    → a blocked version resolves, then 403s on download
$ terraform init                  → network_mirror per our docs: 404 on every provider
$ go mod download                 → still dials sum.golang.org directly
$ conda install                   → repodata.json.zst 404s, full uncompressed transfer

# after
$ npm audit                       → works, on npm's own paths
$ bundle install                  → compact index, filtered, blocked versions never offered
$ terraform init                  → network mirror (path-routed) or registry protocol (host-routed)
$ go mod download                 → checksum database proxied, no egress needed
$ conda install                   → .zst served, filtered before compression
```

---

## 2. Motivation

### 2.1 The failure is always the same shape

Five findings, one mechanism:

| Finding | What we built | What the client calls |
| --- | --- | --- |
| npm audit | `/-/npm/v1/audit/{quick,bulk}` | `/-/npm/v1/security/audits/quick`, `/-/npm/v1/security/advisories/bulk` |
| Terraform | the registry protocol at `/v1/…` | a network mirror, per our own docs |
| RubyGems | `specs.4.8.gz` + the JSON APIs | the compact index — `/versions`, `/info/{gem}` |
| Go | modules only | modules **and** `/sumdb/…` |
| conda | `repodata.json` | `repodata.json.zst` first |

In each row, somebody read the protocol, built something adjacent to it, and
wrote a test for what they built. The test is the problem: a test written from
the implementation cannot discover that the implementation answers the wrong
question. `crates/web/tests/proxy_npm_edge_cases.rs:76` and
`vuln_proxy_endpoints.rs:38` both POST to `/-/npm/v1/audit/…` and assert a
sensible response. Both pass. Both are wrong in the only way that matters.

### 2.2 RubyGems is not a coverage gap, it is a live block leak

The other four degrade a feature. This one breaks a guarantee.

`RegistryKind::listing_filter()` marks the RubyGems Marshal indexes
`Unsupported`, with the reason: hiding a version from a Marshal index would need
a Marshal encoder in Rust, "to hide what the JSON APIs already hide for every
client released this decade".

The reasoning is correct about the JSON APIs and wrong about which API the
default client reaches. Bundler resolves from the compact index; failing that,
the dependency API; failing that, `specs.4.8.gz`. We serve neither of the first
two. So **every `bundle install` lands on the one index we do not filter.** A
blocked gem version is offered to the resolver, chosen, written to
`Gemfile.lock`, and refused at download — the mid-resolve failure RFC 0006 §2
opens by describing.

The fix is not a Marshal encoder. The compact index is three plain-text
documents, and `DocumentBody::Text` already exists for exactly this
(`crates/core/src/ports/registry/client.rs:44`). The `Unsupported` reason string
becomes true once the compact index is served, instead of merely defensible.

### 2.3 The docs are the third witness, and they also drifted

Every `docs/registries/*.md` carries a hand-maintained "Endpoint reference"
table. `rubygems.md:116-126` lists six routes and no compact index.
`terraform.md:19-30` configures a protocol the code does not implement.
`npm.md:3` promises `npm audit` works. `use/vulnerability-proxy.md:80-85`
publishes the wrong paths as a reference.

Three independent records of the API surface — routes, tests, docs — and all
three agreed with each other and disagreed with the client. That is what a
hand-maintained table buys: consistency with itself.

---

## 3. Goals / non-goals

**Goals.** Every finding in `docs/internal/registry-api-coverage.md` fixed —
§3.1 through §3.5, the §2 Terraform download gate, and the whole §4 long tail.
Two mechanisms that make the class of failure detectable. Docs regenerated from
the code rather than maintained beside it.

**Non-goals.**

- **GitHub Packages** (`npm.pkg.github.com`, `maven.pkg.github.com`, `ghcr.io`).
  The survey named its absence; the resolution is that it is not a `github`-kind
  gap. `npm.pkg.github.com` speaks npm, and the way to proxy it is an `npm`
  registry with that upstream — which works today. The `github` kind is a
  *release* mirror, which is why `supports_local_mode()` is false for it. Stated
  as a non-goal so the survey row is closed rather than carried.
- **The legacy NuGet OData `/v2/` API**, Composer 1's `providers-url`, and the
  cargo git index. Three protocols no current client selects; adding them is
  surface without a caller, which is how we got here.
- **Correctness of documents we already serve.** RFC 0006 owns that. This RFC
  adds endpoints and the obligation that new ones are filtered; it does not
  re-audit existing filters.
- **Rate limiting, pagination and conditional requests** on the new endpoints
  beyond what the existing helpers already provide.

---

## 4. The obligation every new endpoint inherits

Before the per-ecosystem design, the rule that constrains all of it.

RFC 0006's contract is enforced by three checks (§13.7): `listing_filter()` is
an exhaustive match over `RegistryKind`, `blocking::strip` is exhaustive for the
same reason, and `every_advertised_filter_is_reachable_from_dispatch` checks the
two against each other. Those checks are exhaustive over **kinds**, not over
**documents**. A kind that already answers `Filtered` for one document can grow a
second, unfiltered one and nothing complains.

This RFC adds fourteen endpoints that name versions. Each one is a listing:

| New endpoint | Kind | Shape | Filter |
| --- | --- | --- | --- |
| `/versions` (compact index) | rubygems | whole-registry | `dispatch_multi` + snapshot |
| `/info/{gem}` | rubygems | per-package | `dispatch`, new `DocumentKind` |
| `/api/v1/dependencies` | rubygems | multi-package, query-selected | `dispatch_multi` |
| network-mirror `index.json` | terraform | per-provider | `dispatch`, new `DocumentKind` |
| network-mirror `{version}.json` | terraform | single-version | repair by composition |
| `channeldata.json` | conda | whole-registry | `dispatch_multi` |
| `/pypi/{name}/json` | pypi | per-package | `dispatch`, new `DocumentKind` |
| `/-/package/{pkg}/dist-tags` | npm | per-package | must not name a blocked version |
| `/-/v1/search` | npm | multi-package | `dispatch_multi` |
| `/api/v1/crates?q=` | cargo | multi-package | `dispatch_multi` |
| `autocomplete` | nuget | multi-package | `dispatch_multi` |
| `search.json`, `list.json` | composer | multi-package | `dispatch_multi` |
| `api/{namespace}` | openvsx | multi-package | filtered at the `vsx/source.rs` chokepoint |
| `/v1/modules/{…}/{version}` | terraform | single-version | 404 when blocked |

Two of these need machinery that does not exist yet:

- **`dispatch_multi` handles exactly one kind today** — conda
  (`blocking/mod.rs:210`), with `other =>` falling through to a
  `tracing::warn!`. Six kinds above need it. The warn arm becomes a real
  dispatch table, and the same 30-second `blocks:{registry}` snapshot (RFC 0006
  §13.5) serves all of them.
- **Search results are ranked, not just filtered.** Removing a package from a
  page of 20 leaves 19, and the client paginates by offset, so removal silently
  shortens result pages. Search filters remove *versions* from each hit and drop
  a hit only when every version is blocked; the total count is adjusted to match.

**Enforcement.** `every_advertised_filter_is_reachable_from_dispatch` is
extended from kinds to `(kind, DocumentKind)` pairs, and `listing_filter()`'s
`ListingDocument` gains the `DocumentKind` it describes. A kind that serves a
document not named in its `listing_filter()` slice fails the test. That is the
check that would have caught RubyGems: `specs.4.8.gz` was named and marked
`Unsupported`, but nothing recorded that the *document Bundler actually reads*
was neither served nor named.

---

## 5. Mechanism 1 — protocol conformance fixtures

A test whose paths are copied from the client, not from us.

`crates/web/tests/protocol_conformance.rs`: one table per ecosystem of literal
request lines, each with the source it was taken from, run against a full
in-process app (`common::make_app`) and asserted to **route** — any status but
404-from-no-route. It asserts nothing about the body. It is a routing conformance
test, and routing is exactly what we got wrong five times.

```rust
const NPM: &[Conformance] = &[
    // npm/lib/commands/audit.js — the two audit endpoints
    Conformance::post("/-/npm/v1/security/audits/quick"),
    Conformance::post("/-/npm/v1/security/advisories/bulk"),
    Conformance::get("/-/v1/search?text=express"),
    Conformance::get("/-/package/express/dist-tags"),
    Conformance::get("/-/whoami"),
    Conformance::get("/-/ping"),
    …
];
```

Route registration order is a live hazard in this codebase — `lib.rs:775-781`
carries a paragraph about `api/{namespace}/{extension}` swallowing
`api/plugins/{id}`, and `:739-743` about `vscode/item` being taken for
`{name}/{version}`. A conformance table is the only thing that catches a
regression there, because a misordered route returns a *wrong handler's* answer,
not an error.

**The ratchet.** Landing the table red would leave the tree red, which this
repository does not do. So each entry carries an optional `not_yet: &'static str`
naming the phase that will serve it:

```rust
Conformance::get("/versions").not_yet("RFC 0009 §6.3 — phase 2"),
```

`not_yet` entries assert the *opposite* — that the path 404s. When a phase lands
its endpoint, the test fails until `not_yet` is deleted. The list only shrinks,
and it is a published inventory of what we do not serve, which is the artifact
the survey had to be written by hand to produce.

---

## 6. Mechanism 2 — generated endpoint references

`utoipa` already produces `ui/openapi.json`, every proxy handler is already
tagged (`tag = "proxy/npm"`), and `crates/web/tests/openapi_contract.rs` already
fails on a `200` without a `body`. The endpoint tables in
`docs/registries/*.md` are the one place that information is retyped by hand.

`task docs:endpoints` renders, per registry page, the method/path/summary table
for that page's tag, between markers:

```markdown
<!-- generated:endpoints:proxy/npm -->
| Method | Path | Description |
…
<!-- /generated:endpoints -->
```

`task docs:endpoints:check` fails on drift, wired into `task docs:design`
alongside the existing `docs:listing-coverage:check` and `docs:roadmap` checks.
Prose around the block stays hand-written — the generated part is the inventory,
not the explanation.

This is the mechanism that makes `rubygems.md` unable to advertise six routes
while nine exist, and it costs one Taskfile target because the spec is already
built.

---

## 7. Detailed design, per ecosystem

### 7.1 npm — the audit paths, and the rest of the CLI

**The fix.** Register npm's real paths and forward to the matching upstream path:

| Route | Upstream |
| --- | --- |
| `POST …/-/npm/v1/security/audits/quick` | `{upstream}/-/npm/v1/security/audits/quick` |
| `POST …/-/npm/v1/security/advisories/bulk` | `{upstream}/-/npm/v1/security/advisories/bulk` |

`forward_npm_audit` (`npm/read.rs:270`) currently interpolates one template
(`:284`); it takes the full upstream path instead. The two existing routes stay
as aliases — they cost nothing, some deployment may have scripted them, and
removing them is a separate decision from fixing the bug.

The four tests that assert the old paths (`proxy_npm_edge_cases.rs:76,88,101,183`,
`vuln_proxy_endpoints.rs:38,49,61,138,714`) keep working through the aliases, and
gain siblings on the real paths. `docs/registries/npm.md:123` and
`docs/use/vulnerability-proxy.md:80-85` are corrected.

**The rest.** `GET /-/v1/search` (filtered per §4), `GET`/`PUT`/`DELETE
/-/package/{pkg}/dist-tags[/{tag}]`, `GET /-/whoami`, `GET /-/ping`, `DELETE
/{pkg}/-rev/{rev}` (unpublish, gated by the same ownership rules as publish),
and the abbreviated packument (`Accept: application/vnd.npm.install-v1+json`) as
a `DocumentKind` alongside the full one, for the same cache-collision reason PyPI
needed two (RFC 0006 §13.4).

`dist-tags` is the sharp one: it is a map of tag → version, and a tag naming a
blocked version is a listing that hands the client a version it cannot have.
`best_latest` (`blocking/mod.rs:474`) already computes the repair.

npm login (`PUT /-/user/org.couchdb.user:{u}`, `POST /-/v1/login`) is **not**
added — BatleHub issues its own tokens and has an OIDC path; a second credential
issuer on the npm protocol is a security surface, not a coverage gap.

### 7.2 Terraform — both protocols, and the download gate

The decision taken for this RFC: **both**. They are not alternatives.

**Network mirror** (works under path routing, providers only):

```
GET /proxy/{reg}/{hostname}/{namespace}/{type}/index.json
    → { "versions": { "1.0.0": {}, "1.1.0": {} } }
GET /proxy/{reg}/{hostname}/{namespace}/{type}/{version}.json
    → { "archives": { "linux_amd64": { "url": …, "hashes": ["h1:…"] } } }
```

`url` is relative to the `{version}.json` document, so it points back at our own
artifact route by construction — the `X-Terraform-Get` problem does not arise on
this protocol at all. `index.json` is a per-provider listing and is filtered;
`{version}.json` names one version and is a 404 when that version is blocked.

**Registry protocol** (requires host routing, modules and providers):

```
GET https://{host}/.well-known/terraform.json
    → { "modules.v1": "/v1/modules/", "providers.v1": "/v1/providers/" }
```

Discovery is host-rooted by the protocol, so it is served only when the request
arrives on a host bound to exactly one registry — `host_routed_registry(req)`
(`middleware/host_routing.rs:50`) returns `Some`. On a path-routed request the
route returns 404 with a body naming the constraint, because a
`.well-known` document that cannot say *which* registry it describes is worse
than none. RFC 0001 shipped, so this is configuration, not new machinery.

That also makes the source addresses legal: `tf.example.com/myorg/mycloud` is
the three segments Terraform requires, where
`batlehub.example.com/proxy/internal-tf/myorg/mycloud` (`terraform.md:135`) is
five and is rejected before a request is made.

Completing the registry protocol needs, beyond discovery:

- `GET /v1/modules/{ns}/{name}/{provider}/{version}` — module metadata, missing
  today; 404 when the version is blocked.
- `signing_keys` in the provider download response. Terraform verifies the
  provider zip's GPG signature against the keys the registry declares and
  refuses the provider when they are absent — so a private provider is currently
  unusable even with everything else correct. `crates/adapters/src/repo/openpgp.rs`
  already signs deb/rpm/pacman metadata with Ed25519; the provider upload flow
  gains an optional detached `.sig` plus the registry's public key, surfaced here.
- Module and provider list/search endpoints (`/v1/modules`, `/v1/providers`),
  filtered as multi-package listings.

**The download gate** (survey §2, RFC 0006 §13.6). `…/{version}/download/{os}/{arch}`
in proxy mode resolves the upstream's `X-Terraform-Get` and hands the client that
URL, so the bytes never pass through the proxy and no rule runs on them — a
blocked provider version is still downloadable by anyone who can read the
listing. The header is rewritten to our own artifact route, which already exists
(`terraform/providers/read.rs:168`) and already goes through the gate in local
mode. This is the one item in this RFC that closes a *policy* hole rather than a
coverage one.

**Docs.** `docs/registries/terraform.md` is rewritten around the two protocols,
with the host-routing prerequisite stated where an operator reads it, and both
broken snippets (`:19-30` network mirror against registry-protocol routes,
`:82`/`:135` five-segment sources) replaced.

### 7.3 RubyGems — the compact index

Three documents, all `text/plain`, all `DocumentBody::Text`:

```
GET /versions          → gem versions md5      (whole registry, append-only)
GET /info/{gem}        → version deps checksum  (per package)
GET /names             → one gem name per line  (whole registry)
```

`/versions` and `/names` are whole-registry documents and filter through
`dispatch_multi` against the `blocks:{registry}` snapshot, exactly as conda's
`repodata.json` does (RFC 0006 §13.5) — including the same up-to-30-second lag,
which is acceptable for the same reason and must be documented for the same
reason.

`/info/{gem}` is per-package: a new `DocumentKind::COMPACT_INFO`, a
line-oriented filter in `blocking/rubygems.rs` next to the existing ones, and a
line-oriented parse — the same shape as the PyPI simple-page filter
(`blocking/pypi.rs:73`, "line-oriented rather than a DOM rewrite").

**One interaction worth stating.** Bundler fetches `/versions` incrementally with
HTTP `Range` and validates the result against the digest the server declares. A
filtered `/versions` changes when a block is added, so a client with a cached
prefix sees a digest mismatch and refetches the whole document. That is a
bandwidth cost on block changes, not a correctness problem, and it is the
behaviour Bundler already has for the upstream's own rewrites. We serve the
digest of what we send, never the upstream's.

`GET /api/v1/dependencies?gems=a,b,c` is added alongside — Bundler's middle
fallback, cheap once the compact index machinery exists, and it keeps older
Bundlers off `specs.4.8.gz`.

`listing_filter()`'s RubyGems entry gains the three compact-index documents as
`Filtered`. The Marshal entry stays `Unsupported`, and its reason string is
rewritten to be true: the JSON *and compact* APIs hide what it cannot, and no
current client reads it.

### 7.4 Go — the checksum database

```
GET /proxy/{registry}/sumdb/{sumdb-name}/{path:.*}
```

A byte passthrough, in the shape of the existing vuln passthrough
(`goproxy/vuln.rs:40-73`): `require_registry_type`, resolve the configured sumdb
base, `forward_get`. No filtering — the sumdb is a signed transparency log,
editing it is neither possible nor wanted, and `DocumentBody` deliberately has no
binary variant for exactly this class of document
(`ports/registry/client.rs:36-39`).

Configuration gains a `sumdb` field per goproxy registry defaulting to
`sum.golang.org`, and `""` to disable — a private-module-only registry has no
sumdb and should 404 rather than proxy a lookup that leaks module paths upstream.

`docs/registries/goproxy.md:31` currently documents `GONOSUMDB` for private
modules; it gains the public case, which is that no `GONOSUMCHECK` is needed at
all once the sumdb is proxied.

### 7.5 conda — compressed repodata and channeldata

`repodata.json.zst` and `repodata.json.bz2` as compressed encodings of the
document we already synthesise and filter. Filtering runs on the JSON, then the
result is compressed — so RFC 0006's guarantee carries over with no new filter.
Both are cached under their own artifact key so the compression is paid once per
TTL, not per request.

The `{filename}` route regex
(`conda.rs:220`, `{filename:.+\.(?:tar\.bz2|conda)}`) currently means a `.zst`
request falls through the whole route table rather than reaching a handler. The
two new routes are literal and registered before it.

`channeldata.json` is a whole-channel document used by `conda search` for
cross-platform discovery: a `dispatch_multi` filter next to
`conda::strip_repodata` (`blocking/mod.rs:210`).

### 7.6 The long tail

| Kind | Added |
| --- | --- |
| **cargo** | `GET /api/v1/crates?q=` (search, filtered); `PUT`/`DELETE /api/v1/crates/{name}/owners` — ownership is readable but not manageable today, and the local-registry ownership model already exists behind the admin API |
| **NuGet** | `SearchAutocompleteService` (+ the `autocomplete` route, filtered); `SymbolPackagePublish/4.9.0` (+ `.snupkg` accept — `nuget push` of symbols currently fails silently); `ReportAbuseUriTemplate` and `PackageDetailsUriTemplate` pointed at our own UI rather than left to fall back to nuget.org links, which for a private registry is a small information leak in CLI output |
| **Composer** | `GET /search.json` — the adapter already calls it upstream (`composer/impl_registry.rs:226`) with no route exposing it; `GET /list.json` |
| **PyPI** | `GET /pypi/{name}/json` and `/pypi/{name}/{version}/json` (filtered; a new `DocumentKind`); **PEP 658** `.metadata` siblings with `data-dist-info-metadata` / `data-core-metadata` on the simple page — uv and pip use them to resolve without downloading wheels, and their absence is a silent slowdown proportional to how much anyone uses the PyPI proxy |
| **openvsx / vscode-marketplace** | `sortBy`/`sortOrder` honoured rather than parsed and ignored; `filterType 12` (exclude-with-flags); the OpenVSX namespace endpoints `api/{namespace}`, `api/{ns}/{ext}/reviews`, `api/-/query`, `api/version`; and the OpenVSX publish API `api/-/publish` / `api/user/publish`, which is what `ovsx publish` calls — we accept only `PUT …/{ext}/{version}/vsix`, which no tool sends |

Everything in this table filters through the same obligation in §4. The openvsx
additions filter at the `vsx/source.rs` chokepoint, where the existing gallery
filter already sits (RFC 0006 §13.1-bis), rather than through `strip` — the
entries render into two protocols and a `(kind, document, package)` signature
cannot address them.

---

## 8. What this does not change

The seams RFC 0006 built are sufficient for all of the above, and this RFC adds
no new ones:

- `VersionDocument`/`DocumentBody` gain no variants. Every new document is JSON
  or text. The deliberate absence of a binary variant
  (`ports/registry/client.rs:36-39`) is preserved, and the two documents that
  might have tempted one — the Go sumdb and conda's compressed repodata — are a
  passthrough and a transport encoding respectively, neither of which is a
  document the filter sees.
- `ProxyService::handle`'s authorise → resolve → filter → stream order is
  unchanged.
- `validate_coordinate` / `ensure_safe_key` remain the two funnels, and every new
  handler that builds a storage key validates at the edge for a clean `400`, per
  `CLAUDE.md`'s adding-a-registry rule 3. The Terraform network mirror is the one
  new path that takes a `{hostname}` segment from the request, and it is
  validated against the configured registry rather than trusted.

---

## 9. Phasing

Each phase leaves the tree green: builds, clippy clean, tests pass, and the
conformance ratchet strictly shrinks.

| Phase | Content | Closes |
| --- | --- | --- |
| 0 | Both mechanisms. `protocol_conformance.rs` with the full client-path inventory and `not_yet` entries for everything unserved; `task docs:endpoints` + `:check` wired into `docs:design`; `listing_filter()`'s `ListingDocument` gains its `DocumentKind` and the reachability test extends to pairs. No behaviour change. | — |
| 1 | npm audit paths + forward fix + both docs pages. | §7.1 (audit) |
| 2 | RubyGems compact index, `/names`, `/info/{gem}`, dependency API; `dispatch_multi` generalised from conda-only to a real dispatch table; `listing_filter()` RubyGems entry rewritten. | §7.3 |
| 3 | conda `.zst`/`.bz2`/`channeldata.json`. | §7.5 |
| 4 | Go sumdb passthrough + config field. | §7.4 |
| 5 | Terraform: network mirror, `.well-known` discovery, module metadata, `signing_keys`, `X-Terraform-Get` rewrite, list/search, docs rewritten. | §7.2 |
| 6 | The long tail: cargo, NuGet, Composer, PyPI, openvsx. | §7.6 |
| 7 | npm's remaining CLI surface — search, dist-tags, whoami, ping, unpublish, abbreviated packument. | §7.1 (rest) |
| 8 | Docs: regenerate every endpoint table, refresh `ui/openapi.json` + the TypeScript client, mark the survey closed, add the conda snapshot-lag note for `/versions` and `channeldata.json`. | — |

Phase 0 is a prerequisite for the ratchet but not for the fixes; phases 1–4 are
independent of each other and each closes one finding. Phase 5 is the largest and
the only one that touches configuration and host routing. Phase 2 is the one that
closes a live policy hole, and phase 5 closes the other.

**Ordering rationale.** Phase 1 first because it is the smallest diff for a
feature we document as working. Phase 2 second because it is a block leak, not a
coverage gap. Phases 3 and 4 are mechanical. Phase 5 is deferred not because it
matters least but because it is the only one with a design decision already
spent, and it wants the conformance harness from phase 0 to have proven itself on
four easier ecosystems first.

---

## 10. Risks

**The conformance table is only as good as its sources.** It encodes what we
believe the client calls. If the belief is wrong, the table is confidently wrong
in the same way the old tests were — with a green build. Mitigation: every entry
carries a comment naming where the path was read from (client source file,
protocol spec section), so a reviewer can check the claim rather than the code.
The paths in this RFC's phase 1 and 2 should be verified against a real `npm
audit` and `bundle install` transcript before those phases land — the survey
verified our side against the route table, not the client's side against a
client.

**Generated docs tables lose their editorial voice.** A generated inventory is
worse prose than a hand-picked table. Mitigation: only the table is generated;
the surrounding explanation, which is the part worth reading, stays hand-written,
and the marker comments make the boundary obvious to the next editor.

**Six kinds newly using `dispatch_multi` all pay the snapshot.** conda's
30-second `blocks:{registry}` lag becomes RubyGems', cargo search's, npm search's
and Composer's too. That is the documented trade (RFC 0006 §13.5) — the listing
lags, the download gate does not — but it now applies to the default install path
of two more ecosystems, and `docs/guide/admin-policies.md` must say so rather
than describing it as a conda quirk.

**Aliased npm audit routes are surface with no caller.** §3 calls that an
anti-pattern and §7.1 then does it. The justification is that they are already
shipped and removing them is a breaking change to any deployment that scripted
them; they are marked deprecated in the generated table and removed in a later
release, not kept indefinitely.

---

## 11. Open questions

1. **Does the Terraform network mirror need per-registry `{hostname}` validation,
   or is echoing the client's segment sufficient?** The mirror protocol puts the
   *original* registry hostname in the path so one mirror can serve providers
   from several registries. We have one upstream per registry, so the segment is
   redundant — but silently ignoring it would serve `registry.terraform.io`
   providers under an `example.com` path. Proposed: validate against the
   configured upstream host and 404 on mismatch.
2. **Should `/-/v1/search` and `cargo search` proxy upstream or answer from our
   own index?** Proxying is faithful and leaks the query; answering locally is
   private but returns only what we have cached, which for a proxy-mode registry
   is a confusing subset. Proposed: proxy in proxy mode, local in local mode,
   merge in hybrid — the same rule the rest of the codebase already follows.
3. **Symbol packages (`.snupkg`) need a storage key shape.** They are a second
   artifact for an existing coordinate, so `PackageId::with_artifact("snupkg")`
   fits, but symbol *servers* are conventionally separate. Proposed: same
   coordinate, `snupkg` sub-coordinate, no separate symbol server.
