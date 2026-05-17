# Configuration Reference

proxy-cache is configured with a single TOML file. This document covers every option, how they interact, and includes copy-paste examples for common deployment scenarios.

---

## Table of Contents

1. [Quick Start](#1-quick-start)
2. [How Configuration Works](#2-how-configuration-works)
3. [Full Reference](#3-full-reference)
   - [server](#31-server)
   - [database](#32-database)
   - [auth](#33-auth)
   - [storage](#34-storage)
   - [registries](#35-registries)
   - [otel](#36-otel-optional)
4. [Permissions Reference](#4-permissions-reference)
5. [Environment Variable Overrides](#5-environment-variable-overrides)
6. [Worked Examples](#6-worked-examples)
7. [CLI Reference](#7-cli-reference)
8. [User-Generated API Tokens](#8-user-generated-api-tokens)

---

## 1. Quick Start

Copy this into `config.toml`, start PostgreSQL, and run the server:

```toml
[server]
port = 8080

[database]
type = "postgresql"
url = "postgresql://proxy_cache:changeme@localhost:5432/proxy_cache"

[[auth]]
type = "token"

[[auth.tokens]]
value = "my-admin-token"
role = "admin"
user_id = "admin"

[storage]
type = "filesystem"
path = "./cache"

[[registries]]
type = "npm"
name = "npm"

[registries.rbac]
anonymous = ["releases:read", "source:read"]
user = ["releases:read", "source:read"]
admin = ["*"]
```

```sh
proxy-cache --config config.toml
```

Verify the server is running:

```sh
curl http://localhost:8080/api/openapi.json
```

Authenticated requests use a Bearer token:

```sh
curl -H "Authorization: Bearer my-admin-token" http://localhost:8080/...
```

---

## 2. How Configuration Works

### Loading order

1. The TOML file at the path given to `--config` is parsed (default: `config.toml` in the working directory).
2. Environment variables matching `PROXY_CACHE__<SECTION>__<FIELD>` are applied on top of the file values.
3. The config is validated: registry names must not be empty and registry types must be one of `github`, `npm`, `cargo`, `pypi`, `composer`.

### Auth evaluation order

The `[[auth]]` array is tried in declaration order. The first provider that recognises a credential wins and the request proceeds with that identity. If no provider matches, the request is treated as `anonymous`. Putting a token provider before OIDC means static tokens are checked first, which is slightly more efficient.

---

## 3. Full Reference

### 3.1 `[server]`

Controls the HTTP listener and optional SPA serving.

```toml
[server]
host = "0.0.0.0"        # default
port = 8080             # default
# static_dir = "./ui/dist"  # optional: serve the built Vue SPA from this path
```

| Field | Type | Default | Notes |
|---|---|---|---|
| `host` | string | `"0.0.0.0"` | Bind address |
| `port` | u16 | `8080` | TCP port |
| `static_dir` | string | — | Path to the built SPA; when set, the server serves the frontend at `/` |

---

### 3.2 `[database]`

proxy-cache uses PostgreSQL for storing registry metadata and user tokens.

```toml
[database]
type = "postgresql"
url = "postgresql://proxy_cache:changeme@localhost:5432/proxy_cache"
max_connections = 10    # default
```

| Field | Type | Default | Notes |
|---|---|---|---|
| `type` | string | — | Must be `"postgresql"` |
| `url` | string | — | Full PostgreSQL DSN including credentials |
| `max_connections` | u32 | `10` | Connection pool size |

The `url` field can be overridden at runtime via `PROXY_CACHE__DATABASE__URL` without touching the config file.

---

### 3.3 `[[auth]]`

An array of auth providers tried in declaration order. Three types are supported.

#### 3.3.1 Token auth (`type = "token"`)

Validates static bearer tokens defined in the config file. Useful for CI/CD pipelines and simple setups.

```toml
[[auth]]
type = "token"

[[auth.tokens]]
value = "my-ci-token"     # the bearer token value
role = "user"             # "admin", "user", or "anonymous"
user_id = "ci-bot"        # optional: display name in logs

[[auth.tokens]]
value = "my-admin-token"
role = "admin"
user_id = "admin"
```

| Field | Type | Required | Notes |
|---|---|---|---|
| `value` | string | yes | The Bearer token string |
| `role` | string | yes | `"admin"`, `"user"`, or `"anonymous"` |
| `user_id` | string | no | Used in audit logs |

> **Security note:** Token values are stored in plaintext in the config file. In production, inject them via environment variables or a secrets manager and reference them from there.

#### 3.3.2 OIDC auth (`type = "oidc"`)

Validates JWT Bearer tokens issued by any standards-compliant OIDC provider (Authentik, Keycloak, Dex, etc.). Optionally enables browser-based SSO login.

```toml
[[auth]]
type = "oidc"
# name = "oidc"           # default; must be unique when running multiple OIDC providers
issuer_url = "https://sso.example.com/application/o/proxy-cache/"
client_id = "proxy-cache"
# client_secret = "..."   # required for confidential clients
# redirect_uri = "https://proxy-cache.example.com/api/v1/auth/oidc/callback"
# frontend_url = ""       # default: same origin as the backend
scopes = ["openid", "profile", "email", "groups"]
user_id_claim = "preferred_username"   # default: "sub"
role_claim = "groups"                  # default: "role"

[auth.role_mappings]
"authentik Admins" = "admin"
"proxy-users"      = "user"
```

| Field | Type | Default | Notes |
|---|---|---|---|
| `name` | string | `"oidc"` | Provider name; becomes the group prefix (e.g. `"oidc:team-a"`). Must be unique across providers. |
| `issuer_url` | string | — | Base URL of the OIDC provider; `/.well-known/openid-configuration` is appended for endpoint discovery |
| `client_id` | string | — | OAuth2 client identifier |
| `client_secret` | string | — | Required for confidential clients; optional for public clients |
| `redirect_uri` | string | — | When set, enables browser SSO at `/api/v1/auth/oidc/callback` (default provider) or `/api/v1/auth/oidc/{name}/callback` (named providers). Must be registered with the OIDC provider. |
| `frontend_url` | string | `""` | After a successful SSO callback the browser is redirected to `{frontend_url}/?oidc_access_token=...`. Leave empty in production (same origin). Set to `http://localhost:5173` when running the Vite dev server separately. |
| `scopes` | string[] | `["openid","profile","email"]` | OAuth2 scopes to request |
| `user_id_claim` | string | `"sub"` | JWT claim used as the user identifier. `"preferred_username"` gives human-readable names from Authentik/Keycloak. |
| `role_claim` | string | `"role"` | JWT claim inspected for role mapping. May be a string or array of strings; the highest matching role wins. |
| `role_mappings` | map | `{}` | Maps JWT claim values to proxy roles (`"admin"`, `"user"`, `"anonymous"`). Values not present default to `anonymous`. |

**Group namespacing:** Claim values that appear as keys in `role_mappings` are stored as-is in the identity's group list. Claim values not in `role_mappings` are prefixed with `{name}:` (e.g. `"oidc:team-a"`). This allows the RBAC `groups` table to use `"*:team-a"` as a cross-provider wildcard.

**Running multiple OIDC providers:** Set a unique `name` on each. Their callback URLs will be `/api/v1/auth/oidc/{name}/callback`.

#### 3.3.3 Kubernetes auth (`type = "kubernetes"`)

Validates Kubernetes service account tokens via the Kubernetes TokenReview API. All fields default to the standard in-cluster mounted secrets and environment variables, so minimal configuration is needed when running inside a cluster.

```toml
[[auth]]
type = "kubernetes"
# name = "kubernetes"   # default

# All of the following default to in-cluster values:
# api_server   = "https://kubernetes.default.svc"
# ca_cert_path = "/var/run/secrets/kubernetes.io/serviceaccount/ca.crt"
# token_path   = "/var/run/secrets/kubernetes.io/serviceaccount/token"
# audiences    = ["proxy-cache"]

[auth.role_mappings]
"system:serviceaccount:prod:ci-deployer" = "admin"
"system:serviceaccounts:staging"         = "user"
"system:serviceaccounts"                 = "anonymous"
```

| Field | Type | Default | Notes |
|---|---|---|---|
| `name` | string | `"kubernetes"` | Provider name; becomes the group prefix |
| `api_server` | string | from `KUBERNETES_SERVICE_HOST` / `KUBERNETES_SERVICE_PORT` env | Kubernetes API server URL |
| `ca_cert_path` | string | `/var/run/secrets/kubernetes.io/serviceaccount/ca.crt` | CA cert for API server TLS verification |
| `token_path` | string | `/var/run/secrets/kubernetes.io/serviceaccount/token` | proxy-cache's own service account token for TokenReview calls; re-read each request to handle automatic rotation |
| `audiences` | string[] | `["proxy-cache"]` | Audiences sent in the TokenReview request |
| `role_mappings` | map | `{}` | Maps Kubernetes usernames or group names to proxy roles |

**Role mapping keys:** Kubernetes sets `username: "system:serviceaccount:<namespace>:<name>"` and `groups: ["system:serviceaccounts", "system:serviceaccounts:<namespace>", ...]`. When a token matches multiple keys, the highest role wins.

---

### 3.4 `[storage]`

Two formats are supported: single-backend (simpler, supports env-var overrides) and multi-backend (allows per-registry routing).

#### Single backend

```toml
# Filesystem
[storage]
type = "filesystem"
path = "./cache"

# S3 (or S3-compatible: MinIO, RustFS, etc.)
[storage]
type = "s3"
bucket = "my-artifacts"
region = "us-east-1"
prefix = "proxy-cache/"         # optional, default: none
endpoint_url = "http://minio:9000"  # optional: omit for real AWS
force_path_style = true         # optional: required for MinIO and RustFS
```

**Filesystem fields:**

| Field | Type | Required | Notes |
|---|---|---|---|
| `path` | string | yes | Directory for cached files; created if it does not exist |

**S3 fields:**

| Field | Type | Required | Notes |
|---|---|---|---|
| `bucket` | string | yes | S3 bucket name |
| `region` | string | yes | AWS region (e.g. `"us-east-1"`) |
| `prefix` | string | no | Key prefix for all stored objects |
| `endpoint_url` | string | no | Custom endpoint for S3-compatible stores |
| `force_path_style` | bool | no | Required for MinIO, RustFS, and other S3-compatible stores that use path-style URLs |

S3 credentials are sourced from the standard AWS SDK credential chain: `AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY` environment variables, `~/.aws/credentials`, EC2/ECS instance metadata, and so on.

#### Multi-backend

Use this when different registries should store artifacts in different backends.

```toml
[storage]
default = "primary"           # required: name of the fallback backend

[[storage.backends]]
name = "primary"
type = "filesystem"
path = "./cache"

[[storage.backends]]
name = "s3-artifacts"
type = "s3"
bucket = "release-artifacts"
region = "eu-west-1"
```

Then assign a registry to a specific backend with the `storage` field:

```toml
[[registries]]
type = "github"
name = "github"
storage = "s3-artifacts"    # this registry uses s3-artifacts; others use "primary"
```

> **Note:** Environment variable overrides for storage fields (`PROXY_CACHE__STORAGE__PATH`, etc.) only work with the single-backend form. Multi-backend configs must be changed in the file.

---

### 3.5 `[[registries]]`

An array of package registry proxies. Each entry configures one registry endpoint.

```toml
[[registries]]
type = "cargo"
name = "cargo"
# upstreams = ["https://crates.io"]   # default for cargo
# index_url = "https://index.crates.io"  # default; set for self-hosted registries
# storage = "backend-name"            # optional: use a named storage backend

[registries.cache]
metadata_ttl_secs = 300     # default: 300 (5 minutes)
artifact_strategy = "permanent"  # default; "ttl" re-checks artifacts after metadata TTL

[registries.rbac]
anonymous = []
user = ["releases:read", "source:read"]
admin = ["*"]

[registries.rbac.groups]
"team-a" = ["releases:read", "source:read"]
"*:ops"  = ["*"]   # wildcard: any provider's "ops" group

[[registries.rules]]
kind = "release_age_gate"
min_age_secs = 3600         # default: 3600 (1 hour)
bypass_roles = ["admin"]

# [[registries.rules]]
# kind = "require_signed_release"
# enabled = true
```

**Top-level fields:**

| Field | Type | Required | Notes |
|---|---|---|---|
| `type` | string | yes | `"github"`, `"npm"`, `"cargo"`, `"pypi"`, `"composer"` |
| `name` | string | yes | Unique identifier; used in proxy URL paths |
| `upstreams` | string[] | no | Upstream URLs tried in order on cache miss; 404 from one falls through to the next. Defaults to the registry's built-in URL. |
| `index_url` | string | no | Cargo only: sparse crate index URL. Defaults to `https://index.crates.io`. Required for self-hosted Gitea/Forgejo registries. |
| `storage` | string | no | Name of the storage backend. Must match a `[[storage.backends]]` name. Omit to use the default backend. |

**`[registries.cache]` fields:**

| Field | Type | Default | Notes |
|---|---|---|---|
| `metadata_ttl_secs` | u64 | `300` | How long release metadata (version lists, release info) is cached in seconds |
| `artifact_strategy` | string | `"permanent"` | `"permanent"`: once an artifact is cached it is never re-fetched. `"ttl"`: artifacts may be re-checked after the metadata TTL. |

**`[registries.rbac]` fields:**

| Field | Type | Default | Notes |
|---|---|---|---|
| `anonymous` | string[] | `[]` | Permissions granted to unauthenticated requests |
| `user` | string[] | `[]` | Permissions granted to authenticated users (inherits anonymous perms) |
| `admin` | string[] | `[]` | Permissions granted to admins (inherits user and anonymous perms) |
| `groups` | map | `{}` | Dynamic group permissions (see [Section 4](#4-permissions-reference)) |

**`[[registries.rules]]` — Release age gate:**

| Field | Type | Default | Notes |
|---|---|---|---|
| `kind` | string | — | Must be `"release_age_gate"` |
| `min_age_secs` | u64 | `3600` | Releases younger than this are blocked. If a release has no publish timestamp the gate is skipped (allow). |
| `bypass_roles` | string[] | `[]` | Roles that skip the gate (e.g. `["admin"]`) |

**`[[registries.rules]]` — Require signed release:**

| Field | Type | Default | Notes |
|---|---|---|---|
| `kind` | string | — | Must be `"require_signed_release"` |
| `enabled` | bool | `false` | When true, blocks releases that do not have a verified signature |

---

### 3.6 `[otel]` (optional)

Enables OpenTelemetry distributed tracing via OTLP gRPC.

```toml
[otel]
endpoint = "http://localhost:4317"
service_name = "proxy-cache"   # default
```

| Field | Type | Default | Notes |
|---|---|---|---|
| `endpoint` | string | — | OTLP gRPC endpoint |
| `service_name` | string | `"proxy-cache"` | Service name reported in traces |

The entire section can be enabled without a config file change by setting `PROXY_CACHE__OTEL__ENDPOINT` — the section is created automatically if the env var is present.

---

## 4. Permissions Reference

### Roles

Three built-in roles are evaluated with inheritance: `admin` inherits all `user` permissions, `user` inherits all `anonymous` permissions. This means if `anonymous` can do `releases:read`, admins can too without repeating the permission.

| Role | Description |
|---|---|
| `anonymous` | Unauthenticated request, or no auth provider matched |
| `user` | Successfully authenticated via any provider |
| `admin` | Full access |

### Permission strings

| Permission | Meaning |
|---|---|
| `releases:read` | List releases and download release assets |
| `source:read` | Download source tarballs |
| `*` | All permissions (wildcard) |

### Group-based permissions

Groups supplement role permissions — a request passes if it satisfies either the role check or any group check. Permissions from roles and groups are additive (union).

Group names in `[registries.rbac.groups]` are matched against the namespaced group strings produced by auth providers:

- **Exact match:** `"oidc:team-a"` — only matches `team-a` from the provider named `"oidc"`
- **Wildcard prefix:** `"*:team-a"` — matches `team-a` from any provider (`oidc:team-a`, `kubernetes:team-a`, etc.)

Example:

```toml
[registries.rbac.groups]
"oidc:developers" = ["releases:read", "source:read"]
"*:ops"           = ["*"]
```

---

## 5. Environment Variable Overrides

Environment variables override config file values at startup. They follow the `PROXY_CACHE__<SECTION>__<FIELD>` convention (double-underscore separator).

| Variable | Config field | Notes |
|---|---|---|
| `PROXY_CACHE__SERVER__HOST` | `server.host` | |
| `PROXY_CACHE__SERVER__PORT` | `server.port` | Parsed as u16 |
| `PROXY_CACHE__SERVER__STATIC_DIR` | `server.static_dir` | |
| `PROXY_CACHE__DATABASE__URL` | `database.url` | |
| `PROXY_CACHE__DATABASE__MAX_CONNECTIONS` | `database.max_connections` | Parsed as u32 |
| `PROXY_CACHE__STORAGE__PATH` | `storage.path` | Single filesystem backend only |
| `PROXY_CACHE__STORAGE__BUCKET` | `storage.bucket` | Single S3 backend only |
| `PROXY_CACHE__STORAGE__REGION` | `storage.region` | Single S3 backend only |
| `PROXY_CACHE__STORAGE__ENDPOINT_URL` | `storage.endpoint_url` | Single S3 backend only |
| `PROXY_CACHE__OTEL__ENDPOINT` | `otel.endpoint` | Creates the `[otel]` section if absent |
| `PROXY_CACHE__OTEL__SERVICE_NAME` | `otel.service_name` | |

> Storage env-var overrides only work with the **single-backend** `[storage]` form. Multi-backend configs (`[[storage.backends]]`) must be changed in the file.

---

## 6. Worked Examples

### 6.1 Local Development

Minimal setup for local development: static token auth, filesystem cache, npm and Cargo open to anonymous reads.

```toml
[server]
port = 8080

[database]
type = "postgresql"
url = "postgresql://proxy_cache:changeme@localhost:5432/proxy_cache"

[[auth]]
type = "token"

[[auth.tokens]]
value = "dev-admin-token"
role = "admin"
user_id = "admin"

[storage]
type = "filesystem"
path = "./tmp/cache"

[[registries]]
type = "npm"
name = "npm"

[registries.rbac]
anonymous = ["releases:read", "source:read"]
user = ["releases:read", "source:read"]
admin = ["*"]

[[registries]]
type = "cargo"
name = "cargo"

[registries.rbac]
anonymous = ["releases:read", "source:read"]
user = ["releases:read", "source:read"]
admin = ["*"]
```

### 6.2 Production with OIDC (Authentik)

OIDC SSO via Authentik, GitHub registry restricted to authenticated users, release age gate to prevent downloading packages within the first hour of release.

```toml
[server]
host = "0.0.0.0"
port = 8080
static_dir = "/app/ui/dist"

[database]
type = "postgresql"
url = "postgresql://proxy_cache:changeme@db:5432/proxy_cache"

[[auth]]
type = "oidc"
issuer_url = "https://sso.example.com/application/o/proxy-cache/"
client_id = "proxy-cache"
client_secret = "my-client-secret"
redirect_uri = "https://proxy-cache.example.com/api/v1/auth/oidc/callback"
scopes = ["openid", "profile", "email", "groups"]
user_id_claim = "preferred_username"
role_claim = "groups"

[auth.role_mappings]
"authentik Admins" = "admin"
"proxy-users"      = "user"

# Static token for CI pipelines that can't do OIDC
[[auth]]
type = "token"

[[auth.tokens]]
value = "ci-pipeline-token"
role = "user"
user_id = "ci"

[storage]
type = "filesystem"
path = "/data/cache"

[[registries]]
type = "github"
name = "github"

[registries.rbac]
anonymous = []
user = ["releases:read", "source:read"]
admin = ["*"]

[registries.rbac.groups]
"oidc:developers" = ["releases:read", "source:read"]
"*:ops"           = ["*"]

[[registries.rules]]
kind = "release_age_gate"
min_age_secs = 3600
bypass_roles = ["admin"]
```

### 6.3 Kubernetes Deployment

Kubernetes service account auth with in-cluster defaults, S3 storage with credentials from environment variables.

```toml
[server]
port = 8080
static_dir = "/app/ui/dist"

[database]
type = "postgresql"
url = "postgresql://proxy_cache:changeme@postgres-svc:5432/proxy_cache"

[[auth]]
type = "kubernetes"
# api_server, ca_cert_path, and token_path all default to in-cluster values

[auth.role_mappings]
"system:serviceaccount:prod:ci-deployer"  = "admin"
"system:serviceaccounts:staging"          = "user"
"system:serviceaccounts:dev"              = "user"

[storage]
type = "s3"
bucket = "proxy-cache-artifacts"
region = "us-east-1"
# AWS credentials come from the pod's IAM role or AWS_ACCESS_KEY_ID / AWS_SECRET_ACCESS_KEY

[[registries]]
type = "npm"
name = "npm"

[registries.rbac]
anonymous = []
user = ["releases:read", "source:read"]
admin = ["*"]

[[registries]]
type = "github"
name = "github"

[registries.rbac]
anonymous = []
user = ["releases:read"]
admin = ["*"]
```

proxy-cache's ServiceAccount needs permission to call the Kubernetes TokenReview API:

```yaml
apiVersion: rbac.authorization.k8s.io/v1
kind: ClusterRole
metadata:
  name: proxy-cache-tokenreview
rules:
  - apiGroups: ["authentication.k8s.io"]
    resources: ["tokenreviews"]
    verbs: ["create"]
---
apiVersion: rbac.authorization.k8s.io/v1
kind: ClusterRoleBinding
metadata:
  name: proxy-cache-tokenreview
roleRef:
  apiGroup: rbac.authorization.k8s.io
  kind: ClusterRole
  name: proxy-cache-tokenreview
subjects:
  - kind: ServiceAccount
    name: proxy-cache
    namespace: proxy-cache
```

### 6.4 Multi-Backend Storage

Default filesystem backend for all registries, dedicated S3 backend for large GitHub release artifacts.

```toml
[server]
port = 8080

[database]
type = "postgresql"
url = "postgresql://proxy_cache:changeme@localhost:5432/proxy_cache"

[[auth]]
type = "token"

[[auth.tokens]]
value = "admin-token"
role = "admin"

[storage]
default = "local"

[[storage.backends]]
name = "local"
type = "filesystem"
path = "./cache"

[[storage.backends]]
name = "s3-releases"
type = "s3"
bucket = "github-releases"
region = "us-east-1"

[[registries]]
type = "github"
name = "github"
storage = "s3-releases"       # large release assets go to S3

[registries.rbac]
anonymous = []
user = ["releases:read", "source:read"]
admin = ["*"]

[[registries]]
type = "npm"
name = "npm"
# storage not set — uses the "local" default backend

[registries.rbac]
anonymous = ["releases:read", "source:read"]
user = ["releases:read", "source:read"]
admin = ["*"]
```

---

## 7. CLI Reference

```
proxy-cache --config config.toml          # start the server (default: config.toml)
proxy-cache dump-spec                     # print the OpenAPI JSON spec to stdout
```

Redirect the spec to a file for use with code generators:

```sh
proxy-cache dump-spec > openapi.json
```

---

## 8. User-Generated API Tokens

Users authenticated via OIDC can create personal long-lived API tokens without going through SSO each time. This is the recommended approach for CI/CD pipelines when Kubernetes service account auth is not available.

```sh
# Create a token (valid for 30 days, cannot exceed creator's role)
curl -X POST https://proxy-cache.example.com/api/v1/auth/tokens \
  -H "Authorization: Bearer <oidc-access-token>" \
  -H "Content-Type: application/json" \
  -d '{"name": "ci-token", "expires_in_days": 30}'

# List active tokens
curl https://proxy-cache.example.com/api/v1/auth/tokens \
  -H "Authorization: Bearer <oidc-access-token>"

# Revoke a token
curl -X DELETE https://proxy-cache.example.com/api/v1/auth/tokens/<token-id> \
  -H "Authorization: Bearer <oidc-access-token>"
```

Key properties:
- Token values are shown **once** at creation time; store them securely.
- A token's role cannot exceed the role of the user who created it.
- Token auth (`type = "token"`) in the config file and user-generated tokens are two separate mechanisms; user-generated tokens are always available to OIDC-authenticated users with no extra `[[auth]]` entry needed.
