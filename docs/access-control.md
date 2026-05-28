# Access Control — Beta Channel, IP Blocking & Team Namespaces

This document covers the access-control features available in BatleHub:

- **Beta/Pre-Release Channel** — restrict pre-release package versions to a list of approved users or groups
- **IP-Based Blocking** — automatically block abusive IPs (fail2ban-style) and manage manual bans
- **Team Namespaces & Package Visibility** — assign package name prefixes to auth-provider groups and control per-package download visibility

---

## Table of Contents

1. [Beta/Pre-Release Channel](#1-betapre-release-channel)
   - [How it works](#11-how-it-works)
   - [Configuration](#12-configuration)
   - [Managing members](#13-managing-members)
   - [What users see](#14-what-users-see)
   - [Registry support](#15-registry-support)
2. [IP-Based Blocking](#2-ip-based-blocking)
   - [How it works](#21-how-it-works)
   - [Configuration](#22-configuration)
   - [Manual block management](#23-manual-block-management)
   - [Storage backends](#24-storage-backends)
3. [Team Namespaces & Package Visibility](#3-team-namespaces--package-visibility)
   - [How it works](#31-how-it-works)
   - [Managing namespace claims](#32-managing-namespace-claims)
   - [Package visibility](#33-package-visibility)
   - [Registry support](#34-registry-support)
4. [Combining features](#4-combining-features)

---

## 1. Beta/Pre-Release Channel

### 1.1 How it works

BatleHub determines whether a version is a pre-release by parsing its version string as [semver](https://semver.org/). Any version with a pre-release component (the hyphenated suffix) is considered a pre-release:

| Version | Pre-release? |
|---------|-------------|
| `1.0.0` | No |
| `1.0.0-beta.1` | **Yes** |
| `1.0.0-rc.2` | **Yes** |
| `1.0.0-alpha` | **Yes** |
| `2.0.0` | No |

There is no separate publish step or flag — the **version string itself** determines gating. Publish `mylib@1.0.0-beta.1` the same way as any other version; BatleHub infers it is a pre-release from the `-beta.1` suffix.

When `beta_channel.enabled = true` for a registry:

- **Non-members**: pre-release versions are **hidden** from index/version listings and artifact downloads return 404.
- **Members**: pre-release versions are visible and downloadable alongside stable versions.

Stable versions are always visible to everyone regardless of membership.

### 1.2 Configuration

Add a `[registries.beta_channel]` block to any registry running in `local` or `hybrid` mode:

```toml
[[registries]]
type  = "npm"
name  = "my-npm"
mode  = "local"

[registries.beta_channel]
enabled = true
```

That is the only config option. Members are managed at runtime via the admin API (see §1.3).

Disabling the feature (`enabled = false` or omitting the block entirely) makes all published versions visible to everyone, regardless of any members in the database.

### 1.3 Managing members

All endpoints require an `Admin` role token.

#### List members

```sh
curl -H "Authorization: Bearer <admin-token>" \
  https://batlehub.example.com/api/v1/admin/registries/my-npm/beta-channel
```

Response:

```json
[
  { "principal_type": "user",  "principal_id": "alice",   "granted_by": "admin" },
  { "principal_type": "group", "principal_id": "qa-team", "granted_by": null }
]
```

#### Add a member

`principal_type` must be `"user"` or `"group"`. A `"group"` entry grants access to all users who carry that group claim (from OIDC or Kubernetes auth).

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
# Remove a user
curl -s -X DELETE \
  -H "Authorization: Bearer <admin-token>" \
  https://batlehub.example.com/api/v1/admin/registries/my-npm/beta-channel/user/alice

# Remove a group
curl -s -X DELETE \
  -H "Authorization: Bearer <admin-token>" \
  https://batlehub.example.com/api/v1/admin/registries/my-npm/beta-channel/group/qa-team
```

Returns `204 No Content`.

### 1.4 What users see

#### As a non-member

```sh
# npm — packument only lists stable versions
npm view my-package --registry https://batlehub.example.com/proxy/my-npm

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

### 1.5 Registry support

Beta-channel gating applies to **local and hybrid mode** registries. It has no effect on proxy-only registries (where BatleHub does not control what versions exist upstream).

| Registry | Listing gated | Download gated |
|----------|--------------|---------------|
| npm | Yes (packument + version metadata) | Yes (tarball) |
| Cargo | Yes (sparse index) | Yes (.crate file) |
| Go modules | Yes (@v/list, @latest) | Yes (.zip) |
| RubyGems | Yes (versions endpoint, gem info) | Yes (.gem file) |
| Maven | Yes (maven-metadata.xml versions) | Yes (artifact) |
| Terraform modules | Yes (versions response) | Yes (artifact) |
| Terraform providers | Yes (versions response) | Yes (binary) |

> **Note:** Maven versions that are not valid semver (e.g. `1.0-SNAPSHOT`) are never treated as pre-releases and are always visible. SNAPSHOT gating would require a separate feature.

---

## 2. IP-Based Blocking

### 2.1 How it works

BatleHub counts violation events per IP address within a sliding time window. When the count exceeds a configurable threshold, the IP is automatically blocked for a configurable duration.

A **violation** is any response with a status code in the `trigger_on_status` list (default: 429 and 401). This means:

- An IP that repeatedly hits rate limits accumulates violations → auto-blocked.
- An IP that brute-forces auth tokens accumulates violations → auto-blocked.

Blocked IPs receive `403 Forbidden` with an `X-Block-Expires` header containing the Unix timestamp when the block lifts. The block check runs **before authentication**, so blocked IPs consume no auth resources.

The store is fail-open: if the backing store is unavailable, IPs are allowed through rather than hard-blocked.

### 2.2 Configuration

Add an `[ip_blocking]` section at the root of `config.toml` (not inside a registry block):

```toml
[ip_blocking]
enabled               = true
violation_threshold   = 10       # violations before auto-block
violation_window_secs = 300      # counting window (5 minutes)
ban_duration_secs     = 3600     # block duration (1 hour)
trigger_on_status     = [429, 401]
```

All fields have defaults; only `enabled = true` is required to activate the feature:

| Field | Default | Description |
|-------|---------|-------------|
| `enabled` | `false` | Enable IP blocking |
| `violation_threshold` | `10` | Violations in window before auto-block |
| `violation_window_secs` | `300` | Window duration in seconds |
| `ban_duration_secs` | `3600` | How long an auto-block lasts (seconds) |
| `trigger_on_status` | `[429, 401]` | HTTP status codes that count as violations |

#### Aggressive example (tight limits)

```toml
[ip_blocking]
enabled               = true
violation_threshold   = 3
violation_window_secs = 60
ban_duration_secs     = 86400      # 24 hours
trigger_on_status     = [429, 401, 403]
```

#### Behind a load balancer

If BatleHub sits behind a proxy or load balancer, real client IPs arrive via `X-Forwarded-For`. BatleHub reads the first IP from that header when present:

```
X-Forwarded-For: 1.2.3.4, 10.0.0.1
```

→ BatleHub uses `1.2.3.4` as the client IP. Make sure your load balancer sets this header correctly and strips any client-supplied values to prevent spoofing.

### 2.3 Manual block management

All endpoints require an `Admin` role token.

#### List blocked IPs

```sh
curl -H "Authorization: Bearer <admin-token>" \
  https://batlehub.example.com/api/v1/admin/ip-blocks
```

Response:

```json
[
  {
    "ip":         "1.2.3.4",
    "blocked_at": 1748304000,
    "unblock_at": 1748307600,
    "reason":     "auto"
  },
  {
    "ip":         "5.6.7.8",
    "blocked_at": 1748300000,
    "unblock_at": 1748386400,
    "reason":     "known bad actor"
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
|-------|----------|-------------|
| `ip` | Yes | IP address to block |
| `reason` | No | Human-readable reason (stored for audit) |
| `duration_secs` | No | Block duration; defaults to `3600` |

Returns `204 No Content`.

#### Unblock an IP

```sh
curl -s -X DELETE \
  -H "Authorization: Bearer <admin-token>" \
  https://batlehub.example.com/api/v1/admin/ip-blocks/1.2.3.4
```

Returns `204 No Content`. Auto-blocking will resume if the IP continues to trigger violations.

### 2.4 Storage backends

IP violation counters and block records are stored in the same backend as the rate-limit store, selected from `config.cache.cache_type`:

| `cache_type` | Violation counters | Block list | Survives restart |
|-------------|-------------------|-----------|-----------------|
| `memory` (default) | In-process HashMap | In-process HashMap | No |
| `postgres` | `ip_violation_counters` table | `ip_blocks` table | Yes |
| `redis` | `violation:{ip}` key with TTL | `block:{ip}` key with TTL | Yes (if Redis persists) |

For production, use `postgres` or `redis` so blocks survive restarts and are shared across multiple BatleHub instances.

---

## 4. Combining features

The two features are independent and can be used together. A typical setup for a private registry with beta testing:

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

In this setup:
1. Rate limiting protects against high-frequency abuse → violations count towards IP blocking.
2. Auth failures (401) also count → brute-force attempts trigger auto-blocking.
3. Beta releases are only visible to members added via the admin API.

---

## 3. Team Namespaces & Package Visibility

### 3.1 How it works

A **team namespace** maps a package name prefix to an auth-provider group. Once claimed, only members of that group — plus admins — can publish packages whose name starts with `prefix` or `prefix/`.

**Example:** claiming prefix `frontend` for group `oidc:frontend-team` means that publishing `frontend/utils`, `frontend/components`, or a package named exactly `frontend` requires the publisher's identity to carry the `oidc:frontend-team` group claim. Publishing `backend/api` is unaffected.

Groups are not managed inside BatleHub. Membership is read from the `groups` claim delivered by the configured auth provider (OIDC, Kubernetes, or static token) on every request — no separate sync is required.

**Package visibility** controls who can _download_ a package, regardless of who published it:

| Visibility | Who can download |
|------------|-----------------|
| `public` (default) | Everyone, including unauthenticated users |
| `internal` | Any authenticated user |
| `team` | Members of the group that owns the namespace |

Visibility is **package-level**: all versions of a package share the same setting. When a new version is published, it inherits the existing package visibility automatically. Admins bypass all visibility checks and can always download any package.

There is **no configuration required** — namespace claims and visibility are managed entirely at runtime via the admin API. No TOML changes, no server restart.

### 3.2 Managing namespace claims

All endpoints require an `Admin` role token.

#### List claims

```sh
curl -H "Authorization: Bearer <admin-token>" \
  https://batlehub.example.com/api/v1/admin/registries/internal-npm/namespaces
```

```json
[
  { "registry": "internal-npm", "prefix": "frontend", "group_id": "oidc:frontend-team", "claimed_by": "admin" },
  { "registry": "internal-npm", "prefix": "backend",  "group_id": "oidc:backend-team",  "claimed_by": null }
]
```

#### Claim a namespace

```sh
curl -X POST \
  -H "Authorization: Bearer <admin-token>" \
  -H "Content-Type: application/json" \
  -d '{"prefix":"frontend","group_id":"oidc:frontend-team","claimed_by":"admin"}' \
  https://batlehub.example.com/api/v1/admin/registries/internal-npm/namespaces
```

- `prefix` — package name prefix (no trailing slash). May contain slashes for deeply nested scopes (e.g. `org/team`).
- `group_id` — the group name as it appears in the auth provider's claim (e.g. `oidc:frontend-team`, `kubernetes:ci-bots`).
- `claimed_by` — optional free-text note, typically the admin who created the claim.

Returns `204 No Content` on success; `409 Conflict` if the prefix is already claimed.

#### Release a claim

```sh
curl -X DELETE \
  -H "Authorization: Bearer <admin-token>" \
  https://batlehub.example.com/api/v1/admin/registries/internal-npm/namespaces/frontend
```

Prefixes containing slashes are passed verbatim in the URL path:

```sh
curl -X DELETE \
  -H "Authorization: Bearer <admin-token>" \
  https://batlehub.example.com/api/v1/admin/registries/internal-npm/namespaces/org/team
```

Returns `204 No Content` even if the claim did not exist.

### 3.3 Package visibility

#### Get visibility

```sh
curl -H "Authorization: Bearer <admin-token>" \
  https://batlehub.example.com/api/v1/admin/registries/internal-npm/packages/frontend%2Futils/visibility
```

```json
{ "visibility": "public" }
```

Package names containing slashes must be percent-encoded (`/` → `%2F`) in the URL.

#### Set visibility

```sh
# Restrict to team members only
curl -X PUT \
  -H "Authorization: Bearer <admin-token>" \
  -H "Content-Type: application/json" \
  -d '{"visibility":"team"}' \
  https://batlehub.example.com/api/v1/admin/registries/internal-npm/packages/frontend%2Futils/visibility

# Open to any authenticated user
curl -X PUT \
  -H "Authorization: Bearer <admin-token>" \
  -H "Content-Type: application/json" \
  -d '{"visibility":"internal"}' \
  https://batlehub.example.com/api/v1/admin/registries/internal-npm/packages/frontend%2Futils/visibility

# Restore public access
curl -X PUT \
  -H "Authorization: Bearer <admin-token>" \
  -H "Content-Type: application/json" \
  -d '{"visibility":"public"}' \
  https://batlehub.example.com/api/v1/admin/registries/internal-npm/packages/frontend%2Futils/visibility
```

Accepted values: `public`, `internal`, `team`. Returns `204 No Content` on success; `404 Not Found` if the package has never been published. Returns `400 Bad Request` for unknown visibility values.

#### What happens at download time

When a download or metadata request arrives for a package with non-public visibility, BatleHub evaluates:

1. **Is the caller an admin?** → allow.
2. **Is the package `public`?** → allow.
3. **Is the package `internal`?** → allow if the caller is authenticated (any role ≥ User).
4. **Is the package `team`?** → look up the namespace claim for the package; allow only if the caller's group claims include the owning group. If no claim is found, deny all non-admin access.

Download attempts that fail visibility checks receive `403 Forbidden`. The index / metadata endpoints (sparse Cargo index, npm packument, Go `@v/list`, etc.) are also gated by the same check — a user who cannot download a package also cannot see it in version listings.

### 3.4 Registry support

Team namespaces and package visibility apply to all registry types that support `local` or `hybrid` mode:

| Registry | Namespace prefix example |
|----------|--------------------------|
| npm | `@scope` or `team/` |
| Cargo | `my-crate` (exact) or `my-prefix/` |
| Go modules | `github.com/org/` |
| RubyGems | `my-gem` |
| Maven | `com.example.group:` |
| Terraform modules | `namespace/module/provider` |
| Terraform providers | `namespace/type` |
| Composer | `vendor/` |
| OpenVSX / VSIX | `publisher.name` |

Namespace prefixes are matched using a longest-prefix rule: if both `frontend` and `frontend/ui` are claimed, `frontend/ui/button` is governed by the `frontend/ui` claim.
