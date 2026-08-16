# Registry protocol coverage — what each client asks for, and what we answer

**Status:** survey, 2026-08-16. Not published (`docs/internal/` is `srcExclude`d).

RFC 0006 §13.1-bis found that `openvsx`/`vscode-marketplace` cached VSIX *bytes*
but exposed none of the gallery routes an editor actually calls — a feature gap
that read as a design decision because nothing in the tree stated the protocol
surface anywhere. This document is the generalisation of that finding: for every
`RegistryKind`, the endpoints a real client calls, the ones BatleHub serves, and
the difference.

**Method.** The "served" column is the route table — every `#[get]`/`#[post]`/
`#[put]`/`#[delete]` under `crates/web/src/handlers/proxy/`, cross-checked against
the registration order in `crates/web/src/lib.rs:628-795`. The "needed" column is
the published protocol for each ecosystem's default client. Claims about our own
behaviour are anchored to a file; claims about client behaviour are not, and are
the part of this document most worth re-checking before acting on it.

This is a *coverage* survey. It says nothing about whether what we do serve is
correct — RFC 0006 covers listing correctness, and the security constraints in
`CLAUDE.md` cover the storage-key edges.

---

## 1. Verdict per kind

| Kind | Verdict | The gap in one line |
| --- | --- | --- |
| `npm` | ⚠️ **broken feature** | `npm audit` is served on a path npm does not call (§3.1). Plus no search / dist-tags / whoami / unpublish. |
| `terraform` | ⚠️ **documented setup cannot work** | No service discovery, and the docs configure a *network mirror* while the code implements the *registry protocol* (§3.2). |
| `rubygems` | ⚠️ **blocks leak to the default client** | No compact index and no dependency API, so Bundler falls back to the one index we do not filter (§3.3). |
| `goproxy` | ⚠️ gap | No `/sumdb/` proxying — the checksum-database half of `GOPROXY` is absent (§3.4). |
| `conda` | ◐ degraded | Only uncompressed `repodata.json`; modern conda/mamba ask for `.zst` first and get a 404 (§3.5). |
| `cargo` | ◐ partial | Index + publish + yank complete; no `cargo search`, owners is read-only (§4.1). |
| `nuget` | ◐ partial | Service index omits autocomplete and symbol publish; paged registrations pass through (§4.2). |
| `composer` | ◐ partial | Composer 2 path complete; no `search.json`/`list.json` route (§4.3). |
| `pypi` | ✅ complete for pip | Simple index (HTML + PEP 691), download, upload. No JSON API — optional (§4.4). |
| `maven` | ✅ complete | Path-addressed `GET`/`PUT` on `maven2/**` covers the whole protocol (§4.5). |
| `openvsx` / `vscode-marketplace` | ✅ closed by RFC 0006 | Gallery + OpenVSX REST now served; residual gaps are secondary (§4.6). |
| `jetbrains-marketplace` | ✅ broad | 17 routes covering XML, IDE and files APIs (§4.7). |
| `deb` / `rpm` / `pacman` | ✅ complete | Indexes generated *and* OpenPGP-signed (§4.8). |
| `github` / `gitlab` / `forgejo` | ✅ complete for release consumption | Wildcards over `api/v4/**` and `api/packages/**` (§4.9). |
| `jetbrains` / `generic` | ✅ n/a | Path mirrors — a wildcard is the whole protocol (§4.10). |

Five kinds are "complete" only because their protocol is a file tree. The four
warned rows are where a user following our own documentation hits a wall.

---

## 2. The cross-cutting question: does the metadata point back at us?

A proxy that serves correct metadata containing *upstream* download URLs is not a
proxy — the client fetches the bytes directly, and the rule chain never runs.
`blocking::rewrite_urls` (`crates/core/src/services/blocking/mod.rs:446`) only
handles npm and Composer, with a comment that the rest "address downloads by a
path the client builds itself". I checked that claim, because it is the kind of
comment that is true when written and false later:

