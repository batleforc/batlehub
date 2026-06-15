# Package Explorer

The Package Explorer is a browsable catalog of every package BatleHub knows about. It collapses all versions of a package into a single row, combines proxied packages with locally published ones, and lets you search for packages that haven't been proxied yet by querying upstream registries in real time.

[[toc]]

---

## Overview {#overview}

The Explorer is available to any user at `/explore` in the web UI. It has two views:

- **Catalog** (`/explore`) — one row per unique package name, across all accessible registries or filtered to a single one. Sortable by download count, name, or last accessed.
- **Package detail** (`/explore/packages/<registry>/<name>`) — all known versions with their source (proxied vs. locally published), per-version firewall status, and a gate summary showing your access level for that registry.

### Data sources {#sources}

| Source | Where the data lives |
| --- | --- |
| **Proxied** | `package_statuses` — every package ever requested through the proxy |
| **Local** | `local_packages` — packages published directly to a BatleHub `local` or `hybrid` registry |
| **Upstream** | Live search call to the upstream registry API (npm, crates.io, RubyGems) when you type a query |

Proxied and local versions of the same package are merged into a single entry showing `Both` as the source.

---

## Using the catalog {#catalog}

### Registry sidebar {#sidebar}

The left panel lists **every accessible registry**, including those that have not yet had any packages pass through them (shown with a count of `0`). Click a registry to filter the table; click **All registries** to see everything.

### Search {#search}

Type in the search box. After a 300 ms debounce two things happen:

