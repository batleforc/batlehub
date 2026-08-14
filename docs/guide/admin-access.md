# Access & audit

## Team Namespaces & Package Visibility {#team-namespaces}

Team namespaces let you assign a package name prefix within a registry to an auth-provider group. Only group members — and admins — may publish packages under that prefix. Package visibility independently controls who can download a package.

This feature requires no TOML changes and no server restart — claims and visibility are managed entirely via the admin API.

For the full reference (visibility levels, download-time enforcement, longest-prefix rule, registry support matrix) see the [Access Control guide](/guide/access-control#team-namespaces).

### Managing namespace claims

```sh
# List claims for a registry
curl -H "Authorization: Bearer <admin-token>" \
  https://batlehub.example.com/api/v1/admin/registries/internal-npm/namespaces

# Claim a prefix for a group
curl -X POST \
  -H "Authorization: Bearer <admin-token>" \
  -H "Content-Type: application/json" \
  -d '{"prefix":"frontend","group_id":"oidc:frontend-team","claimed_by":"admin"}' \
  https://batlehub.example.com/api/v1/admin/registries/internal-npm/namespaces

# Release a claim (prefix may contain slashes, passed verbatim in the path)
curl -X DELETE \
  -H "Authorization: Bearer <admin-token>" \
  https://batlehub.example.com/api/v1/admin/registries/internal-npm/namespaces/frontend
```

### Managing package visibility

Visibility is package-level — all versions share the same setting. Accepted values: `public` (default), `internal`, `team`.

```sh
# Read current visibility
curl -H "Authorization: Bearer <admin-token>" \
  https://batlehub.example.com/api/v1/admin/registries/internal-npm/packages/frontend%2Futils/visibility

# Restrict to team members only
curl -X PUT \
  -H "Authorization: Bearer <admin-token>" \
  -H "Content-Type: application/json" \
  -d '{"visibility":"team"}' \
  https://batlehub.example.com/api/v1/admin/registries/internal-npm/packages/frontend%2Futils/visibility
```

Package names containing slashes must be percent-encoded in the URL (`/` → `%2F`).

---

## Audit log {#audit-log}

Every access-control decision (allow or deny) is recorded in PostgreSQL.

```sh
# Last 50 decisions across all registries
curl -H "Authorization: Bearer <admin-token>" \
  "http://localhost:8080/api/v1/admin/audit-log?limit=50"

# Filter by registry and outcome
curl -H "Authorization: Bearer <admin-token>" \
  "http://localhost:8080/api/v1/admin/audit-log?registry=npm&outcome=deny&limit=100"
```

Example entry:

```json
{
  "id": "01j...",
  "timestamp": "2025-05-22T10:00:00Z",
  "registry": "npm",
  "package": "lodash",
  "version": "4.17.21",
  "user_id": "ci",
  "role": "user",
  "outcome": "allow",
  "rule": null
}
```

---

## Beta/Pre-Release Channel {#beta-channel}

Gate pre-release versions (e.g. `1.0.0-beta.1`) to specific users or groups. Non-members see only stable versions and get 404 on pre-release artifact downloads.

Enable per registry:

```toml
[registries.beta_channel]
enabled = true
```

Manage members at runtime:

```sh
# Add a user
curl -s -X POST \
  -H "Authorization: Bearer <admin-token>" \
  -H "Content-Type: application/json" \
  -d '{"principal_type":"user","principal_id":"alice"}' \
  http://localhost:8080/api/v1/admin/registries/my-npm/beta-channel

# List members
curl -H "Authorization: Bearer <admin-token>" \
  http://localhost:8080/api/v1/admin/registries/my-npm/beta-channel

# Remove a member
curl -X DELETE -H "Authorization: Bearer <admin-token>" \
  http://localhost:8080/api/v1/admin/registries/my-npm/beta-channel/user/alice
```

See the [Access Control guide](/guide/access-control#beta-channel) for the full reference, including group membership, per-registry support table, and user-facing behaviour.

---

## IP-Based Blocking {#ip-blocking}

Automatically block IPs that trigger too many violations (rate-limit hits, auth failures) within a time window.

```toml
[ip_blocking]
enabled               = true
violation_threshold   = 10
violation_window_secs = 300      # 5-minute window
ban_duration_secs     = 3600     # 1-hour block
trigger_on_status     = [429, 401]
```

Manage blocks manually:

```sh
# List blocked IPs
curl -H "Authorization: Bearer <admin-token>" \
  http://localhost:8080/api/v1/admin/ip-blocks

# Block an IP
curl -s -X POST \
  -H "Authorization: Bearer <admin-token>" \
  -H "Content-Type: application/json" \
  -d '{"ip":"1.2.3.4","reason":"bad actor","duration_secs":86400}' \
  http://localhost:8080/api/v1/admin/ip-blocks

# Unblock
curl -s -X DELETE \
  -H "Authorization: Bearer <admin-token>" \
  http://localhost:8080/api/v1/admin/ip-blocks/1.2.3.4
```

Blocked IPs receive `403 Forbidden` with `X-Block-Expires`. The check runs before authentication. Violation counts and blocks are stored in the same backend as the rate-limit store (`memory` / `postgres` / `redis`).

See the [Access Control guide](/guide/access-control#ip-blocking) for the full reference including load-balancer setup and storage backend comparison.