| Kind | Absolute URLs in metadata? | Rewritten? |
| --- | --- | --- |
| npm | `dist.tarball` | ✅ `blocking/npm.rs` via `rewrite_urls` |
| Composer | `dist.url` | ✅ `blocking/composer.rs` via `rewrite_urls` |
| PyPI | `<a href>` in the simple page | ✅ in the handler — `pypi/simple.rs:92` |
| NuGet | `packageContent` in registration pages | ✅ `nuget/registration.rs:65,104` |
| NuGet | service index resource `@id`s | ✅ `nuget/service_index.rs:34` |
| JetBrains Marketplace | plugin download URLs | ✅ `jetbrains_marketplace/xml.rs:65` |
| cargo, Go, Maven, conda, RubyGems, Terraform *modules* | none — client builds the path | n/a |
| **Terraform providers** | `X-Terraform-Get` → upstream | ❌ **not rewritten in proxy mode** |

The last row is RFC 0006 §13.6, restated here because it belongs in a coverage
table as much as in an implementation note: `…/{version}/download` in proxy mode
resolves the upstream's `X-Terraform-Get` and hands the client a URL to fetch
directly. No bytes pass through the proxy, so no rule runs on them, so a blocked
provider version is still downloadable by anyone who can read the listing. Local
mode does go through the gate.

---

## 3. The four findings worth acting on

### 3.1 npm — `npm audit` is served on a path npm never calls

BatleHub serves:

```
POST /proxy/{registry}/-/npm/v1/audit/quick     npm/read.rs:214
POST /proxy/{registry}/-/npm/v1/audit/bulk      npm/read.rs:250
```

The npm CLI calls:

```
POST {registry}/-/npm/v1/security/audits/quick
POST {registry}/-/npm/v1/security/advisories/bulk
```

The two paths do not overlap, so `npm audit` against a BatleHub registry gets a
404 on the bulk endpoint, falls back to quick, and gets a 404 there too. The
forward is wrong in the same way on the upstream side —
`npm/read.rs:284` builds `{upstream}/-/npm/v1/audit/{endpoint}`, which is not a
path `registry.npmjs.org` answers either. So both halves of the round trip are
addressed to an endpoint that exists in neither direction.

`docs/registries/npm.md:3` says "`npm audit` works through the proxy" and
`:123` says it "works automatically once the registry is configured".
`docs/use/vulnerability-proxy.md:80-85` publishes the wrong paths as the
reference table, including the claim that `quick` is what `npm audit
--prefer-online` uses.

Four tests assert the current paths (`proxy_npm_edge_cases.rs`,
`vuln_proxy_endpoints.rs`), so the suite pins the bug rather than catching it —
the tests exercise our route, never npm's.

**Fix shape:** register the two real paths (keeping the current ones as aliases
costs nothing), forward to the matching upstream path per endpoint rather than
interpolating one template, and correct both docs pages. Cheap, and it turns a
documented feature from broken to working.

### 3.2 Terraform — no service discovery, and the docs configure the wrong protocol

Two separate problems that compound.

**No `.well-known/terraform.json`.** Terraform resolves a registry host by
fetching `https://<host>/.well-known/terraform.json` and reading `modules.v1` /
`providers.v1` from it. There is no such route anywhere in the tree — the only
`.well-known` matches in `crates/` are OIDC discovery in the auth adapters.
Without it Terraform cannot find our `/proxy/{registry}/v1/...` prefix, which is
not the default `/v1/` it would otherwise assume.

**The documented setup uses a protocol we do not implement.**
`docs/registries/terraform.md:19-30` tells the operator to configure:

```hcl
provider_installation {
  network_mirror { url = "https://batlehub.example.com/proxy/<registry>/" }
}
```

The **provider network mirror protocol** is `GET <url>/{hostname}/{namespace}/
{type}/index.json` and `GET <url>/{hostname}/{namespace}/{type}/{version}.json`.
Our routes are the **registry protocol** — `/v1/providers/{ns}/{type}/versions`
and `/v1/providers/{ns}/{type}/{version}/download/{os}/{arch}`
(`terraform/providers/read.rs:27,75`). A `network_mirror` pointed at BatleHub
gets 404s for every provider.

The private-provider snippet at `:135` has a third problem: a Terraform source
address is exactly `hostname/namespace/type`, and
`batlehub.example.com/proxy/internal-tf/myorg/mycloud` is five segments.
Terraform rejects it before any request is made. Same for the module source at
`:82`. Both would need subdomain routing (RFC 0001) to become a legal address,
with discovery served at the subdomain root.

**Also missing from the registry protocol:** `GET /v1/modules/{ns}/{name}/
{provider}/{version}` (module metadata), the module and provider *list*/search
endpoints, and `signing_keys` in the provider download response — Terraform
verifies the provider zip's GPG signature against it and refuses the provider
when it is absent.