1. The main table is filtered by substring match on the package name (server-side, case-insensitive).
2. An **upstream search** fires for registries that support it (see [Upstream search](#upstream-search)). Results appear at the bottom of the same table, marked **Not Yet Proxied**.

### Sort {#sort}

| Option | Behaviour |
| --- | --- |
| Most Downloaded | Packages with the highest total access-event count first |
| Name A–Z | Alphabetical by package name |
| Recently Accessed | Packages last requested most recently first |

### Table columns {#columns}

| Column | Notes |
| --- | --- |
| **Package** | Package name (monospace). |
| **Registry** | Registry the package belongs to. |
| **Versions** | Number of known versions (cached). For upstream-only rows: the latest version string from the upstream registry. |
| **Downloads** | Total access-event count across all versions. `—` for upstream-only rows. |
| **Source** | `Proxied`, `Local`, or `Both` for cached packages. For upstream-only rows: the package's description (if available). |
| **Proxy** | `Proxied` (solid badge) for packages already in the cache; `Not Yet Proxied` (dashed outline badge) for upstream-only results. |

A `Has blocked` badge appears alongside the **Source** badge when at least one version is currently blocked.

---

## Package detail {#detail}

Click any cached row in the catalog to open the detail page for that package.

### Gate summary {#gate}

The **Access Gate** card shows two checks against your current session:

| Check | Green | Red / Grey |
| --- | --- | --- |
| **Registry access** | Your role can proxy from this registry | Your role cannot access this registry |
| **Beta channel** | You are a beta-channel member — pre-release versions are visible | You are not a member; pre-release versions are hidden |

The gate card reflects what your current token allows. If the registry is accessible but a specific version is blocked, that appears in the Firewall column of the versions table rather than in the gate card.

### Versions table {#versions}

Each row is one version of the package. Columns:

| Column | Notes |
| --- | --- |
| **Version** | Version string. Pre-release versions (containing `-`) are shown in italic with a `pre-release` badge. |
| **Source** | `Proxied` (from upstream cache) or `Local` (published directly). |
| **Firewall** | See below. |
| **Downloads** | Total access-event count for this exact version. |
| **Last Accessed** | Most recent access-event timestamp. |
| **Published** | `published_at` timestamp for local packages; `—` for proxied. |

#### Firewall status {#firewall}

| Badge | Meaning |
| --- | --- |
| `Clear` | Version is available. |
| `Blocked` | An administrator blocked this version. Hover the badge to see the reason, who blocked it, and when. |
| `Yanked` | Version was yanked after publish (local packages only). |

---

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

---

## Access control {#access-control}

### Proxy access vs. explore access {#access-separation}

By default, any user who can proxy from a registry can also browse it in the Explorer. You can restrict browsing independently of proxying using `[registries.rbac.explore]`.

This is useful when:

- You want CI/CD tokens to be able to download packages but not enumerate what's in the registry.
- You have a sensitive internal registry that should be accessible by tooling but not visible in the UI.

### Configuration {#rbac-config}

Add an `explore` block inside `[registries.rbac]`:

```toml
[[registries]]
name = "internal-cargo"
type = "cargo"
mode = "hybrid"
upstreams = ["https://index.crates.io"]

[registries.rbac]
user  = ["read"]    # regular users can proxy/download
admin = ["read"]    # admins can proxy/download

[registries.rbac.explore]
anonymous = false   # anonymous users cannot browse
user      = false   # regular users cannot browse (proxy-only)
admin     = true    # admins can browse
```

All three fields default to `true`, so omitting the `explore` block (or omitting individual fields) grants browse access to every role that already has proxy access.

### Inheritance {#rbac-inheritance}

Explore access is always capped by proxy access. A role that cannot proxy from a registry cannot explore it either, regardless of the `explore` flags:

```txt
effective explore access = proxy access AND explore permission
```

Group-level explore permissions are not separately configurable — group members inherit the explore access of their role (user or anonymous).

---

## REST API {#api}

All endpoints require a Bearer token with at least the `user` role (or `anonymous` role if the registry is public). Only registries the caller can explore are included in responses.

### List packages {#api-list}

```http
GET /api/v1/explore/packages
```

**Query parameters:**

| Parameter | Type | Default | Description |
| --- | --- | --- | --- |
| `registry` | string | — | Filter to a single registry. |
| `name` | string | — | Substring filter on package name (case-insensitive). |
| `sort` | `downloads` \| `name` \| `recent` | `downloads` | Sort order. |
| `page` | integer | `0` | Zero-based page number. |
| `per_page` | integer | `20` | Results per page. |

**Response:**

```json
{
  "items": [
    {
      "registry": "cargo",
      "name": "tokio",
      "version_count": 50,
      "total_downloads": 12500,
      "last_accessed": "2026-05-31T10:00:00Z",
      "source": "proxied",
      "has_blocked": false
    }
  ],
  "total": 150,
  "page": 0,
  "per_page": 20,
  "upstream_unavailable": false
}
```

`source` is one of `"proxied"`, `"local"`, or `"both"`.

`upstream_unavailable` is `true` only when the database was unreachable **and** no cached data was available for this query. Results will be empty. See [Explorer cache](#cache) for details.

### Registry statistics {#api-stats}

```http
GET /api/v1/explore/registries
```

Returns per-registry package counts and total download events for registries that already have cached packages. The web UI calls this alongside `GET /api/v1/registries` (which returns all configured registries) and merges the two lists so that empty registries show a count of `0`.

**Response:**

```json
{
  "registries": [
    { "registry": "cargo", "package_count": 120, "total_downloads": 45000 },
    { "registry": "npm",   "package_count":  30, "total_downloads":  8200 }
  ],
  "upstream_unavailable": false
}
```

### Package detail {#api-detail}

```http
GET /api/v1/explore/packages/{registry}/{name}
```

Returns all known versions of a package, the caller's gate status, and per-version firewall status.

**Response:**

```json
{
  "registry": "cargo",
  "name": "tokio",
  "gate": {
    "registry_accessible": true,
    "beta_member": false
  },
  "versions": [
    {
      "version": "1.38.0",
      "source": "proxied",
      "firewall": { "status": "clear" },
      "download_count": 500,
      "last_accessed": "2026-05-31T10:00:00Z",
      "published_at": null,
      "is_prerelease": false
    },
    {
      "version": "0.9.0",
      "source": "proxied",
      "firewall": {
        "status": "blocked",
        "reason": "CVE-2021-12345",
        "blocked_by": "admin",
        "blocked_at": "2026-01-10T12:00:00Z"
      },
      "download_count": 80,
      "last_accessed": "2026-01-09T09:00:00Z",
      "published_at": null,
      "is_prerelease": false
    }
  ],
  "upstream_unavailable": false
}
```

`firewall.status` is one of `"clear"`, `"blocked"`, or `"yanked"`. Blocked entries include `reason`, `blocked_by`, and `blocked_at`.

### Upstream search {#api-upstream}

```http
GET /api/v1/explore/upstream?name=<query>&registry=<optional>&limit=<n>
```

Queries upstream registry search APIs for packages matching `name`. Only registries the caller can explore are searched.

| Parameter | Type | Default | Description |
| --- | --- | --- | --- |
| `name` | string | (required) | Search query. |
| `registry` | string | — | Limit to a single registry. |
| `limit` | integer | `10` | Maximum results per registry. |

**Response:**

```json
{
  "items": [
    {
      "registry": "npm",
      "name": "lodash",
      "latest_version": "4.17.21",
      "description": "Lodash modular utilities.",
      "already_cached": false
    }
  ]
}
```

`already_cached: true` means the package already appears in the main catalog (the UI suppresses it from the Not Yet Proxied section).

---

## Explorer cache {#cache}

Explorer catalog results are served from an **in-memory cache** to avoid scanning all package tables on every page load. This is important for large registries with tens of thousands of packages.

### How it works {#cache-how}

| Property | Value |
| --- | --- |
| TTL | 10 minutes |
| Scope | Per query (registry filter + name search + sort + page) |
| Invalidation | TTL expiry, admin flush, or successful publish |
| Stale-on-failure | Yes — expired entries are kept and served if the database is unreachable |
| Persistence | In-memory only; cleared on server restart |
| Multi-instance | Each instance has its own cache; use the admin API to flush all instances after bulk data changes |

### Stale-while-unavailable {#cache-stale}

If the database becomes unreachable during a request, BatleHub checks whether a stale (expired) cache entry exists for that exact query:

- **Stale entry exists** → The stale results are returned silently. The response includes `"upstream_unavailable": false` because data is available.
- **No cache entry** → An empty result is returned with `"upstream_unavailable": true`. The UI surfaces a warning badge to indicate that results may be incomplete.

This means the Explorer remains usable during database outages as long as the queries being issued have been cached at least once before the outage.

> The upstream search endpoint (`GET /api/v1/explore/upstream`) is **not cached** — it fans out to live upstream registries and always returns real-time results.

### Automatic invalidation {#cache-auto-invalidate}

The cache is invalidated automatically when:

1. **A package is published** to a local or hybrid registry via `cargo publish`, `npm publish`, etc. Only the entries for that specific registry are cleared.
2. **TTL expires** after 10 minutes.

There is no automatic invalidation when a package is first proxied (i.e. downloaded for the first time through BatleHub). Those entries appear in the Explorer at the next TTL refresh, typically within 10 minutes.

### Manual invalidation {#cache-admin}

Admins can flush the cache from the admin panel or via the API.

#### Admin UI

Navigate to **Admin → Explore Cache** (`/admin/explore-cache`). Two actions are available:

- **Invalidate by Registry** — select a registry from the dropdown and click **Invalidate Registry**. Only cache entries that include that registry are cleared.
- **Invalidate All** — flushes the entire cache. All registries are affected.

After invalidation the next request for any flushed query will re-query the database, repopulating the cache transparently.

#### Admin API

```http
POST /api/v1/admin/explore/invalidate
Authorization: Bearer <admin-token>
Content-Type: application/json
```

**Request body:**

| Field | Type | Description |
| --- | --- | --- |
| `registry` | string (optional) | Registry to flush. Omit to flush everything. |

**Flush one registry:**

```sh
curl -X POST https://batlehub.example.com/api/v1/admin/explore/invalidate \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"registry": "npm"}'
# {"ok": true}
```

**Flush all registries:**

```sh
curl -X POST https://batlehub.example.com/api/v1/admin/explore/invalidate \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{}'
# {"ok": true}
```

**Responses:**

| Status | Description |
| --- | --- |
| 200 | `{"ok": true}` — cache flushed |
| 403 | Admin role required |

### Multi-instance deployments {#cache-ha}

The explorer cache is **per-process**. In a multi-replica deployment (Kubernetes, Docker Swarm), each replica has its own independent cache. This means:

- A flush via the API only affects the replica that handled the request.
- Cache TTLs on other replicas tick independently.

After a bulk data operation (database migration, mass publish, registry restructuring) you should call the flush endpoint for **every replica**, or wait up to 10 minutes for TTL expiry to propagate naturally.

See [High Availability](./high-availability) for replica-aware rollout strategies.

---

## Performance notes {#performance}

The catalog queries run two CTEs that union `package_statuses` and `local_packages`, then join access-event counts. The following indexes (added in migration 017) keep these fast:

| Index | Purpose |
| --- | --- |
| `idx_access_events_pkg` on `(registry, package_name, package_version)` | JOIN condition in the package list |
| `idx_access_events_pkg_allowed_recent` on `(registry, package_name, package_version, outcome, created_at DESC)` | `last_accessed_by` correlated subquery |
| `idx_access_events_registry_name` on `(registry, package_name)` | LATERAL access-event count in the explore catalog |
| `idx_package_statuses_registry_name` on `(registry, package_name)` | Explorer GROUP BY aggregation |

These indexes are created automatically when BatleHub starts and runs migrations. No manual action is required.

For large registries (> 50 000 packages), the 10-minute in-memory cache reduces the load of repeated Explorer requests to near-zero. If you need a shorter TTL to reflect publishes faster, use the admin flush endpoint as part of your CI/CD pipeline (see [Automatic invalidation](#cache-auto-invalidate)).
