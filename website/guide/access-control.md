# Access Control

BatleHub provides two complementary access-control features for private and hybrid registries:

- **Beta/Pre-Release Channel** — restrict pre-release package versions to approved users or groups
- **IP-Based Blocking** — automatically block abusive IPs (fail2ban-style) and manage manual bans

[[toc]]

---

## Beta/Pre-Release Channel {#beta-channel}

### How it works {#beta-how-it-works}

BatleHub determines whether a version is a pre-release by parsing its version string as [semver](https://semver.org/). Any version with a pre-release component (the hyphenated suffix) is treated as a pre-release:

| Version | Pre-release? |
|---------|-------------|
| `1.0.0` | No |
| `1.0.0-beta.1` | **Yes** |
| `1.0.0-rc.2` | **Yes** |
| `1.0.0-alpha` | **Yes** |

There is **no separate flag or publish step** — the version string itself determines gating. Publish `mylib@1.0.0-beta.1` the same way as any other version; BatleHub infers it is a pre-release from the `-beta.1` suffix.

When `beta_channel.enabled = true` for a registry:

- **Non-members** — pre-release versions are hidden from version listings, and artifact downloads return 404.
- **Members** — pre-release versions are visible and downloadable alongside stable versions.

Stable versions are always visible to everyone regardless of membership.

### Configuration {#beta-config}

Add a `[registries.beta_channel]` block to any registry in `local` or `hybrid` mode:

```toml
[[registries]]
type = "npm"
name = "my-npm"
mode = "local"

[registries.beta_channel]
enabled = true
```

`enabled` is the only option. Members are managed at runtime via the admin API.

Omitting the block (or setting `enabled = false`) makes all versions visible to everyone.

### Managing members {#beta-members}

All endpoints require an `Admin` role token.

#### List members

```sh
curl -H "Authorization: Bearer <admin-token>" \
  https://batlehub.example.com/api/v1/admin/registries/my-npm/beta-channel
```

```json
[
  { "principal_type": "user",  "principal_id": "alice",   "granted_by": "admin" },
  { "principal_type": "group", "principal_id": "qa-team", "granted_by": null }
]
```

#### Add a member

`principal_type` is `"user"` or `"group"`. A `"group"` entry grants access to every user carrying that group claim (from OIDC or Kubernetes auth).

```sh
# Add a specific user
curl -s -X POST \
  -H "Authorization: Bearer <admin-token>" \
  -H "Content-Type: application/json" \
  -d '{"principal_type":"user","principal_id":"alice","granted_by":"admin"}' \
  https://batlehub.example.com/api/v1/admin/registries/my-npm/beta-channel

# Add an entire group
curl -s -X POST \
  -H "Authorization: Bearer <admin-token>" \
  -H "Content-Type: application/json" \
  -d '{"principal_type":"group","principal_id":"qa-team"}' \
  https://batlehub.example.com/api/v1/admin/registries/my-npm/beta-channel
```

Returns `204 No Content` on success, `409 Conflict` if the principal is already a member.

#### Remove a member

```sh
curl -s -X DELETE \
  -H "Authorization: Bearer <admin-token>" \
  https://batlehub.example.com/api/v1/admin/registries/my-npm/beta-channel/user/alice

curl -s -X DELETE \
  -H "Authorization: Bearer <admin-token>" \
  https://batlehub.example.com/api/v1/admin/registries/my-npm/beta-channel/group/qa-team
```

### What users see {#beta-user-experience}

#### As a non-member

```sh
# npm — only stable versions are listed
npm view my-package versions --registry https://batlehub.example.com/proxy/my-npm
# [ '1.0.0', '1.1.0' ]

# Attempting to install a pre-release → 404
npm install my-package@1.0.0-beta.1 --registry https://batlehub.example.com/proxy/my-npm
# npm error 404 Not Found
```

#### As a member

```sh
# All versions listed, including pre-releases
npm view my-package versions --registry https://batlehub.example.com/proxy/my-npm
# [ '1.0.0', '1.0.0-beta.1', '1.0.0-rc.2', '1.1.0' ]

npm install my-package@1.0.0-beta.1 --registry https://batlehub.example.com/proxy/my-npm
# added 1 package
```

### Registry support {#beta-registries}

Gating applies in **local and hybrid mode** only — proxy-only registries proxy upstream as-is.

| Registry | Listing gated | Download gated |
|----------|:------------:|:--------------:|
| npm | ✓ | ✓ |
| Cargo | ✓ | ✓ |
| Go modules | ✓ | ✓ |
| RubyGems | ✓ | ✓ |
| Maven | ✓ | ✓ |
| Terraform modules | ✓ | ✓ |
| Terraform providers | ✓ | ✓ |

::: warning Maven and non-semver versions
Maven versions that are not valid semver (e.g. `1.0-SNAPSHOT`) are never treated as pre-releases and are always visible. SNAPSHOT gating would require a separate feature.
:::

---

## IP-Based Blocking {#ip-blocking}

### How it works {#ip-how-it-works}

BatleHub counts violation events per IP address within a sliding time window. When the count exceeds the configured threshold, the IP is automatically blocked for the configured duration.

A **violation** is any response whose status code appears in `trigger_on_status` (default: 429 and 401). This means:

- Repeated rate-limit hits → violations accumulate → auto-block.
- Auth brute-force attempts → violations accumulate → auto-block.

Blocked IPs receive `403 Forbidden` with an `X-Block-Expires` header containing the Unix timestamp when the block lifts. The check runs **before authentication**, so blocked IPs consume no auth resources.

The store is fail-open: if the backing store is unavailable, requests are allowed through rather than hard-blocked.

### Configuration {#ip-config}

Add an `[ip_blocking]` section at the **root** of `config.toml` (not inside a `[[registries]]` block):

```toml
[ip_blocking]
enabled               = true
violation_threshold   = 10       # violations before auto-block
violation_window_secs = 300      # counting window (5 minutes)
ban_duration_secs     = 3600     # block duration (1 hour)
trigger_on_status     = [429, 401]
```

| Field | Default | Description |
|-------|---------|-------------|
| `enabled` | `false` | Activate IP blocking |
| `violation_threshold` | `10` | Violations in the window before auto-block |
| `violation_window_secs` | `300` | Window duration in seconds |
| `ban_duration_secs` | `3600` | How long an auto-block lasts |
| `trigger_on_status` | `[429, 401]` | HTTP status codes that count as violations |

Only `enabled = true` is required; all other fields have sensible defaults.

::: tip Behind a load balancer
If BatleHub sits behind a proxy, real client IPs arrive via `X-Forwarded-For`. BatleHub uses the **first** IP from that header. Ensure your load balancer sets this header correctly and strips any client-supplied values to prevent spoofing.
:::

### Manual block management {#ip-admin}

All endpoints require an `Admin` role token.

#### List blocked IPs

```sh
curl -H "Authorization: Bearer <admin-token>" \
  https://batlehub.example.com/api/v1/admin/ip-blocks
```

```json
[
  {
    "ip":         "1.2.3.4",
    "blocked_at": 1748304000,
    "unblock_at": 1748307600,
    "reason":     "auto"
  }
]
```

#### Block an IP manually

```sh
curl -s -X POST \
  -H "Authorization: Bearer <admin-token>" \
  -H "Content-Type: application/json" \
  -d '{"ip":"1.2.3.4","reason":"known bad actor","duration_secs":86400}' \
  https://batlehub.example.com/api/v1/admin/ip-blocks
```

| Field | Required | Description |
|-------|:--------:|-------------|
| `ip` | Yes | IP address to block |
| `reason` | No | Stored for audit purposes |
| `duration_secs` | No | Defaults to `3600` |

#### Unblock an IP

```sh
curl -s -X DELETE \
  -H "Authorization: Bearer <admin-token>" \
  https://batlehub.example.com/api/v1/admin/ip-blocks/1.2.3.4
```

Auto-blocking will resume if the IP continues to trigger violations after being unblocked.

### Storage backends {#ip-storage}

Violation counters and block records share the backend selected by `config.cache.cache_type`:

| `cache_type` | Storage | Survives restart | Shared across instances |
|-------------|---------|:---------------:|:----------------------:|
| `memory` (default) | In-process | No | No |
| `postgres` | `ip_violation_counters` + `ip_blocks` tables | Yes | Yes |
| `redis` | Keys with TTL | Yes (if Redis persists) | Yes |

Use `postgres` or `redis` in production so blocks survive restarts and are enforced consistently across multiple BatleHub replicas.

---

## Combining both features {#combining}

The two features are independent and work well together. A common private-registry setup:

```toml
[[registries]]
type = "npm"
name = "my-npm"
mode = "local"

[registries.rate_limit]
requests_per_window = 100
window_secs         = 60
enforcement         = "block"

[registries.beta_channel]
enabled = true

[ip_blocking]
enabled               = true
violation_threshold   = 10
violation_window_secs = 300
ban_duration_secs     = 3600
trigger_on_status     = [429, 401]
```

Flow:
1. Rate limiting blocks excessive requests → 429 counts as a violation.
2. Auth failures (401) also count → brute-force attempts auto-block the source IP.
3. Beta releases are visible only to users or groups added via the admin API.
