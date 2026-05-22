# Administration

This page covers everything an administrator needs to operate BatleHub: configuration, storage, auth providers, registry management, health monitoring, and cache cleanup.

For the complete TOML reference see [`docs/configuration.md`](https://github.com/batleforc/batlehub/blob/main/docs/configuration.md).

[[toc]]

---

## Configuration {#configuration}

BatleHub reads a single TOML file, defaulting to `config.toml` in the working directory. Override the path with `--config /path/to/config.toml`.

### Loading order

1. TOML file is parsed.
2. Environment variables `PROXY_CACHE__<SECTION>__<FIELD>` are applied on top.
3. Registry names and types are validated.

### Key environment variable overrides

| Variable | Config field |
|----------|-------------|
| `PROXY_CACHE__SERVER__PORT` | `server.port` |
| `PROXY_CACHE__DATABASE__URL` | `database.url` |
| `PROXY_CACHE__STORAGE__PATH` | `storage.path` |
| `PROXY_CACHE__OTEL__ENDPOINT` | `otel.endpoint` |

### Minimal production config

```toml
[server]
host = "0.0.0.0"
port = 8080
static_dir = "/app/ui/dist"
cors_allowed_origins = ["https://batlehub.example.com"]

[database]
type = "postgresql"
url  = "postgresql://batlehub:changeme@postgres:5432/batlehub"

[[auth]]
type = "token"

[[auth.tokens]]
value   = "change-me-admin-token"
role    = "admin"
user_id = "admin"

[storage]
type = "filesystem"
path = "/var/cache/batlehub"

[[registries]]
type = "npm"
name = "npm"

[registries.rbac]
anonymous = ["releases:read", "source:read"]
```

---

### Registry modes

Every registry can run in one of three modes:

| Mode | Behaviour |
|------|-----------|
| `proxy` | Default. Forwards all requests to upstream; publishing is rejected. |
| `local` | BatleHub is the only source. No upstream needed. Teams publish directly. |
| `hybrid` | Local-first. Serves locally-published packages; falls back to upstream for everything else. |

```toml
[[registries]]
type = "cargo"
name = "internal"
mode = "local"         # or "hybrid"

[registries.rbac]
user  = ["source:read"]
admin = ["*"]
```

---

### Auth providers {#auth}

Auth providers are evaluated in declaration order. The first provider that recognises a credential wins. Requests with no matching credential are treated as `anonymous`.

#### Static tokens

```toml
[[auth]]
type = "token"

[[auth.tokens]]
value   = "ci-pipeline-token"
role    = "user"
user_id = "ci"
```

#### OIDC (Authentik, Keycloak, Dex, …)

```toml
[[auth]]
type         = "oidc"
issuer_url   = "https://sso.example.com/application/o/batlehub/"
client_id    = "batlehub"
client_secret = "client-secret"
redirect_uri = "https://batlehub.example.com/api/v1/auth/oidc/callback"
scopes       = ["openid", "profile", "email", "groups"]

user_id_claim = "preferred_username"
role_claim    = "groups"

[auth.role_mappings]
"authentik Admins" = "admin"
"proxy-users"      = "user"
```

#### Kubernetes service accounts

```toml
[[auth]]
type = "kubernetes"
# api_server, ca_cert_path, token_path all default to in-cluster values

[auth.role_mappings]
"system:serviceaccount:prod:ci-deployer" = "admin"
"system:serviceaccounts:staging"         = "user"
```

#### User-generated API tokens

Authenticated users (OIDC sessions) can generate short-lived tokens via the Web UI or API:

```sh
curl -X POST \
  -H "Authorization: Bearer <oidc-token>" \
  -H "Content-Type: application/json" \
  -d '{"name": "my-token", "expires_in_days": 30, "role": "user"}' \
  https://batlehub.example.com/api/v1/auth/tokens
```

The raw token value is returned **once** — save it immediately.

---

## Storage {#storage}

### Filesystem

```toml
[storage]
type = "filesystem"
path = "/var/cache/batlehub"
```

### S3-compatible (AWS S3, MinIO, RustFS)

```toml
[storage]
type   = "s3"
bucket = "batlehub-artifacts"
region = "us-east-1"

# For self-hosted S3 (MinIO, RustFS): set a custom endpoint
# endpoint = "http://rustfs:9900"

# Credentials (omit to use IAM role / instance profile on AWS)
# access_key_id     = "AKIAIOSFODNN7EXAMPLE"
# secret_access_key = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"
```

### Multi-backend storage

Different registries can use different backends — for example, filesystem for most registries and dedicated S3 for large GitHub release artifacts:

```toml
[storage]
type = "filesystem"
path = "/var/cache/batlehub"

[[storage.backends]]
name = "github-s3"
type = "s3"
bucket = "batlehub-github"
region = "us-east-1"

[[registries]]
type    = "github"
name    = "github"
storage = "github-s3"
```

### S3 with RustFS (self-hosted)

Start RustFS via the bundled Compose file, then create the bucket:

```sh
task compose:s3:db            # start RustFS + Postgres + Authentik
mc alias set local http://localhost:9900 rustfsadmin rustfsadmin
mc mb local/artifacts         # or: task compose:s3:bucket:create
task run:s3                   # run the server with the S3 config
```

---

## Health & Observability {#health}

### Health endpoint

```sh
curl -H "Authorization: Bearer <admin-token>" \
  http://localhost:8080/api/v1/admin/health
```

Returns per-registry status (upstream reachability, cache hit rate) and overall server status.

### Clear registry cache

Forces the next request for any package in the registry to re-fetch from upstream:

```sh
curl -X POST \
  -H "Authorization: Bearer <admin-token>" \
  http://localhost:8080/api/v1/admin/registries/npm/clear-cache
```

### OpenTelemetry (Jaeger, Tempo)

Enable distributed tracing by adding an `[otel]` block:

```toml
[otel]
endpoint = "http://jaeger:4317"
```

Start the full observability stack locally:

```sh
task compose:otel   # starts Postgres + server + Jaeger
```

Then open `http://localhost:16686` for the Jaeger UI.

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

Blocked packages return `403 Forbidden` to all clients, regardless of role.

```sh
curl -X POST \
  -H "Authorization: Bearer <admin-token>" \
  -H "Content-Type: application/json" \
  -d '{"registry": "npm", "name": "lodash", "version": "4.17.20"}' \
  http://localhost:8080/api/v1/admin/packages/block
```

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
