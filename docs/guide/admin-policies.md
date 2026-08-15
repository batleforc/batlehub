# Policies & packages

## Cache policy {#cache-policy}

For a full explanation of how caching works end-to-end — request lifecycle, backend selection, rate-limit counters, deduplication — see the dedicated **[Caching guide](/guide/caching)**.

All cache settings live under `[registries.cache]` and are per-registry.

### Eviction

```toml
[registries.cache]
metadata_ttl_secs = 300      # re-check version lists after 5 minutes (default)
serve_stale       = true     # serve cached metadata when upstream is down (default)

artifact_ttl_secs = 2592000  # delete artifacts older than 30 days
idle_days         = 14       # delete artifacts not accessed for 14 days
max_size_bytes    = 10737418240  # 10 GiB storage cap — evicts LRU when exceeded
keep_latest_n     = 5        # keep only the 5 most-recently-cached versions per package
```

All eviction fields are optional. Omitting a field disables that eviction strategy. Strategies compose: an artifact is evicted as soon as **any** active strategy triggers.

| Field | Default | Description |
|-------|---------|-------------|
| `metadata_ttl_secs` | `300` | Metadata cache TTL in seconds |
| `serve_stale` | `true` | Serve stale metadata on upstream 5xx instead of propagating the error |
| `artifact_ttl_secs` | — | Evict artifacts older than N seconds |
| `idle_days` | — | Evict artifacts not accessed for N days |
| `max_size_bytes` | — | Storage cap; LRU artifacts are removed when exceeded |
| `keep_latest_n` | — | Keep only the N most recent versions per package |

### Cache warming {#cache-warming}

Cache warming pre-fetches artifact versions so they are available with zero latency on first request. Configure it alongside eviction:

```toml
[registries.cache]
warm_packages    = ["lodash", "react", "typescript@5.4.5"]
warm_latest_n    = 3   # warm the 3 most recent versions of bare-name entries
warm_concurrency = 4   # up to 4 parallel downloads
```

| Field | Default | Description |
|-------|---------|-------------|
| `warm_packages` | `[]` | Packages to warm at startup. `"name"` warms the latest `warm_latest_n` versions; `"name@version"` warms exactly one. |
| `warm_latest_n` | `1` | Versions to pre-fetch per bare-name entry |
| `warm_concurrency` | `2` | Maximum parallel downloads per warming run |

BatleHub starts warming immediately after binding the server socket, so the HTTP server is available while warming runs in the background.

#### On-demand warming via admin API

Re-warm a package at any time without restarting:

```sh
# Warm using the registry's configured warm_latest_n
curl -X POST http://localhost:8080/api/v1/admin/registries/npm/warm \
  -H "Authorization: Bearer <admin-token>" \
  -H "Content-Type: application/json" \
  -d '{"package": "lodash"}'

# Override the version count for this request only
curl -X POST http://localhost:8080/api/v1/admin/registries/npm/warm \
  -H "Authorization: Bearer <admin-token>" \
  -H "Content-Type: application/json" \
  -d '{"package": "lodash", "versions": 10}'

# Warm a single pinned version
curl -X POST http://localhost:8080/api/v1/admin/registries/cargo/warm \
  -H "Authorization: Bearer <admin-token>" \
  -H "Content-Type: application/json" \
  -d '{"package": "serde@1.0.200"}'
```

Response:

```json
{"warmed": 3, "skipped": 0, "errors": 0}
```

- `warmed` — artifact versions fetched and stored in this run
- `skipped` — versions already present in the cache (no download needed)
- `errors` — versions that failed to fetch or store

::: tip Registry support
Version enumeration (used for bare-name warming) is implemented for every package-based registry type. A pinned entry is always `name@version`, where the name half is the coordinate that registry addresses packages by — `"lodash@4.17.21"` (npm), `"com.google.guava:guava@33.0.0-jre"` (Maven), `"providers/hashicorp/aws@5.0.0"` (Terraform), `"rails@7.1.0"` (RubyGems), `"monolog/monolog@3.5.0"` (Composer). For **GitHub**, bare names enumerate releases via the Releases API (paginated). For **VS Code Marketplace**, bare names enumerate all extension versions via the Gallery API. For **Conda**, BatleHub synthesises the version list by scanning `repodata.json` across `noarch`, `linux-64`, `osx-64`, `osx-arm64`, and `win-64`. For **JetBrains Marketplace**, an entry is the plugin `xmlId` (`"org.rust.lang"`, `"org.rust.lang@0.4.201"`) and bare names enumerate versions via `/plugins/list` — which covers the **Stable** channel only, so EAP/nightly builds are not pre-fetched.
:::

### Content-addressable deduplication