**Fix shape:** decide which protocol we mean to speak. The network mirror is
much the smaller job (two JSON documents, no discovery, no signing keys, no
legal-source-address problem) and matches what an air-gapped estate wants; the
registry protocol is what the routes already half-implement but needs discovery
plus subdomain routing to be reachable at all. Whichever is chosen, the other
must leave the docs.

### 3.3 RubyGems — the index Bundler actually uses is the one we do not filter

Served: `specs.4.8.gz`, `latest_specs.4.8.gz`, `prerelease_specs.4.8.gz`,
`quick/Marshal.4.8/{filename}`, `api/v1/gems/{name}.json`,
`api/v1/versions/{name}.json`, `gems/{filename}`, plus push/yank/unyank.

Missing: the **compact index** — `GET /versions`, `GET /info/{gem}`, `GET
/names` — and the **dependency API**, `GET /api/v1/dependencies?gems=`.

Bundler's source resolution tries the compact index first, falls back to the
dependency API, and falls back again to the full Marshal index. With both
middle rungs missing, every `bundle install` lands on `specs.4.8.gz`.

That is the rung `RegistryKind::listing_filter()` marks
`Unsupported` — "hiding a version from a Ruby Marshal index would need a Marshal
encoder in Rust, to hide what the JSON APIs already hide for every client
released this decade" (`registry_kind.rs`). The reasoning is sound about the
JSON APIs and wrong about which API the default client reaches: **Bundler never
calls them.** A version blocked in BatleHub is visible to `bundle install`,
resolvable, and then refused at download — exactly the mid-resolve failure RFC
0006 set out to eliminate.

`docs/registries/rubygems.md:116-126` publishes an endpoint reference listing
six routes, none of them a compact-index route, so the drift is not visible to a
reader either.

**Fix shape:** serve the compact index. It is three plain-text documents, it is
filterable with the machinery RFC 0006 already built (no Marshal encoder
involved), it is what modern clients prefer anyway, and it closes the block leak
as a side effect. The `Unsupported` reason string then becomes true instead of
merely defensible.

### 3.4 Go — no checksum database proxy

Served: `@v/list`, `@latest`, `@v/{filename}` (`.info`/`.mod`/`.zip`), publish,
and the vuln DB passthrough at `/v1/index.json`, `/v1/ID/{id}.json`,
`/v1/query`.

Missing: `GET /sumdb/{sumdb-name}/{path}`. The `GOPROXY` protocol includes
proxying the checksum database so a client behind a proxy never needs to reach
`sum.golang.org` itself. Without it, `go mod download` through BatleHub still
makes a direct outbound connection to `sum.golang.org` for every module it has
not seen — which fails closed in an air-gapped estate.

`docs/registries/goproxy.md:31` documents the workaround (`GONOSUMDB`/
`GONOSUMCHECK` prefixes) but only for *private* modules; public modules fetched
through the proxy still need the real sumdb.

**Fix shape:** a passthrough route in the shape of the existing vuln
passthrough. It is a byte mirror with no filtering semantics — the sumdb is a
signed transparency log and editing it is neither possible nor wanted.

### 3.5 Conda — only the uncompressed repodata

Served: `{platform}/repodata.json`, `{platform}/current_repodata.json`,
`{platform}/{filename}` restricted by regex to `.tar.bz2`/`.conda`
(`conda.rs:46,172,220`).

Missing: `repodata.json.zst`, `repodata.json.bz2`, and `channeldata.json`.
Modern conda and mamba request the `.zst` variant first and fall back on 404, so
this is a performance and bandwidth regression rather than a broken feature —
but `repodata.json` for a real channel is tens of megabytes, requested on every
solve, and the fallback path means we pay full uncompressed transfer every time.
The `{filename}` regex means `.zst` does not even reach a handler that could
404 politely; it falls through the route table.

`channeldata.json` is used by `conda search` for cross-platform package
discovery. Its absence degrades search, not install.

**Fix shape:** serve the compressed variants of the document we already
synthesise. The filter runs before compression, so RFC 0006's guarantees carry
over unchanged.

---

## 4. The rest, in less detail

### 4.1 cargo

