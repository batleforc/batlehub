# Package Explorer — searching

Three searches live behind one box, and each answers a different question.

| Search | Answers | Where |
| --- | --- | --- |
| Names, here | *do we have something called `retry`* | always on |
| **README prose, here** | *which of our libraries does exponential backoff* | opt-in — below |
| Names, upstream | *does this exist at all, and should we pull it* | [below](#upstream-search) |

## Searching what a package says {#readme-search}

A name search cannot answer *"which of our internal libraries does exponential
backoff"*. That is the question a developer actually arrives with, and for an
internal package it is the question an internal package page is the only place in
the world that could answer — there is no npmjs.com to go and read instead.

With `[search] readmes = true`
([configuration](/guide/admin-config#search-readmes)), the listing endpoint takes
a scope:

```
GET /api/v1/explore/packages?q=exponential+backoff&in=name|readme|both
```

`in` defaults to `name`, which is the behaviour that has always shipped. Each hit
gains two fields:

| Field | Meaning |
| --- | --- |
| `matched_in` | `name` \| `readme` \| `both` — why this row is here |
| `snippet` | The matched fragment of the README, as plain text, or `null` |

**A name match always outranks a prose match.** A package literally called
`retry` comes before one that mentions retrying, however densely. That is what a
reader means when they type a name; it is not a tuning parameter.

`matched_in` is there because a result that matches nothing the reader can see
reads as a bug. A row whose name has nothing to do with the query and whose
README mentions it in passing is a *correct* result and an inexplicable one
without the label.

**Only stored READMEs are searchable** — that is, only versions this instance
holds or hosts. A README derived on the fly for a version this instance holds no
bytes of has no row to index, and writing one is what the discovery read
deliberately refuses to do. The empty state says so rather than implying the
query found nothing.

With the feature off, `in=readme` and `in=both` are accepted and answer exactly
as `in=name` does, and the response carries `readme_search_enabled: false` — so a
client can tell *"no package here says that"* from *"this instance does not
search prose"*.

## Upstream search {#upstream-search}

When you type a query (≥ 2 characters), the Explorer also queries upstream registries to surface packages you haven't yet routed through BatleHub. Results are appended to the bottom of the main table with a **Not Yet Proxied** badge in the Proxy column.

### Supported registries {#upstream-supported}

| Registry type | Default search endpoint | Notes |
| --- | --- | --- |
| `npm` | `{upstream}/-/v1/search` | Full text search |
| `openvsx` | `{upstream}/api/-/search` | Full text search; results use `publisher.name` format |
| `cargo` | `{upstream}/api/v1/crates` | Full text search |
| `rubygems` | `{upstream}/api/v1/search.json` | Full text search |
| `composer` | `https://packagist.org/search.json` | Full text search; version field is `"latest"` (Packagist omits it from search results) |
| `maven` | `https://search.maven.org/solrsearch/select` | Solr full text search against Maven Central |
| `terraform` | `{upstream}/v1/modules/search` (modules) + namespace/exact provider lookup | The Terraform Registry Protocol has no full-text provider search. See note below. |
| `pypi` | `{upstream}/pypi/{name}/json` | Exact name lookup only (PyPI removed its public search API) |
| `nuget` | `{upstream}/v3/query` | NuGet v3 search service; full text search |
| `goproxy` | `https://pkg.go.dev/search` | The GOPROXY protocol has no search endpoint, so BatleHub queries pkg.go.dev (HTML). Version is `"latest"`; configurable/disable via `search_url`. |

The remaining registry types have **no upstream search API**, so the Explorer shows
only their cached and locally-published packages (no "Not Yet Proxied" rows):
`github`, `forgejo`, `gitlab` (release proxies — search a repo by `owner/repo`
directly), `vscode-marketplace`, `conda`, and the path-based `deb` / `rpm`
repository formats.

> **Terraform provider search limitation**
>
> The Terraform Registry Protocol v1 has no full-text provider search endpoint.
> BatleHub works around this with two fallback strategies:
>
> - **Namespace lookup** — the query is treated as a provider namespace.
>   Searching `netbirdio` returns all providers published under that org
>   (e.g. `providers/NetBirdIO/netbird`).
> - **Exact pair lookup** — if the query contains `/`, it is treated as
>   `namespace/type` and resolved directly (e.g. `netbirdio/netbird`).
>   The lookup is case-insensitive.
>
> Module search always runs in parallel using full-text matching.

Upstream search failures are silently swallowed — if a registry's search API is unreachable, the cached results are unaffected.

### Configuring the search URL {#search-url-config}

For `maven`, `composer`, and `goproxy`, the search service lives on a different host than the repository (Maven Central's Solr, Packagist, and pkg.go.dev respectively). BatleHub uses the public defaults above, but you can override or disable this per registry with `search_url`:

```toml
# Use a private Nexus instance for both proxying and search
[[registries]]
type      = "maven"
name      = "nexus"
upstreams = ["https://nexus.internal/repository/maven-public"]
search_url = "https://nexus.internal/solrsearch"

# Use a private Satis server — search endpoint is on the same host
[[registries]]
type      = "composer"
name      = "satis"
upstreams = ["https://satis.internal"]
search_url = "https://satis.internal"

# Point Go search at a private pkg.go.dev-compatible site (default: https://pkg.go.dev)
[[registries]]
type      = "goproxy"
name      = "go"
search_url = "https://pkgsite.internal"

# Disable upstream search entirely for a sensitive registry
[[registries]]
type      = "cargo"
name      = "internal-cargo"
upstreams = ["https://cargo.internal"]
search_url = ""
```

| Value | Behaviour |
| --- | --- |
| Absent (default) | Use the registry type's built-in default search endpoint |
| `"https://..."` | Use this base URL for search |
| `""` (empty string) | Disable upstream search for this registry |