BatleHub stores artifact bytes at a content-addressed key (`blob/{sha256}`) and maps logical artifact keys (e.g. `artifact:npm/lodash/4.17.21`) to that blob via a reference count. When identical bytes appear under multiple logical keys — the same package mirrored across two registries, a yanked-then-re-released version — only one copy is stored on disk or in S3.

This is automatic and requires no configuration. Pre-deduplication artifacts stored before upgrading continue to be served normally.

---

## Package management {#package-management}

### List packages

```sh
# All packages
curl -H "Authorization: Bearer <admin-token>" \
  "http://localhost:8080/api/v1/admin/packages"

# Filter by registry and name
curl -H "Authorization: Bearer <admin-token>" \
  "http://localhost:8080/api/v1/admin/packages?registry=npm&name=lodash"
```

### Block a package version

A block does two things, and both matter:

1. **The version disappears from version listings.** It is removed from the npm
   packument and from local/hybrid version indexes, and `dist-tags.latest` is
   recomputed to the newest version that is still allowed. A client asking for
   `latest`, or for a range like `^4.17.0`, therefore resolves to an allowed
   version and installs successfully — it never selects the blocked one.
2. **Downloading it returns `403 Forbidden`** to all clients regardless of role,
   with the reason you recorded. Hiding governs which version a resolver
   *picks*; this governs whether someone who names the version explicitly may
   have it. Pinning `lodash@4.17.20` in a lockfile fails with a message that
   says why, rather than looking like a missing package.

A block recorded against a version covers every file in it — the npm tarball,
a Maven classifier, a Terraform provider binary. Blocking one specific artifact
(by passing `artifact`) leaves the version's other files alone.

```sh
curl -X POST \
  -H "Authorization: Bearer <admin-token>" \
  -H "Content-Type: application/json" \
  -d '{"registry": "npm", "name": "lodash", "version": "4.17.20", "reason": "CVE-2021-23337"}' \
  http://localhost:8080/api/v1/admin/packages/block
```

::: warning Listing filtering covers npm and local/hybrid registries
Every **local/hybrid** registry filters its version listings (npm, Maven, NuGet,
RubyGems, Go, JetBrains, Terraform, Composer). For **proxied** registries the
filtering currently applies to npm packuments; other proxied ecosystems still
list the blocked version in their upstream index, and the block takes effect at
download time with a `403`.
:::

### Unblock

```sh
curl -X POST \
  -H "Authorization: Bearer <admin-token>" \
  -H "Content-Type: application/json" \
  -d '{"registry": "npm", "name": "lodash", "version": "4.17.20"}' \
  http://localhost:8080/api/v1/admin/packages/unblock
```

### Bulk block

```sh
curl -X POST \
  -H "Authorization: Bearer <admin-token>" \
  -H "Content-Type: application/json" \
  -d '{"packages": [{"registry":"npm","name":"bad-pkg","version":"1.0.0"}]}' \
  http://localhost:8080/api/v1/admin/packages/bulk-block
```

### Invalidate cache

Removes the cached artifact so the next request re-fetches from upstream:

```sh
curl -X POST \
  -H "Authorization: Bearer <admin-token>" \
  -H "Content-Type: application/json" \
  -d '{"registry": "npm", "name": "lodash", "version": "4.17.21"}' \
  http://localhost:8080/api/v1/admin/packages/invalidate
```

---

## Rules {#rules}

Rules are optional per-registry policies evaluated after RBAC.

### Release age gate

Block packages published less than `min_age_secs` ago:

```toml
[[registries.rules]]
kind         = "release_age_gate"
min_age_secs = 3600       # 1 hour
bypass_roles = ["admin"]  # admins can still install new packages
```

### Deny latest tag

Force clients to pin exact versions:

```toml
[[registries.rules]]
kind         = "deny_latest"
bypass_roles = ["admin"]
```

### Trusted publisher

Restrict downloads to packages published by an allowed org, user, or scope. The publisher is derived from metadata already resolved during the proxy fetch — no extra upstream calls.

```toml
[[registries.rules]]
kind         = "trusted_publisher"
allow        = ["my-org", "trusted-user"]
bypass_roles = ["admin"]
```

Publisher support by registry type (matching is case-insensitive):

- **GitHub**, **GitLab**, **Forgejo** — the top-level owner/group segment of the package path (`"owner/repo"` → `"owner"`)
- **npm** — the scope for scoped packages (`"@scope/name"` → `"scope"`); otherwise the publishing user
- **OpenVSX**, **VS Code Marketplace** — the publisher segment of the extension id (`"publisher.extension"` → `"publisher"`)
- **Not yet supported: Cargo** and any other registry type — configuring this rule there denies every request (fail-closed)

See [`docs/guide/configuration.md`](https://github.com/batleforc/batlehub/blob/main/docs/guide/configuration.md) for the full field table.