Served: sparse index `config.json` + `{path}` (`cargo/index.rs:23,76`), crate
download, `PUT /api/v1/crates/new`, yank, unyank, `GET
/api/v1/crates/{name}/owners`.

Missing: `GET /api/v1/crates?q=` (`cargo search` — 404s today);
`PUT`/`DELETE /api/v1/crates/{name}/owners` (`cargo owner --add/--remove`, so
ownership is readable but not manageable through the proxy). The git-based index
is not served, which is correct — sparse is the default since 1.70 and the git
index is a separate transport, not a missing endpoint.

Note that cargo is the one kind whose blocked versions are marked `yanked`
rather than removed (`listing_filter()`), which is deliberate and documented.

### 4.2 NuGet

Served: service index, `registration5/{id}/index.json`, flat index + download,
`v3/query`, `PUT api/v2/package`, `DELETE v2/package/{id}/{version}`, and the
two vulnerability documents.

The service index advertises six resources (`service_index.rs:40-68`):
`RegistrationsBaseUrl/3.6.0`, `PackageBaseAddress/3.0.0`, `PackagePublish/2.0.0`,
`SearchQueryService` (twice, plain and `/3.5.0`), `VulnerabilitiesUrl/6.7.0`.

Missing: `SearchAutocompleteService` (`dotnet package search` completion),
`SymbolPackagePublish/4.9.0` (`nuget push` of `.snupkg` symbol packages —
currently silently unsupported), `ReportAbuseUriTemplate` and
`PackageDetailsUriTemplate` (cosmetic; the client falls back to nuget.org
links, which for a private registry is a small information leak in the CLI
output). The legacy OData `/v2/` API is absent, which is fine for any client
released this decade.

Paged registration pages pass through unfiltered — already declared as
`Qualified` in `listing_filter()`, and logged.

### 4.3 Composer

Served: `packages.json` with `metadata-url` and `available-packages`
(`composer/metadata.rs:36,64-65`), `p2/{path}`, `dist/{vendor}/{package}/
{version}`, `api/security-advisories/`, upload, yank.

That is the whole Composer 2 read path. Missing: `search.json` — the adapter
calls it upstream (`composer/impl_registry.rs:226`) but no proxy route exposes
it, so `composer search` against BatleHub 404s while our own explore UI works;
`list.json` (bulk package enumeration); the Composer 1 `providers-url` scheme,
which is deliberate.

### 4.4 PyPI

Served: `simple/` root, `simple/{package}/` in both HTML and PEP 691 JSON as two
`DocumentKind`s (RFC 0006 §13.4), `packages/{filename}`, `POST legacy/` upload.
Hrefs are rewritten to point back at us (`pypi/simple.rs:92`).

That is everything pip, uv and Poetry need from a custom index. Missing: the
JSON API (`/pypi/{name}/json`, `/pypi/{name}/{version}/json`), which Poetry uses
only when the source *is* PyPI itself, and some ad-hoc tooling expects. Optional.
PEP 658 metadata files (`.metadata` siblings) are worth a look — uv and pip use
them to avoid downloading wheels during resolution, and their absence is a
silent slowdown rather than an error.

### 4.5 Maven

`GET`/`PUT` on `maven2/{path:.*}` (`maven/proxy.rs:34,137`). Maven is entirely
path-addressed — POMs, jars, sources, javadoc, checksums, signatures and
`maven-metadata.xml` are all files under that tree — so a wildcard is complete
coverage by construction. The Central search API is not proxied and no build
tool needs it.

`<lastUpdated>` is deliberately not refreshed on filtered metadata (RFC 0006
§13.2).

### 4.6 openvsx / vscode-marketplace

Closed by RFC 0006 §13.1-bis. Served: `POST vscode/gallery/extensionquery`,
`vscode/gallery/publishers/{p}/vsextensions/{n}/{v}/vspackage`,
`vscode/asset/…`, `vscode/unpkg/…`, `vscode/item`, the OpenVSX REST API
(`api/-/search`, `api/{ns}/{ext}`, `api/{ns}/{ext}/{v}`,
`api/{ns}/{ext}/{v}/file/{name}`), plus VSIX `GET`/`PUT`.

`extensionquery` handles filter types 1, 4, 5, 7, 8, 9 and 10 and the flags
bitmask (`vsx/protocol.rs:109-126`), ignoring unknown criteria rather than
failing — the right default.

