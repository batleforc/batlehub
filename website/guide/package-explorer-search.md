# Package Explorer — Upstream search

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