Residual gaps, all secondary: `sortBy`/`sortOrder` are parsed but the result
order is our own; filter type 12 (exclude-with-flags) is not honoured; the
OpenVSX namespace endpoints (`api/{namespace}`, `api/{ns}/{ext}/reviews`,
`api/-/query`, `api/version`) and the OpenVSX publish API (`api/-/publish`,
`api/user/publish`) are absent — we publish via `PUT …/{ext}/{version}/vsix`
instead, which `ovsx publish` does not call.

### 4.7 jetbrains-marketplace

17 routes across three surfaces: XML (`updatePlugins.xml`, `plugins/list`), the
IDE API (`api/searchPlugins`, `api/search/plugins`, `api/search/updates/
compatible`, `api/plugins/{id}`, `api/plugins/{id}/updates`,
`api/search/aggregation/{field}`, `feature/getImplementations`,
`api/products/intellij/plugins/{id}/comments`) and files
(`files/pluginsXMLIds.json`, `files/jbPluginsXMLIds.json`,
`files/brokenPlugins.json`, `files/IDE/extensions.json`, meta and download
routes, `plugin/download`, `pluginManager`), plus upload.

The broadest coverage of any kind here, and wider than RFC 0006 §4.3 claimed
(§13.1 corrected the table). No gap found worth naming.

### 4.8 deb / rpm / pacman

`GET {path:.*}` per format plus a publish route each. Local publishing
regenerates the indexes — `Packages`, `Packages.gz` and `Release` for deb
(`repo/publish.rs:248-267`), `repodata/` for rpm, `<repo>.db` for pacman — and
signs them with Ed25519 OpenPGP (`crates/adapters/src/repo/openpgp.rs`):
clear-signed `InRelease`, detached `Release.gpg`, `repomd.xml.asc`, and
`%PGPSIG%` desc fields for pacman.

The most complete ecosystems in the tree. Blocked versions are deliberately not
filtered from signed indexes — editing one invalidates the signature and the
client rejects the whole repository (`listing_filter()`).

### 4.9 github / gitlab / forgejo

GitHub: releases list, release by tag, asset by id and by name, tarball,
zipball, raw. GitLab: the same via `/-/` paths plus a wildcard over
`api/v4/{path:.*}`. Forgejo: releases through the GitHub handlers plus a
wildcard over `api/packages/{path:.*}`.

The two wildcards make GitLab's and Forgejo's package registries complete by
construction. GitHub has no equivalent wildcard, so GitHub Packages
(`npm.pkg.github.com`, `maven.pkg.github.com`, `ghcr.io`) is not proxied —
consistent with these kinds being *release* mirrors, and `supports_local_mode()`
correctly returning false for all three.

### 4.10 jetbrains / generic

Path mirrors — `jetbrains/{path:.*}` over `download.jetbrains.com` and
`generic/{path:.*}` over an arbitrary tree. `is_path_addressed()` returns true
for both, `listing_filter()` returns an empty slice, and a wildcard is the
entire protocol. Nothing missing.

---

## 5. Suggested order

1. **npm audit paths** (§3.1) — smallest diff, fixes a feature we document as
   working, and the existing tests only need their URLs changed.
2. **RubyGems compact index** (§3.3) — closes a block leak, not just a coverage
   gap, and makes an `Unsupported` reason string honest.
3. **Conda compressed repodata** (§3.5) — mechanical, and the bandwidth win is
   proportional to how much anyone uses the conda proxy.
4. **Go sumdb passthrough** (§3.4) — small, and it is the difference between
   working and not working air-gapped.
5. **Terraform** (§3.2) — needs a decision before it needs code. The docs are
   wrong either way and should be corrected in the same change.
6. Everything in §4 — real, none of it blocking a default workflow.

Items 1–4 are each a day or less. Item 5 is the one that wants an RFC, because
"which Terraform protocol do we speak" also decides whether subdomain routing
(RFC 0001) is a prerequisite.

---

## 6. What this survey does not cover

- Whether served endpoints return *correct* documents. Only presence was checked.
- Authentication semantics per ecosystem (the npm `_auth` vs bearer split, cargo's
  token header, Terraform's per-host credentials).
- Rate-limit, pagination and conditional-request behaviour on the endpoints we do
  serve.
- The admin/UI API surface (`/api/v1/**`) — RFC 0004 and 0004-bis own that.
