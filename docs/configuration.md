# Configuration Reference

batlehub is configured with a single TOML file. This document covers every option, how they interact, and includes copy-paste examples for common deployment scenarios.

---

## Table of Contents

1. [Quick Start](#1-quick-start)
2. [How Configuration Works](#2-how-configuration-works)
3. [Full Reference](#3-full-reference)
   - [server](#31-server)
   - [database](#32-database)
   - [cache](#32a-cache)
   - [auth](#33-auth)
     - [Token auth](#331-token-auth-type--token)
     - [OIDC auth](#332-oidc-auth-type--oidc)
     - [Kubernetes auth](#333-kubernetes-auth-type--kubernetes)
     - [Actions OIDC auth](#334-actions-oidc-auth-type--actions-oidc)
   - [storage](#34-storage)
   - [registries](#35-registries)
     - [rate_limit](#rate_limit)
     - [beta_channel](#registriesbeta_channel)
   - [ip_blocking](#36-ip_blocking-optional)
   - [otel](#37-otel-optional)
   - [proxy](#38-proxy-optional)
4. [Permissions Reference](#4-permissions-reference)
5. [Environment Variable Overrides](#5-environment-variable-overrides)
6. [Worked Examples](#6-worked-examples)
   - [6.1 Local Development](#61-local-development)
   - [6.2 Production with OIDC](#62-production-with-oidc-authentik)
   - [6.3 Kubernetes Deployment](#63-kubernetes-deployment)
   - [6.4 Go Module Proxy](#64-go-module-proxy)
   - [6.5 Self-Hosted Private Registries](#65-self-hosted-private-registries)
   - [6.6 Private Cargo Registry (local / hybrid mode)](#66-private-cargo-registry-local--hybrid-mode)
   - [6.7 Private npm Registry (local / hybrid mode)](#67-private-npm-registry-local--hybrid-mode)
   - [6.8 Private VS Code Extension Registry (local / hybrid mode)](#68-private-vs-code-extension-registry-local--hybrid-mode)
   - [6.9 Private Go Module Proxy (local / hybrid mode)](#69-private-go-module-proxy-local--hybrid-mode)
   - [6.10 Multi-Backend Storage](#610-multi-backend-storage)
   - [6.11 Terraform Provider Cache](#611-terraform-provider-cache)
   - [6.12 Private Maven Registry (local / hybrid mode)](#612-private-maven-registry-local--hybrid-mode)
   - [6.13 Private Terraform Registry (local / hybrid mode)](#613-private-terraform-registry-local--hybrid-mode)
   - [6.14 Rate Limiting — Per-User + Per-Group](#614-rate-limiting)
   - [6.15 Private Composer Registry (local / hybrid mode)](#615-private-composer-registry-local--hybrid-mode)
   - [6.16 Corporate HTTP Proxy (air-gapped environments)](#616-corporate-http-proxy-air-gapped-environments)
7. [CLI Reference](#7-cli-reference)
8. [User-Generated API Tokens](#8-user-generated-api-tokens)
9. [Hot Reload & Dynamic Config](#9-hot-reload--dynamic-config)
   - [9.1 File Watcher](#91-file-watcher)
   - [9.2 API Endpoints](#92-api-endpoints)
   - [9.3 Global Admin Banner](#93-global-admin-banner)
10. [Self-Hosted / Private Registries](#10-self-hosted--private-registries)
11. [SBOM Generation](#11-sbom-generation)

---

## 1. Quick Start

Copy this into `config.toml`, start PostgreSQL, and run the server:

```toml
[server]
port = 8080

[database]
type = "postgresql"
url = "postgresql://batlehub:changeme@localhost:5432/batlehub"

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
batlehub --config config.toml
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
3. The config is validated: registry names must not be empty and registry types must be one of `github`, `npm`, `cargo`, `openvsx`, `vscode-marketplace`, `goproxy`, `maven`, `terraform`, `rubygems`, `composer`, `pypi`, `conda`.

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

batlehub uses PostgreSQL for storing registry metadata and user tokens.

```toml
[database]
type = "postgresql"
url = "postgresql://batlehub:changeme@localhost:5432/batlehub"
max_connections = 10    # default
```

| Field | Type | Default | Notes |
|---|---|---|---|
| `type` | string | — | Must be `"postgresql"` |
| `url` | string | — | Full PostgreSQL DSN including credentials |
| `max_connections` | u32 | `10` | Connection pool size |

The `url` field can be overridden at runtime via `PROXY_CACHE__DATABASE__URL` without touching the config file.

---

### 3.2a `[cache]`

Selects the storage backend for **metadata cache entries** and **rate-limit counters**. Both subsystems share this backend so a single configuration change affects them together.

```toml
# In-process memory (default — no extra infrastructure required)
[cache]
type = "memory"

# PostgreSQL — persistent across restarts, shared across replicas
[cache]
type = "postgres"

# Redis — persistent, shared, TTL-based eviction
[cache]
type = "redis"
url  = "redis://localhost:6379"
```

| Field | Type | Default | Notes |
|---|---|---|---|
| `type` | string | `"memory"` | `"memory"`, `"postgres"`, or `"redis"` |
| `url` | string | — | Redis connection URL; required when `type = "redis"`. Format: `redis://[:<password>@]<host>[:<port>][/<db>]` or `rediss://…` for TLS. |

#### Backend comparison

| Backend | Persistence | Shared across replicas | Extra infra | Best for |
|---------|:-----------:|:---------------------:|:-----------:|---------|
| `memory` | No — resets on restart | No | None | Local dev, single-node |
| `postgres` | Yes | Yes | None (uses the existing `[database]`) | Production, multi-replica |
| `redis` | Yes | Yes | Redis cluster | High-throughput production |

> **`memory` is the default** and requires no config changes. Switch to `postgres` or `redis` when you run multiple server replicas or when you want rate-limit counters to survive server restarts.

> **Redis feature flag:** The `redis` backend is only compiled when the `cache-redis` feature is enabled. The official Docker image includes it. When building from source, pass `--features cache-redis` to `cargo build`.

#### How each backend is used

**Metadata cache:** Version lists and release metadata returned by upstream registries are stored with a TTL (`metadata_ttl_secs`). The cache backend is consulted on every proxy request before hitting the upstream.

**Rate-limit counters:** Each `increment` call atomically bumps a counter keyed by `rl:{registry}:user:{user_id}` (or `rl:{registry}:group:{group}`) and returns the new count plus the window-reset timestamp:
- `memory` — Mutex-protected HashMap; each process has its own counters.
- `postgres` — `INSERT … ON CONFLICT DO UPDATE … RETURNING count`; fully serialisable.
- `redis` — atomic `INCR` with a conditional `EXPIRE` on first write; TTL-based cleanup.

---

### 3.3 `[[auth]]`

An array of auth providers tried in declaration order. Three types are supported.

#### 3.3.1 Token auth (`type = "token"`)

Validates static bearer tokens defined in the config file. Useful for CI/CD pipelines and simple setups.

```toml
[[auth]]
type = "token"

[[auth.tokens]]
value = "my-ci-token"     # the bearer token value (plaintext or Argon2id PHC hash)
role = "user"             # "admin", "user", or "anonymous"
user_id = "ci-bot"        # optional: display name in logs

[[auth.tokens]]
value = "my-admin-token"
role = "admin"
user_id = "admin"
```

| Field | Type | Required | Notes |
|---|---|---|---|
| `value` | string | yes | The Bearer token string — plaintext **or** an Argon2id PHC hash (see below) |
| `role` | string | yes | `"admin"`, `"user"`, or `"anonymous"` |
| `user_id` | string | no | Used in audit logs |

##### Argon2id hashed token values (recommended for production)

Instead of storing a raw token in the config file, store an **Argon2id PHC hash**. BatleHub ships a helper command that generates the hash from the raw token:

```sh
batlehub hash-token my-secret-token
# → $argon2id$v=19$m=65536,t=3,p=4$...
```

Copy the printed hash into the `value` field:

```toml
[[auth.tokens]]
value = "$argon2id$v=19$m=65536,t=3,p=4$..."
role  = "admin"
user_id = "admin"
```

BatleHub automatically detects PHC-format values (those starting with `$argon2`) and verifies incoming bearer tokens against the stored hash. Plaintext values continue to work without any change — the two formats can coexist in the same config file.

> **Why this matters:** If the config file leaks (e.g. committed to VCS by mistake, visible in a Kubernetes ConfigMap), hashed tokens cannot be used directly by an attacker. The raw token only ever needs to exist in your secrets manager or the developer's clipboard.

#### 3.3.2 OIDC auth (`type = "oidc"`)

Validates JWT Bearer tokens issued by any standards-compliant OIDC provider (Authentik, Keycloak, Dex, etc.). Optionally enables browser-based SSO login.

```toml
[[auth]]
type = "oidc"
# name = "oidc"           # default; must be unique when running multiple OIDC providers
issuer_url = "https://sso.example.com/application/o/batlehub/"
client_id = "batlehub"
# client_secret = "..."   # required for confidential clients
# redirect_uri = "https://batlehub.example.com/api/v1/auth/oidc/callback"
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
# audiences    = ["batlehub"]

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
| `token_path` | string | `/var/run/secrets/kubernetes.io/serviceaccount/token` | batlehub's own service account token for TokenReview calls; re-read each request to handle automatic rotation |
| `audiences` | string[] | `["batlehub"]` | Audiences sent in the TokenReview request |
| `role_mappings` | map | `{}` | Maps Kubernetes usernames or group names to proxy roles |

**Role mapping keys:** Kubernetes sets `username: "system:serviceaccount:<namespace>:<name>"` and `groups: ["system:serviceaccounts", "system:serviceaccounts:<namespace>", ...]`. When a token matches multiple keys, the highest role wins.

#### 3.3.4 Actions OIDC auth (`type = "actions-oidc"`)

Validates short-lived OIDC JWTs issued by GitHub Actions or Forgejo Actions to workflow jobs (requires `id-token: write` in the workflow permissions). Rather than mapping a single claim value to a role, it evaluates a list of **rules** — each rule matches on any combination of JWT claims and grants a group name and a role when it matches.

```toml
[[auth]]
type = "actions-oidc"
name = "forgejo-action"                    # default: "actions-oidc"
issuer_url = "https://forgejo.example.com" # GitHub: "https://token.actions.githubusercontent.com"
# user_id_claim = "sub"                    # default

  # Static group: deployers on the main branch
  [[auth.rules]]
  group = "ci-deployers"
  role  = "admin"
  match = "all"              # all conditions must pass (default)
  [[auth.rules.conditions]]
  claim   = "repository_owner"
  pattern = "batleforc"
  [[auth.rules.conditions]]
  claim   = "ref"
  pattern = "refs/heads/main"

  # Dynamic group: every token gets an automatic per-repo/per-branch group
  # e.g. "forgejo-action/batleforc-batlehub/main"
  [[auth.rules]]
  group_template = "{name}/{repository}/{ref_name}"
  role           = "user"
  match          = "all"
  [[auth.rules.conditions]]
  claim   = "repository_owner"
  pattern = "batleforc"       # glob: exact match

  # Regex example: tag-based releases
  [[auth.rules]]
  group = "tag-releasers"
  role  = "user"
  match = "all"
  [[auth.rules.conditions]]
  claim      = "ref"
  pattern    = "^refs/tags/v[0-9]+"
  match_type = "regex"        # explicit; auto-detected from "^" anyway
```

**Provider fields:**

| Field | Type | Default | Notes |
|---|---|---|---|
| `name` | string | `"actions-oidc"` | Provider name. Appears in log output and in `Identity.auth_provider`. Must be unique across all `[[auth]]` entries. |
| `issuer_url` | string | — | OIDC issuer base URL. GitHub: `"https://token.actions.githubusercontent.com"`. Forgejo: your instance URL. |
| `user_id_claim` | string | `"sub"` | JWT claim used as `user_id` in the resolved identity. |
| `rules` | array | `[]` | Ordered list of group rules evaluated against each JWT. All matching rules contribute — they are not exclusive. |

**Rule fields (`[[auth.rules]]`):**

| Field | Type | Default | Notes |
|---|---|---|---|
| `group` | string | — | Static group name granted when the rule matches. At least one of `group` or `group_template` is required. |
| `group_template` | string | — | Template for a dynamically-named group. See template variables below. |
| `role` | string | `"user"` | Role granted by this rule (`"admin"`, `"user"`, `"anonymous"`). The final role is the highest across all matching rules. |
| `match` | `"all"` \| `"any"` | `"all"` | Whether all conditions must pass (AND) or at least one (OR). |
| `conditions` | array | `[]` | Conditions evaluated against JWT claims. An empty list always matches. |

**Condition fields (`[[auth.rules.conditions]]`):**

| Field | Type | Default | Notes |
|---|---|---|---|
| `claim` | string | — | JWT claim key to test (e.g. `"repository"`, `"ref"`, `"environment"`, `"actor"`). |
| `pattern` | string | — | Pattern to match the claim value against. |
| `match_type` | `"auto"` \| `"glob"` \| `"regex"` | `"auto"` | Pattern type. `auto` treats the pattern as regex when it starts with `^`, ends with `$`, or contains `[`, `(`, `+`. Otherwise it is treated as a glob. |

**Pattern types:**

- **Glob** — shell-style wildcards: `myorg/*` matches `myorg/foo` but not `other/foo`. `*` matches any sequence of characters.
- **Regex** — full `regex` crate syntax: `^refs/tags/v[0-9]+` matches any tag starting with `v` followed by digits. Compilation errors abort provider startup.

**Group template variables:**

Templates are `{placeholder}` strings rendered per-request. Substituted values have `/` replaced with `-` (so group names stay path-safe); literal `/` in the template itself is preserved.

| Variable | Value |
|----------|-------|
| `{name}` | Provider's `name` field |
| `{ref_name}` | `ref` claim with `refs/heads/` or `refs/tags/` prefix stripped |
| `{<any claim key>}` | Value of that JWT claim, with `/` → `-` |

Example: with `name = "forgejo-action"`, `repository = "batleforc/batlehub"`, `ref = "refs/heads/main"`:

```
"{name}/{repository}/{ref_name}"  →  "forgejo-action/batleforc-batlehub/main"
```

**GitHub Actions OIDC token claims (representative subset):**

| Claim | Example value | Description |
|-------|---------------|-------------|
| `sub` | `repo:org/repo:ref:refs/heads/main` | Subject (unique token identifier) |
| `repository` | `org/my-repo` | Repository in `owner/name` form |
| `repository_owner` | `org` | Repository owner (user or org) |
| `ref` | `refs/heads/main` | Full Git ref |
| `ref_type` | `branch` or `tag` | Type of ref |
| `workflow` | `CI` | Workflow name |
| `environment` | `production` | Deployment environment (if set) |
| `actor` | `alice` | GitHub username who triggered the run |
| `event_name` | `push` | Triggering event |
| `sha` | `abc123…` | Commit SHA |

Forgejo issues tokens with the same claim structure; only the issuer URL differs.

**Granting access via RBAC:**

Dynamic groups enable wildcard grants. To allow all CI tokens from `batleforc`'s repos to read releases:

```toml
[registries.rbac.groups]
"forgejo-action/*" = ["releases:read"]

# Grant specific per-repo CI full publish access
"forgejo-action/batleforc-batlehub/*" = ["releases:read", "releases:write"]
```

**GitHub Actions workflow snippet:**

```yaml
jobs:
  publish:
    permissions:
      id-token: write   # required to request an OIDC token
      contents: read
    steps:
      - name: Push artifact
        env:
          BATLEHUB_TOKEN: ${{ secrets.BATLEHUB_TOKEN }}
        run: |
          # BatleHub validates the OIDC token; no long-lived secret needed
          # when using actions-oidc — pass the ACTIONS_ID_TOKEN_REQUEST_URL
          # and ACTIONS_ID_TOKEN_REQUEST_TOKEN env vars to your publish tool
          cargo publish --registry batlehub
```

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
prefix = "batlehub/"         # optional, default: none
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
# artifact_ttl_secs = 2592000  # optional: re-fetch artifacts older than 30 days

[registries.rbac]
anonymous = []
user = ["releases:read", "source:read"]
admin = ["*"]

[registries.rbac.groups]
"team-a" = ["releases:read", "source:read"]
"*:ops"  = ["*"]   # wildcard: any provider's "ops" group

[[registries.rules]]
kind = "release_age_gate"
min_age_secs = 3600              # default: 3600 (1 hour)
bypass_roles = ["admin"]
deny_missing_timestamp = false   # set true to block packages with no timestamp

# [[registries.rules]]
# kind = "require_signed_release"
# enabled = true
```

**Top-level fields:**

| Field | Type | Required | Notes |
|---|---|---|---|
| `type` | string | yes | `"github"`, `"npm"`, `"cargo"`, `"openvsx"`, `"vscode-marketplace"`, `"goproxy"`, `"maven"`, `"terraform"`, `"rubygems"`, `"composer"`, `"pypi"`, `"conda"` |
| `name` | string | yes | Unique identifier; used in proxy URL paths |
| `mode` | string | no | `"proxy"` (default), `"local"`, or `"hybrid"`. Supported for `cargo`, `npm`, `openvsx`, `vscode-marketplace`, `goproxy`, `maven`, `terraform`, `rubygems`, `composer`, `pypi`, and `conda`. See [registry modes](#registry-modes). |
| `upstreams` | string[] | no | Upstream URLs tried in order on cache miss; 404 from one falls through to the next. Defaults to the registry's built-in URL. Required for `hybrid` mode. |
| `index_url` | string | no | Cargo only: sparse crate index URL. Defaults to `https://index.crates.io`. Required for `hybrid` mode and self-hosted Gitea/Forgejo registries. |
| `storage` | string | no | Name of the storage backend. Must match a `[[storage.backends]]` name. Omit to use the default backend. |
| `upstream_auth` | table | no | Credentials sent on every upstream request. See [upstream auth](#upstream_auth). |
| `tls` | table | no | TLS settings for upstream connections. See [upstream TLS](#upstream_tls). |
| `proxy` | table | no | HTTP/SOCKS proxy for upstream connections. See [upstream proxy](#upstream_proxy). |

#### Registry modes {#registry-modes}

`cargo`, `npm`, `openvsx`, `vscode-marketplace`, `goproxy`, `maven`, `terraform`, `rubygems`, `composer`, `pypi`, and `conda` registries support three operating modes, set via the `mode` field:

| Mode | Description |
|------|-------------|
| `proxy` | Default. BatleHub only forwards requests to upstream registries. Publishing is rejected. |
| `local` | BatleHub is the authoritative registry. No upstream needed. Clients publish directly to BatleHub. |
| `hybrid` | Local-first. Serves locally published packages directly; falls back to the configured upstream for anything not published locally. Requires `upstreams` (and `index_url` for Cargo). |

Publishing requires at least the `user` role. The `published_by` field is set from the authenticated user's `user_id`.

**Cargo** — `local`/`hybrid` modes expose the full publish API (`PUT /api/v1/crates/new`, yank, unyank, owners) and advertise the `api` URL in `config.json` so Cargo discovers it automatically.

**npm** — `local`/`hybrid` modes accept `npm publish` payloads (`PUT /proxy/{registry}/{name}`) and serve packuments and tarballs from local storage.

**openvsx / vscode-marketplace** — `local`/`hybrid` modes accept raw VSIX uploads (`PUT /proxy/{registry}/{extension_id}/{version}/vsix`) and serve them on download.

**goproxy** — `local`/`hybrid` modes accept Go module zip uploads (`PUT /proxy/{registry}/{module}/@v/{version}.zip`). `go.mod` is extracted automatically from the zip; `.info` is generated from the version and upload timestamp. Serves `@latest`, `@v/list`, `.info`, `.mod`, and `.zip` from local storage.

**maven** — `local`/`hybrid` modes accept `mvn deploy` artifact uploads (`PUT /proxy/{registry}/maven2/{path}`). Non-POM files (JARs, checksums) are stored immediately; the three-phase publish is triggered when the `.pom` file arrives. `maven-metadata.xml` is generated dynamically from the database and never cached client-side. See [Worked Example 6.12](#612-private-maven-registry-local--hybrid-mode).

**terraform** — `local`/`hybrid` modes accept module uploads (`POST /proxy/{registry}/v1/modules/{ns}/{name}/{provider}/{version}`), provider version manifests (`POST .../v1/providers/{ns}/{type}/versions`), and provider binary uploads (`PUT .../artifact/{os}/{arch}`). The `tf_module_download` endpoint returns a `204 + X-Terraform-Get` header pointing at the locally stored tarball. See [Worked Example 6.13](#613-private-terraform-registry-local--hybrid-mode).

**rubygems** — `local`/`hybrid` modes accept `gem push` uploads (`POST /proxy/{registry}/api/v1/gems`). Serves gem files, version index, and REST info from local storage.

**composer** — `local`/`hybrid` modes accept ZIP uploads (`POST /proxy/{registry}/api/upload`). `composer.json` (with `name` and `version` fields) is extracted automatically. Serves `packages.json`, `p2/` metadata, and `dist/` artifacts from local storage.

**pypi** — `local`/`hybrid` modes accept twine-compatible multipart uploads (`POST /proxy/{registry}/legacy/`). The name and version are parsed from the uploaded filename and multipart fields. In `local` mode the Simple API index (`GET /proxy/{registry}/simple/{package}/`) is generated from the database. In `hybrid` mode upstream and local entries are served together.

**conda** — `local`/`hybrid` modes accept raw conda package uploads (`POST /proxy/{registry}/{platform}/`). Metadata (`name`, `version`, `build`, `depends`) is extracted from `info/index.json` inside the `.tar.bz2` or `.conda` archive. In `local` mode `repodata.json` is generated from the database. In `hybrid` mode local entries are merged into the upstream `repodata.json`.

#### Registry-type notes

**`github`** — proxies the GitHub REST API (releases, assets, source tarballs, raw files). Requires `upstreams` to point at `https://api.github.com` (the default).

**`npm`** — proxies the full npm registry protocol: packuments, version metadata, and `.tgz` tarballs. Works with npm, yarn, pnpm, and any tool that speaks the npm registry protocol. Set `mode = "local"` or `mode = "hybrid"` to enable publishing. See [registry modes](#registry-modes) and [Worked Example 6.7](#67-private-npm-registry-local--hybrid-mode).

**`cargo`** — proxies the Cargo sparse index and `.crate` downloads. Set `index_url` for self-hosted Gitea/Forgejo registries. Set `mode = "local"` or `mode = "hybrid"` to enable publishing. See [registry modes](#registry-modes) and [Worked Example 6.6](#66-private-cargo-registry-local--hybrid-mode).

**`openvsx`** — proxies VS Code extension VSIX downloads from [open-vsx.org](https://open-vsx.org) or a compatible host. Extension IDs use the `{publisher}.{name}` convention. Set `mode = "local"` or `mode = "hybrid"` to enable publishing. See [Worked Example 6.8](#68-private-vs-code-extension-registry-local--hybrid-mode).

**`vscode-marketplace`** — proxies VS Code extension VSIX downloads from [marketplace.visualstudio.com](https://marketplace.visualstudio.com) using Microsoft's Gallery API. Extension IDs use the same `{publisher}.{name}` convention as OpenVSX. Metadata is resolved via a `POST /_apis/public/gallery/extensionquery` call; artifacts are fetched directly from `/_apis/public/gallery/publishers/{publisher}/vsextensions/{name}/{version}/vspackage`. Use this type when you need to cache extensions that are only available on the Microsoft marketplace and not mirrored on open-vsx.org. Supports `mode = "local"` and `mode = "hybrid"` for hosting private extensions — see [Worked Example 6.8](#68-private-vs-code-extension-registry-local--hybrid-mode).

```toml
[[registries]]
type = "vscode-marketplace"
name = "vscode"
# upstreams = ["https://marketplace.visualstudio.com"]  # default

[registries.rbac]
user = ["releases:read", "source:read"]
admin = ["*"]
```

Download a VSIX via the proxy:

```sh
# Latest version
curl -H "Authorization: Bearer <token>" \
  http://localhost:8080/proxy/vscode/ms-python.python/latest/vsix \
  -o ms-python.python.vsix

# Pinned version
curl -H "Authorization: Bearer <token>" \
  http://localhost:8080/proxy/vscode/ms-python.python/2024.2.1/vsix \
  -o ms-python.python-2024.2.1.vsix
```

**`goproxy`** — implements the [GOPROXY protocol](https://go.dev/ref/mod#goproxy-protocol) for Go module proxying. Set `mode = "local"` or `mode = "hybrid"` to host private modules — see [registry modes](#registry-modes) and [Worked Example 6.9](#69-private-go-module-proxy-local--hybrid-mode). Supports all five endpoints:

| Endpoint | Description |
|----------|-------------|
| `/{module}/@latest` | Latest version metadata JSON |
| `/{module}/@v/list` | Newline-separated list of known versions |
| `/{module}/@v/{version}.info` | Version metadata JSON |
| `/{module}/@v/{version}.mod` | Raw `go.mod` file |
| `/{module}/@v/{version}.zip` | Module source zip archive |

Module paths may contain slashes (e.g. `golang.org/x/text`). Uppercase-encoded paths (`!{lowercase}` convention) are passed through to the upstream unchanged.

> **Caching note:** `@latest` and `@v/list` responses are cached permanently after the first request, just like other artifacts. They may become stale if new versions are published. Clear the proxy storage (or configure a shorter `metadata_ttl_secs`) to pick up new versions immediately.

Configure the go toolchain to use the proxy:

```sh
export GONOSUMCHECK="*"
export GONOSUMDB="*"
export GOPROXY="http://batlehub.example.com/proxy/go,direct"
```

---

**`maven`** — proxies Maven artifact repositories. Supports `GET` requests for POM files, JARs, source JARs, Javadoc JARs, SHA-1/MD5 checksums, and Maven metadata XML. Compatible with Maven, Gradle, and any tool that speaks the Maven repository protocol. Default upstream: `https://repo1.maven.org/maven2`. Set `mode = "local"` or `mode = "hybrid"` to enable private publishing — see [registry modes](#registry-modes) and [Worked Example 6.12](#612-private-maven-registry-local--hybrid-mode).

Configure Maven to use the proxy:

```xml
<!-- ~/.m2/settings.xml -->
<settings>
  <mirrors>
    <mirror>
      <id>batlehub</id>
      <mirrorOf>central</mirrorOf>
      <url>http://batlehub.example.com/proxy/maven/maven2/</url>
    </mirror>
  </mirrors>
</settings>
```

Configure Gradle to use the proxy:

```kotlin
// settings.gradle.kts
dependencyResolutionManagement {
    repositories {
        maven { url = uri("http://batlehub.example.com/proxy/maven/maven2/") }
    }
}
```

---

**`terraform`** — proxies the Terraform provider and module registry protocol. Supports provider version listing, provider download info (binary URL + checksums), module version listing, and module source download. Default upstream: `https://registry.terraform.io`. Set `mode = "local"` or `mode = "hybrid"` to enable private module and provider publishing — see [registry modes](#registry-modes) and [Worked Example 6.13](#613-private-terraform-registry-local--hybrid-mode).

| Endpoint | Method | Description |
|---|---|---|
| `/v1/providers/{namespace}/{type}/versions` | GET | Provider version list (JSON, cached) |
| `/v1/providers/{namespace}/{type}/{version}/download/{os}/{arch}` | GET | Provider download info JSON (cached; local: rewritten to `/artifact` URL) |
| `/v1/providers/{namespace}/{type}/versions` | POST | **Local/Hybrid:** publish provider version manifest |
| `/v1/providers/{namespace}/{type}/{version}/artifact/{os}/{arch}` | PUT | **Local/Hybrid:** upload provider binary zip |
| `/v1/providers/{namespace}/{type}/{version}/artifact/{os}/{arch}` | GET | **Local/Hybrid:** serve provider binary zip |
| `/v1/modules/{namespace}/{name}/{provider}/versions` | GET | Module version list (JSON, cached) |
| `/v1/modules/{namespace}/{name}/{provider}/{version}/download` | GET | Module source redirect (`204 + X-Terraform-Get`; local: points at `/artifact`) |
| `/v1/modules/{namespace}/{name}/{provider}/{version}` | POST | **Local/Hybrid:** upload module tar.gz |
| `/v1/modules/{namespace}/{name}/{provider}/{version}/artifact` | GET | **Local/Hybrid:** serve module tar.gz |

> **Module download in proxy mode:** passes through the upstream `204 + X-Terraform-Get` header without caching. In Local/Hybrid mode the header is rewritten to point at the local `/artifact` endpoint.

Configure the Terraform CLI to use the proxy for providers:

```hcl
# ~/.terraformrc  (or %APPDATA%/terraform.rc on Windows)
provider_installation {
  network_mirror {
    url = "http://batlehub.example.com/proxy/terraform/"
  }
}
```

---

**`composer`** — implements the [Packagist v2 protocol](https://packagist.org/apidoc) for PHP Composer. Serves `packages.json` (repository root index), `p2/{vendor}/{package}.json` (metadata), and `dist/{vendor}/{package}/{version}` (ZIP artifact downloads). Default upstream: `https://repo.packagist.org`. Set `mode = "local"` or `mode = "hybrid"` to enable private package publishing — see [registry modes](#registry-modes) and [Worked Example 6.15](#615-private-composer-registry-local--hybrid-mode).

| Endpoint | Method | Description |
|---|---|---|
| `/proxy/{registry}/packages.json` | GET | Repository root index (lists all known package names) |
| `/proxy/{registry}/p2/{vendor}/{package}.json` | GET | Package metadata (all versions, dist URLs) |
| `/proxy/{registry}/p2/{vendor}/{package}~dev.json` | GET | Dev-stability metadata variant |
| `/proxy/{registry}/dist/{vendor}/{package}/{version}` | GET | Download ZIP artifact |
| `/proxy/{registry}/api/upload` | POST | **Local/Hybrid:** publish a package (multipart or raw ZIP body) |
| `/proxy/{registry}/api/packages/{vendor}/{package}/versions/{version}` | DELETE | **Local/Hybrid:** yank a version |

---

**`pypi`** — implements the [Python Simple Repository API (PEP 503 / PEP 691)](https://peps.python.org/pep-0503/) and [PyPI JSON API](https://docs.pypi.org/api/json/). Download URLs in Simple index pages are rewritten to route through the proxy cache. Default upstream: `https://pypi.org`. Set `mode = "local"` or `mode = "hybrid"` to enable private publishing via `twine upload` — see [registry modes](#registry-modes).

| Endpoint | Method | Description |
|---|---|---|
| `/proxy/{registry}/simple/` | GET | Root index (all project names) |
| `/proxy/{registry}/simple/{package}/` | GET | Per-package file listing (HTML or JSON via `Accept` header) |
| `/proxy/{registry}/packages/{filename}` | GET | Download wheel or sdist (cached) |
| `/proxy/{registry}/legacy/` | POST | **Local/Hybrid:** twine-compatible multipart publish |

Configure pip:

```ini
# ~/.pip/pip.conf
[global]
index-url = http://batlehub.example.com/proxy/my-pypi/simple/
```

---

**`conda`** — proxies a single conda channel (e.g. `conda-forge`) across all platforms. Caches `repodata.json` and package files per platform. In hybrid mode, locally published packages are merged into the upstream `repodata.json`. Default upstream: `https://conda.anaconda.org`. Set `mode = "local"` or `mode = "hybrid"` to enable private publishing — see [registry modes](#registry-modes).

| Endpoint | Method | Description |
|---|---|---|
| `/proxy/{registry}/{platform}/repodata.json` | GET | Channel index for a platform (e.g. `linux-64`, `noarch`) |
| `/proxy/{registry}/{platform}/current_repodata.json` | GET | Reduced index (proxy mode only) |
| `/proxy/{registry}/{platform}/{filename}` | GET | Download `.conda` or `.tar.bz2` package |
| `/proxy/{registry}/{platform}/` | POST | **Local/Hybrid:** publish a conda package |

Configure conda:

```yaml
# ~/.condarc
channels:
  - http://batlehub.example.com/proxy/my-conda
  - nodefaults
```

Configure Composer to use the proxy by adding a repository entry in `composer.json`:

```json
{
  "repositories": [
    {
      "type": "composer",
      "url": "http://batlehub.example.com/proxy/packagist/",
      "options": {
        "http": {
          "header": ["Authorization: Bearer <your-token>"]
        }
      }
    }
  ]
}
```

Or store credentials in `auth.json` (never commit this file):

```json
{
  "http-basic": {
    "batlehub.example.com": {
      "username": "user",
      "password": "<your-token>"
    }
  }
}
```

---

**`[registries.cache]` fields:**

| Field | Type | Default | Notes |
|---|---|---|---|
| `metadata_ttl_secs` | u64 | `300` | How long release metadata (version lists, release info) is cached in seconds |
| `serve_stale` | bool | `true` | When `true`, serve stale metadata if the upstream returns a transient error (5xx). Keeps the registry usable during upstream outages. |
| `artifact_ttl_secs` | u64? | — | Evict artifacts older than this many seconds. Omit to never expire by age. |
| `idle_days` | u64? | — | Evict artifacts not accessed for this many days. Omit to disable idle eviction. |
| `max_size_bytes` | u64? | — | Storage cap in bytes. When exceeded, the least-recently-used artifacts are removed until usage falls below the cap. Omit for no size limit. |
| `keep_latest_n` | usize? | — | Keep only the N most-recently-cached versions per package. Older versions are evicted when a new one is stored. Omit to keep all versions. |
| `warm_packages` | string[] | `[]` | Packages to pre-fetch at startup and via the admin warm endpoint. Each entry is a bare name (`"lodash"`) or a pinned version (`"lodash@4.17.21"`). |
| `warm_latest_n` | usize | `1` | Number of most-recent versions to warm per bare package name. Pinned-version entries always warm exactly one version. |
| `warm_concurrency` | usize | `2` | Maximum concurrent artifact downloads during a warming run. |

**Eviction example:**

```toml
[registries.cache]
metadata_ttl_secs = 600
artifact_ttl_secs = 2592000   # 30 days
idle_days         = 14
max_size_bytes    = 10737418240  # 10 GiB
keep_latest_n     = 5
```

**Cache warming example:**

```toml
[registries.cache]
warm_packages    = ["lodash", "react", "typescript@5.4.5"]
warm_latest_n    = 3      # warm the 3 most recent versions of bare-name packages
warm_concurrency = 4      # up to 4 parallel downloads
```

At startup, BatleHub pre-fetches the listed packages so they are available with zero latency on first request. The same packages can be re-warmed at any time via the admin API:

```sh
# Warm all configured versions of lodash
curl -X POST http://localhost:8080/api/v1/admin/registries/npm/warm \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"package": "lodash"}'

# Override the version count for this call only
curl -X POST http://localhost:8080/api/v1/admin/registries/npm/warm \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"package": "lodash", "versions": 10}'
```

> **Registry support:** version enumeration (used by bare-name warming) is implemented for **npm**, **Cargo**, **OpenVSX**, and **Go** modules. For GitHub and VS Code Marketplace, pass a pinned version string (e.g. `"owner/repo@v1.2.3"`) to warm a specific version.

**Content-addressable deduplication:**

BatleHub stores physical artifact bytes at a content-addressed key (`blob/{sha256}`) and maps logical artifact keys to that blob via a reference count. When the same bytes are referenced by multiple logical keys (e.g. the same package mirrored across two registries, or a yanked-then-re-released version), only one copy of the data is stored on disk or in S3. The deduplication tables (`artifact_dedup_index`, `artifact_dedup_refs`) are created automatically by the database migration and require no configuration.

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
| `min_age_secs` | u64 | `3600` | Releases younger than this are blocked. |
| `bypass_roles` | string[] | `[]` | Roles that skip the gate entirely, including the missing-timestamp check (e.g. `["admin"]`). |
| `deny_missing_timestamp` | bool | `false` | When `true`, deny downloads for packages whose upstream provides no publish timestamp, instead of skipping the check and allowing the download. Useful for registries like conda where the timestamp field is optional — setting this to `true` ensures every package carries a verifiable age. |

> **Timestamp support by registry type:** The gate is only enforced when the upstream provides a publish timestamp.
> - **npm**, **Cargo**, **OpenVSX**, **VS Code Marketplace**, **Go**, **PyPI** — timestamp always populated; gate is fully enforced.
> - **GitHub** — timestamp populated only for specific-tag release requests (asset downloads). Raw files, source tarballs, and release listings return no timestamp; the gate is skipped for those requests.
> - **Conda** — timestamp is the `timestamp` field (milliseconds since epoch) in `repodata.json`. Most packages carry it, but older or third-party packages may omit it. Use `deny_missing_timestamp = true` to reject packages without a verifiable build date.
> - **Terraform providers** — timestamp populated by `registry.terraform.io` but not mandated by the official spec; other Terraform registries may omit it.

**`[[registries.rules]]` — Require signed release:**

| Field | Type | Default | Notes |
|---|---|---|---|
| `kind` | string | — | Must be `"require_signed_release"` |
| `enabled` | bool | `false` | When true, blocks releases that do not have a verified signature |

**`[[registries.rules]]` — Deny latest:**

Rejects any request that uses `"latest"` as the version tag, forcing consumers to pin explicit versions (supply-chain hygiene).

```toml
[[registries.rules]]
kind = "deny_latest"
bypass_roles = ["admin"]   # omit or leave empty for a hard block
```

| Field | Type | Default | Notes |
|---|---|---|---|
| `kind` | string | — | Must be `"deny_latest"` |
| `bypass_roles` | string[] | `[]` | Roles that may still request `"latest"` (e.g. `["admin"]`). When multiple roles are listed the least-privileged one sets the access floor. When empty, the block applies to all roles. |

> This rule applies to all registry types. `"latest"` is the literal version string sent by the client — for npm it maps to the `latest` dist-tag, for Cargo and Go it triggers upstream `@latest` resolution, and for OpenVSX and VS Code Marketplace it fetches the current published version.

**`[[registries.rules]]` — Version gate:**

Gates downloads by version using an optional approved-version allowlist plus a blocklist of specific versions with known issues. The resolved version is matched against both lists: a `block` match is always rejected, and when `allow` is non-empty a version matching **none** of its entries is also rejected. `block` takes precedence over `allow`.

```toml
[[registries.rules]]
kind = "version_gate"
allow = [">=1.2.0, <2.0.0"]   # optional: when set, only matching versions are served
block = ["1.4.7", "1.5.0"]    # specific versions with known issues
bypass_roles = ["admin"]
```

| Field | Type | Default | Notes |
|---|---|---|---|
| `kind` | string | — | Must be `"version_gate"` |
| `allow` | string[] | `[]` | Approved-version allowlist. When non-empty, a version matching none of these entries is rejected. When empty, all versions are allowed (subject to `block`). |
| `block` | string[] | `[]` | Blocklist of specific versions (or ranges) with known issues. A match is always rejected. |
| `bypass_roles` | string[] | `[]` | Roles that may bypass the gate (e.g. `["admin"]`). When multiple are listed the least-privileged one sets the access floor. When empty, the gate applies to all roles. |

> **Matching:** each entry is treated as a semver range when it contains a range operator (`<`, `>`, `=`, `^`, `~`, `*`, `,`) and parses as a valid [`VersionReq`](https://docs.rs/semver/) (e.g. `">=1.2.0, <2.0.0"`); otherwise it is matched by **exact string equality**. This keeps a bare `"1.2.3"` exact (rather than the caret semantics `^1.2.3` semver would otherwise infer) and lets non-semver version strings (git hashes, dates) be listed verbatim.

**`[[registries.rules]]` — CVE gate:**

Denies downloads of versions with a recorded vulnerability finding at or above a severity threshold. Requires a configured vulnerability scanner — see [Vulnerability scanning](security-scanning.md) for the full setup.

```toml
[[registries.rules]]
kind         = "cve_gate"
min_severity = "high"        # one of: low, medium, high, critical (default: high)
bypass_roles = ["admin"]
```

| Field | Type | Default | Notes |
|---|---|---|---|
| `kind` | string | — | Must be `"cve_gate"` |
| `min_severity` | string | `"high"` | Minimum severity that triggers a block: `"low"`, `"medium"`, `"high"`, or `"critical"`. |
| `bypass_roles` | string[] | `[]` | Roles exempt from the gate. |

**`[[registries.rules]]` — Trusted publisher:**

Restricts downloads to packages published by an allowed org, user, or scope. The publisher is derived from metadata already resolved during the proxy fetch — no extra upstream calls.

```toml
[[registries.rules]]
kind = "trusted_publisher"
allow = ["my-org", "trusted-user"]
bypass_roles = ["admin"]
```

| Field | Type | Default | Notes |
|---|---|---|---|
| `kind` | string | — | Must be `"trusted_publisher"` |
| `allow` | string[] | `[]` | Allowed publisher identifiers. When non-empty, a package whose derived publisher matches none of these is rejected. When empty, the rule allows everything. |
| `bypass_roles` | string[] | `[]` | Roles that may bypass the gate (e.g. `["admin"]`). |

> **Publisher support by registry type:** matching is case-insensitive.
> - **GitHub**, **GitLab**, **Forgejo** — the top-level owner/group segment of the package path (`"owner/repo"` or `"group/subgroup/project"` → `"owner"` / `"group"`).
> - **npm** — the scope for scoped packages (`"@scope/name"` → `"scope"`); otherwise the user who published that version.
> - **OpenVSX**, **VS Code Marketplace** — the publisher segment of the extension id (`"publisher.extension"` → `"publisher"`).
> - **Not yet supported: Cargo** (crate ownership isn't in the sparse index and would need a separate crates.io API call) and any other registry type. Configuring this rule on an unsupported registry **denies every request** — this is a fail-closed supply-chain gate, not a fail-open one.

#### `[registries.integrity]` {#integrity}

Per-registry artifact integrity verification. On the proxy fetch-and-cache path, buffered upstream bytes are hashed and compared against the checksum advertised in the registry metadata (Cargo SHA-256, npm SRI/`shasum`, PyPI SHA-256). Registries that advertise no checksum (NuGet, Maven, GitHub, Go, …) fall through to the "missing" path. Does **not** apply to `firewall_only` registries, which stream straight through without buffering.

```toml
[registries.integrity]
enabled = true            # verify when a checksum is advertised
block_on_mismatch = true  # fail the download on a hash mismatch (never bypassable)
require_metadata = false  # block downloads with no advertised checksum
bypass_roles = ["admin"]  # roles exempt from the require_metadata gate
verify_on_serve = false   # re-hash stored bytes on every serve, not just on first fetch
```

| Field | Type | Default | Notes |
|---|---|---|---|
| `enabled` | bool | `true` | Master switch. When `false`, no verification is performed. |
| `block_on_mismatch` | bool | `true` | Fail the download (and skip caching) when the computed digest does not match the advertised one. A mismatch is never bypassable. |
| `require_metadata` | bool | `false` | Block downloads for which the upstream advertises no usable checksum, unless the caller holds one of `bypass_roles`. Defaults to warn-only. |
| `bypass_roles` | string[] | `[]` | Roles allowed to bypass the `require_metadata` gate. |
| `verify_on_serve` | bool | `false` | Re-verify cached/stored bytes against a **self-computed** SHA-256 (recorded when the bytes are first cached) on every serve — cache hits on the proxy path and local-registry reads — not just on first fetch. Catches storage corruption or tampering of already-cached artifacts. A mismatch fails the download (`502`) and evicts the bad entry so a later request re-fetches clean bytes. Off by default because it reads and hashes the bytes on each serve (the proxy path streams them through the hash so memory stays bounded, then re-opens the entry to serve it). Pre-existing cache rows have no stored checksum and are treated as "skip re-verify" until next refreshed. |

#### `[registries.signing]` {#signing}

Per-registry artifact signing. At publish time, a client supplies a detached signature via the `X-Artifact-Signature` (+ `X-Signature-Type`) headers, stored alongside the artifact. The `required`/`allowed_types` fields gate signature **presence and type** at publish; `verify_on_download`/`trusted_keys` re-check a stored `ed25519` signature on **download**.

```toml
[registries.signing]
required = false                 # reject publishes with no X-Artifact-Signature header
allowed_types = ["ed25519"]      # accepted signature types; empty = any (or none)
verify_on_download = false       # re-verify a stored ed25519 signature on every download
trusted_keys = ["<hex pubkey>"]  # hex-encoded 32-byte Ed25519 public keys trusted to sign
```

| Field | Type | Default | Notes |
|---|---|---|---|
| `required` | bool | `false` | Reject publish requests that do not include an `X-Artifact-Signature` header. |
| `allowed_types` | string[] | `[]` | Accepted signature types (e.g. `["pgp", "ed25519"]`). When empty, any type (or none) is accepted. |
| `verify_on_download` | bool | `false` | Verify a stored `ed25519` detached signature against `trusted_keys` on every download (local-registry reads). A stored signature that fails to verify — or was signed by an untrusted key — fails the download with `502`. Signatures of other types and artifacts with no stored signature are not verified here (presence is governed by `required` at publish time). |
| `trusted_keys` | string[] | `[]` | Hex-encoded 32-byte Ed25519 public keys trusted to sign artifacts in this registry. A download verifies against each in turn; any match passes. |

> **Why Ed25519 only?** RSA-based crypto (the `rsa` crate, and therefore PGP / x509 / the default Sigstore paths) is hard-banned from the dependency tree by `deny.toml` (RUSTSEC-2023-0071). Ed25519 detached-signature verification keeps the tree RSA-free; Sigstore / npm provenance verification is left as a future item for that reason.

#### `[registries.upstream_auth]` {#upstream_auth}

Credentials to send on every upstream request for this registry. Three schemes are supported; choose one.

**Bearer token** — adds `Authorization: Bearer <token>`. Accepted by Gitea, Forgejo, Nexus (npm token), JFrog Artifactory, and GitHub Enterprise.

```toml
[registries.upstream_auth]
type  = "bearer"
token = "npat-xxxx"
```

**Basic auth** — standard HTTP Basic authentication.

```toml
[registries.upstream_auth]
type     = "basic"
username = "deploy"
password = "s3cr3t"
```

**Custom header** — sends an arbitrary header on every request. Useful for registries that use `X-API-Key` or similar schemes.

```toml
[registries.upstream_auth]
type  = "header"
name  = "X-API-Key"
value = "my-api-key"
```

| Field | Type | Schemes | Notes |
|---|---|---|---|
| `type` | string | all | `"bearer"`, `"basic"`, or `"header"` |
| `token` | string | bearer | Bearer token value |
| `username` | string | basic | HTTP Basic username |
| `password` | string | basic | HTTP Basic password |
| `name` | string | header | HTTP header name (e.g. `"X-API-Key"`) |
| `value` | string | header | HTTP header value |

> **Security:** Never commit credentials to version control. Use `${VAR_NAME}` placeholders in the config file to pull secrets from environment variables at startup — see [§5 Environment Variable Overrides](#5-environment-variable-overrides) for details.

#### `[registries.tls]` {#upstream_tls}

TLS settings for upstream connections. Use this when the upstream registry serves a certificate signed by a private or self-hosted CA that is not in the system trust store.

```toml
[registries.tls]
ca_cert_path = "/etc/ssl/corp-ca.pem"
```

| Field | Type | Required | Notes |
|---|---|---|---|
| `ca_cert_path` | string | no | Path to a PEM-encoded CA certificate to add as a trusted root for this registry's upstream connections |

> The certificate is loaded once at startup. To rotate a CA certificate, restart the server.

---

#### `[registries.proxy]` {#upstream_proxy}

Route all outgoing upstream registry requests through an HTTP, HTTPS, or SOCKS5 proxy. Use this in corporate or air-gapped environments where direct Internet access is restricted.

```toml
[registries.proxy]
url = "http://proxy.corp.example.com:3128"

# Optional: proxy credentials (alternative to embedding in the URL)
# username = "proxyuser"
# password = "${PROXY_PASSWORD}"

# Optional: bypass the proxy for specific hosts/domains (comma-separated).
# Equivalent to the NO_PROXY environment variable.
# no_proxy = "localhost,10.0.0.0/8,internal.example.com"
```

| Field | Type | Required | Notes |
|---|---|---|---|
| `url` | string | yes | Proxy URL. Supports `http://`, `https://`, and `socks5://` schemes. Credentials can be embedded directly: `http://user:pass@proxy:3128`. |
| `username` | string | no | Proxy Basic-auth username. Overrides any credentials embedded in `url`. Use `${VAR}` to inject from an environment variable. |
| `password` | string | no | Proxy Basic-auth password. Overrides any credentials embedded in `url`. Use `${VAR}` to inject from an environment variable. |
| `no_proxy` | string | no | Comma-separated list of hosts, domains, or CIDR ranges to bypass the proxy for (e.g. `"localhost,10.0.0.0/8,corp.example.com"`). Equivalent to the standard `NO_PROXY` environment variable. |

> **Scope:** The proxy applies only to upstream registry requests for the registry it is configured on. When absent, the global `[proxy]` section (if set) is used as a fallback — so you can set a single global proxy and override it per-registry where needed.

> **Security:** Avoid committing proxy credentials to version control. Use `${VAR_NAME}` placeholders — see [§5 Environment Variable Overrides](#5-environment-variable-overrides).

> **`HTTP_PROXY` / `HTTPS_PROXY` environment variables:** When no `[registries.proxy]` (and no global `[proxy]`) is configured for a registry, the underlying HTTP client automatically reads the standard `HTTP_PROXY`, `HTTPS_PROXY`, and `NO_PROXY` env vars. As soon as any proxy is configured via the config file, env-var proxy reading is disabled for that registry's client — the config value fully replaces the env var.

#### Forwarding `HTTP_PROXY` into the config

If you want to keep using the standard `HTTP_PROXY` env var while still being able to set `no_proxy` or credentials in the config file, forward the variable through the `${VAR}` substitution mechanism:

```toml
# Shell: export HTTP_PROXY=http://proxy.corp.example.com:3128

[registries.proxy]
url      = "${HTTP_PROXY}"
no_proxy = "localhost,10.0.0.0/8"
```

The same pattern works for the global section:

```toml
[proxy]
url      = "${HTTP_PROXY}"
no_proxy = "${NO_PROXY}"   # forward the standard NO_PROXY list too
```

---

#### `[registries.rate_limit]` {#rate_limit}

Per-registry rate limiting using a **fixed-window counter** algorithm. Limits are tracked per authenticated user (by `user_id`) or per client IP for anonymous requests.

Counters are stored in the **cache backend** selected by `[cache]`:
- `type = "memory"` (default) — counters are per-process; they reset on restart and are **not** shared across multiple server replicas.
- `type = "postgres"` or `type = "redis"` — counters survive restarts and are shared across all replicas, making the limit consistent across a load-balanced cluster.

```toml
[registries.rate_limit]
requests_per_window = 100
window_secs         = 60
enforcement         = "block"   # "block" (default) or "warn"
```

| Field | Type | Default | Notes |
|---|---|---|---|
| `requests_per_window` | u32 | — | Maximum number of requests allowed within `window_secs` |
| `window_secs` | u32 | — | Length of the sliding window in seconds |
| `enforcement` | string | `"block"` | `"block"` returns HTTP 429; `"warn"` allows the request but adds `X-RateLimit-Warning` |

**Response headers:**

| Header | When added | Description |
|---|---|---|
| `X-RateLimit-Limit` | Every proxied response (when configured) | The effective limit that bound this request |
| `Retry-After` | 429 responses (block mode) | Seconds until the bucket refills |
| `X-RateLimit-Reset` | 429 responses (block mode) | Unix timestamp when the bucket refills |
| `X-RateLimit-Warning: rate-limit-exceeded` | Over-limit responses (warn mode) | Signals the limit was exceeded but the request was allowed |

##### Per-group rate limits {#per_group_rate_limits}

All members of a named group share a single request pool. Group names are matched against the strings in the authenticated identity's `groups` list, which are namespaced by auth provider: `"oidc:<group>"`, `"kubernetes:<group>"`, etc.

```toml
[registries.rate_limit]
requests_per_window = 100
window_secs         = 60
enforcement         = "block"

# CI bots share a single 5000 req/min pool across all members:
[[registries.rate_limit.groups]]
name                = "oidc:ci-bots"
requests_per_window = 5000
window_secs         = 60
# enforcement = "block"   # optional; inherits parent enforcement when omitted

# Free-tier users share a more restrictive 200 req/min pool:
[[registries.rate_limit.groups]]
name                = "oidc:free-tier"
requests_per_window = 200
window_secs         = 60
```

`[[registries.rate_limit.groups]]` fields:

| Field | Type | Required | Notes |
|---|---|---|---|
| `name` | string | yes | Exact match against an entry in `Identity.groups` (e.g. `"oidc:ci-bots"`) |
| `requests_per_window` | u32 | yes | Shared pool size for **all** members of this group combined |
| `window_secs` | u32 | yes | Window length in seconds |
| `enforcement` | string | no | Overrides the parent `enforcement` for this group only; defaults to the parent value when omitted |

**Multi-limiter semantics:** both the per-user bucket and every applicable group bucket must have tokens for a request to proceed. If any bucket is exhausted:
- In `block` mode: the request is rejected with HTTP 429. The `Retry-After` and `X-RateLimit-Reset` headers reflect the longest wait among all exhausted buckets.
- In `warn` mode: the request is allowed and `X-RateLimit-Warning` is added to the response.
- If different buckets have different enforcement modes, `block` takes precedence over `warn`.

> **Multi-instance deployments:** Set `[cache] type = "postgres"` or `type = "redis"` to share rate-limit counters across all server replicas. With the default `type = "memory"`, each replica maintains its own independent counters and the effective per-user limit is `requests_per_window × replica_count`.

> **Fail-open behaviour:** If the cache backend is unreachable when a counter needs to be incremented, the request is **allowed** rather than rejected. A `WARN` log entry (`rate-limit store unavailable … failing open`) is emitted for each affected bucket. Monitor for these warnings to detect backend outages.

---

#### `[registries.beta_channel]`

Restricts pre-release versions (semver versions with a non-empty pre-release component, e.g. `1.0.0-beta.1`) so that only members of the registry's beta channel can see and download them. Non-members receive stable versions only and get HTTP 404 on direct pre-release artifact requests.

Applies to registries in `local` or `hybrid` mode. Members are managed via the back-office API.

```toml
[[registries]]
type = "npm"
name = "my-npm"
mode = "local"

[registries.beta_channel]
enabled = true
```

| Field | Type | Default | Notes |
|---|---|---|---|
| `enabled` | bool | `false` | Enable beta-channel access gating for this registry |

**Member management API (admin only):**
- `GET    /api/v1/admin/registries/{registry}/beta-channel` — list members
- `POST   /api/v1/admin/registries/{registry}/beta-channel` — body: `{ "principal_type": "user"|"group", "principal_id": "...", "granted_by": "..." }`
- `DELETE /api/v1/admin/registries/{registry}/beta-channel/{principal_type}/{principal_id}` — remove member

---

### 3.6 `[ip_blocking]` (optional)

Automatically blocks IP addresses that trigger too many violation events within a rolling time window — similar to fail2ban. Blocked IPs receive HTTP 403 with an `X-Block-Expires` header until the ban expires.

```toml
[ip_blocking]
enabled               = true
violation_threshold   = 10      # violations before auto-block
violation_window_secs = 300     # counting window in seconds (5 min)
ban_duration_secs     = 3600    # how long to block the IP (1 hour)
trigger_on_status     = [429, 401]   # HTTP response codes that count as violations
trusted_proxies       = ["10.0.0.1"] # IPs whose X-Forwarded-For header is trusted
```

| Field | Type | Default | Notes |
|---|---|---|---|
| `enabled` | bool | `false` | Enable/disable the middleware |
| `violation_threshold` | int | `10` | Number of violations before auto-block |
| `violation_window_secs` | int | `300` | Window length for counting violations |
| `ban_duration_secs` | int | `3600` | How long the auto-block lasts |
| `trigger_on_status` | int[] | `[429, 401]` | Response status codes that count as violations |
| `trusted_proxies` | string[] | `[]` | Upstream proxy IPs allowed to set `X-Forwarded-For` |

**Backends:** Block state is stored in the same backend as the cache (`memory`, `postgres`, or `redis`). Use `postgres` or `redis` for multi-instance deployments.

**Manual management:** Admins can manage blocks via the back-office API:
- `GET    /api/v1/admin/ip-blocks` — list currently blocked IPs
- `POST   /api/v1/admin/ip-blocks` — body: `{ "ip": "1.2.3.4", "reason": "...", "duration_secs": 3600 }`
- `DELETE /api/v1/admin/ip-blocks/{ip}` — unblock an IP

**Trusted proxies:** When a request arrives through a known reverse proxy, batlehub reads the real client IP from `X-Forwarded-For` only if the TCP peer address appears in `trusted_proxies`. Without this configuration, `X-Forwarded-For` is ignored to prevent header-spoofing attacks.

---

### 3.7 `[otel]` (optional)

Enables OpenTelemetry distributed tracing via OTLP gRPC.

```toml
[otel]
endpoint = "http://localhost:4317"
service_name = "batlehub"   # default
```

| Field | Type | Default | Notes |
|---|---|---|---|
| `endpoint` | string | — | OTLP gRPC endpoint |
| `service_name` | string | `"batlehub"` | Service name reported in traces |

The entire section can be enabled without a config file change by setting `PROXY_CACHE__OTEL__ENDPOINT` — the section is created automatically if the env var is present.

---

### 3.8 `[proxy]` (optional)

A **global** HTTP/SOCKS proxy that applies to all upstream registry requests. Individual registries that define their own `[registries.proxy]` section override this global setting for that registry only.

```toml
[proxy]
url      = "http://proxy.corp.example.com:3128"
# username = "proxyuser"   # optional
# password = "${PROXY_PASSWORD}"   # optional
# no_proxy = "localhost,10.0.0.0/8,internal.example.com"  # optional
```

| Field | Type | Required | Notes |
|---|---|---|---|
| `url` | string | yes | Proxy URL (`http://`, `https://`, or `socks5://`). Credentials can be embedded: `http://user:pass@proxy:3128`. |
| `username` | string | no | Proxy Basic-auth username. |
| `password` | string | no | Proxy Basic-auth password. Use `${VAR}` to keep secrets out of the file. |
| `no_proxy` | string | no | Comma-separated hosts/domains/CIDRs to bypass the proxy for. |

The entire section can be set without touching the config file via environment variables:

```sh
export PROXY_CACHE__PROXY__URL="http://proxy.corp.example.com:3128"
export PROXY_CACHE__PROXY__USERNAME="proxyuser"
export PROXY_CACHE__PROXY__PASSWORD="s3cr3t"
export PROXY_CACHE__PROXY__NO_PROXY="localhost,10.0.0.0/8"
```

`PROXY_CACHE__PROXY__URL` creates the `[proxy]` section automatically if it is not present in the TOML file, so a minimal deployment only needs the single env var set.

> **Precedence:** per-registry `[registries.proxy]` > global `[proxy]`. When neither is set, the underlying HTTP client reads the standard `HTTP_PROXY` / `HTTPS_PROXY` / `NO_PROXY` env vars automatically. Configuring any proxy via the config file disables env-var proxy reading for that registry's client — to forward those env vars in, see [Forwarding `HTTP_PROXY` into the config](#forwarding-http_proxy-into-the-config) above.

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

BatleHub supports two complementary mechanisms for injecting environment variable values into the config file.

### 5.1 Inline substitution — `${VAR_NAME}` {#env-inline}

Write `${VAR_NAME}` anywhere inside a TOML **string value**. BatleHub replaces every placeholder with the corresponding environment variable's value before the TOML is parsed. This is the recommended way to inject secrets such as OIDC client secrets, upstream auth tokens, or passwords.

**Rules:**

| Syntax | Meaning |
|---|---|
| `${VAR_NAME}` | Replaced with `$VAR_NAME` at startup. Error if the variable is not set. |
| `$${VAR_NAME}` | Produces the literal string `${VAR_NAME}` — no lookup performed. |
| Any other `$` | Left unchanged. |

> If a referenced variable is not set, BatleHub exits immediately with a clear error message naming the missing variable. There is no silent fallback or empty-string default — this is intentional to prevent misconfigured deployments from starting.

**OIDC client secret:**

```toml
[[auth]]
type = "oidc"
issuer_url = "https://sso.example.com/application/o/batlehub/"
client_id   = "batlehub"
client_secret = "${OIDC_CLIENT_SECRET}"   # export OIDC_CLIENT_SECRET=<value>
redirect_uri  = "https://hub.example.com/api/v1/auth/oidc/callback"
```

**Upstream registry — Bearer token:**

```toml
[[registries]]
type = "npm"
name = "internal-npm"
upstreams = ["https://gitea.corp.example.com/api/packages/myorg/npm"]

[registries.upstream_auth]
type  = "bearer"
token = "${INTERNAL_NPM_TOKEN}"   # export INTERNAL_NPM_TOKEN=npat-xxxx
```

**Upstream registry — Basic auth:**

```toml
[[registries]]
type     = "cargo"
name     = "internal-cargo"
upstreams = ["https://nexus.corp.example.com/repository/cargo-proxy/"]

[registries.upstream_auth]
type     = "basic"
username = "deploy"
password = "${INTERNAL_CARGO_PASSWORD}"   # export INTERNAL_CARGO_PASSWORD=s3cr3t
```

**Upstream registry — Custom header:**

```toml
[[registries]]
type     = "npm"
name     = "api-keyed-npm"
upstreams = ["https://nexus.corp.example.com/repository/npm-proxy/"]

[registries.upstream_auth]
type  = "header"
name  = "X-API-Key"
value = "${INTERNAL_NPM_API_KEY}"   # export INTERNAL_NPM_API_KEY=my-api-key
```

**Kubernetes / Docker Compose:** mount a Secret as an env var and reference it from the config file.

```yaml
# docker-compose.yml
services:
  batlehub:
    env_file: .env.secrets   # OIDC_CLIENT_SECRET=...
    volumes:
      - ./config.toml:/etc/batlehub/config.toml:ro
```

```yaml
# Kubernetes Deployment
env:
  - name: OIDC_CLIENT_SECRET
    valueFrom:
      secretKeyRef:
        name: batlehub-secrets
        key: oidc-client-secret
```

**Escaping:** if a config value legitimately needs the string `${...}` (e.g. a URL template), write `$${...}`:

```toml
# This stores the literal string "${MY_VAR}" — no variable lookup:
some_template = "$${MY_VAR}/suffix"
```

---

### 5.2 Named overrides — `PROXY_CACHE__*` {#env-named}

A fixed set of top-level fields can also be overridden via named environment variables. These are useful for container deployments where the config file is baked into the image and you need to tweak infrastructure addresses (host, port, DB URL) without rebuilding.

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
| `PROXY_CACHE__PROXY__URL` | `proxy.url` | Creates the `[proxy]` section if absent; applies to all registries |
| `PROXY_CACHE__PROXY__USERNAME` | `proxy.username` | |
| `PROXY_CACHE__PROXY__PASSWORD` | `proxy.password` | |
| `PROXY_CACHE__PROXY__NO_PROXY` | `proxy.no_proxy` | |

> Storage env-var overrides only work with the **single-backend** `[storage]` form. Multi-backend configs (`[[storage.backends]]`) must be changed in the file.

> **Choosing between the two mechanisms:** use `${VAR_NAME}` placeholders for **secrets** (auth tokens, passwords, client secrets) — they work for any field and keep credentials out of the TOML file. Use the `PROXY_CACHE__*` variables for **infrastructure addresses** (database URL, storage path, host/port) where the value is not secret but varies between environments.

---

## 6. Worked Examples

### 6.1 Local Development

Minimal setup for local development: static token auth, filesystem cache, npm and Cargo open to anonymous reads.

```toml
[server]
port = 8080

[database]
type = "postgresql"
url = "postgresql://batlehub:changeme@localhost:5432/batlehub"

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
url = "postgresql://batlehub:changeme@db:5432/batlehub"

[[auth]]
type = "oidc"
issuer_url = "https://sso.example.com/application/o/batlehub/"
client_id = "batlehub"
client_secret = "my-client-secret"
redirect_uri = "https://batlehub.example.com/api/v1/auth/oidc/callback"
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
url = "postgresql://batlehub:changeme@postgres-svc:5432/batlehub"

[[auth]]
type = "kubernetes"
# api_server, ca_cert_path, and token_path all default to in-cluster values

[auth.role_mappings]
"system:serviceaccount:prod:ci-deployer"  = "admin"
"system:serviceaccounts:staging"          = "user"
"system:serviceaccounts:dev"              = "user"

[storage]
type = "s3"
bucket = "batlehub-artifacts"
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

batlehub's ServiceAccount needs permission to call the Kubernetes TokenReview API:

```yaml
apiVersion: rbac.authorization.k8s.io/v1
kind: ClusterRole
metadata:
  name: batlehub-tokenreview
rules:
  - apiGroups: ["authentication.k8s.io"]
    resources: ["tokenreviews"]
    verbs: ["create"]
---
apiVersion: rbac.authorization.k8s.io/v1
kind: ClusterRoleBinding
metadata:
  name: batlehub-tokenreview
roleRef:
  apiGroup: rbac.authorization.k8s.io
  kind: ClusterRole
  name: batlehub-tokenreview
subjects:
  - kind: ServiceAccount
    name: batlehub
    namespace: batlehub
```

### 6.4 Go Module Proxy

Proxy Go modules through `proxy.golang.org` with a release age gate and admin-only bypass. All five GOPROXY endpoints (`.info`, `.mod`, `.zip`, `@latest`, `@v/list`) are served transparently.

```toml
[server]
port = 8080

[database]
type = "postgresql"
url = "postgresql://batlehub:changeme@localhost:5432/batlehub"

[[auth]]
type = "token"

[[auth.tokens]]
value = "admin-token"
role  = "admin"
user_id = "admin"

[storage]
type = "filesystem"
path = "./cache"

[[registries]]
type     = "goproxy"
name     = "go"
# Default upstream is https://proxy.golang.org.
# For an air-gapped environment, point at an internal mirror:
# upstreams = ["https://goproxy.internal.example.com"]

[registries.rbac]
anonymous = []
user      = ["releases:read", "source:read"]
admin     = ["*"]

# Block modules published within the last hour (supply-chain delay window).
[[registries.rules]]
kind         = "release_age_gate"
min_age_secs = 3600
bypass_roles = ["admin"]
```

Configure the go toolchain:

```sh
export GONOSUMCHECK="*"
export GONOSUMDB="*"
export GOPROXY="http://localhost:8080/proxy/go,direct"

# Fetch a specific version — served from cache after the first download
go get golang.org/x/text@v0.3.7
```

### 6.5 Self-Hosted Private Registries

Proxy a private Gitea npm registry with a Bearer token and a self-signed CA certificate. Identical pattern works for Cargo, Go, and OpenVSX.

```toml
[server]
port = 8080

[database]
type = "postgresql"
url  = "postgresql://batlehub:changeme@localhost:5432/batlehub"

[[auth]]
type = "token"

[[auth.tokens]]
value   = "admin-token"
role    = "admin"
user_id = "admin"

[storage]
type = "filesystem"
path = "./cache"

# Public npm registry (no auth needed)
[[registries]]
type = "npm"
name = "npm-public"

[registries.rbac]
anonymous = ["releases:read", "source:read"]
user      = ["releases:read", "source:read"]
admin     = ["*"]

# Private Gitea npm registry
[[registries]]
type      = "npm"
name      = "npm-internal"
upstreams = ["https://gitea.corp.example.com/api/packages/myorg/npm"]

[registries.upstream_auth]
type  = "bearer"
token = "npat-xxxx"

[registries.tls]
ca_cert_path = "/etc/ssl/corp-ca.pem"

[registries.rbac]
anonymous = []
user      = ["releases:read", "source:read"]
admin     = ["*"]

# Private Cargo registry on Nexus with Basic auth
[[registries]]
type      = "cargo"
name      = "cargo-internal"
upstreams = ["https://nexus.corp.example.com/repository/cargo-proxy/"]
index_url = "https://nexus.corp.example.com/repository/cargo-index/"

[registries.upstream_auth]
type     = "basic"
username = "deploy"
password = "s3cr3t"

[registries.tls]
ca_cert_path = "/etc/ssl/corp-ca.pem"

[registries.rbac]
anonymous = []
user      = ["releases:read", "source:read"]
admin     = ["*"]
```

### 6.6 Private Cargo Registry (local / hybrid mode) {#66-private-cargo-registry-local--hybrid-mode}

> For a step-by-step publishing walkthrough, see [`docs/publishing.md § Cargo`](publishing.md#4-cargo).

#### Pure local registry (no upstream)

Use this when you want a completely private Cargo registry that does not proxy crates.io.

```toml
[[registries]]
type = "cargo"
name = "internal"
mode = "local"          # BatleHub is the only source; no upstream needed

[registries.rbac]
anonymous = []
user      = ["source:read"]  # allow download but not publish (publish checks role in service)
admin     = ["*"]
```

Configure Cargo on the client side (`~/.cargo/config.toml` or `.cargo/config.toml` in the project root):

```toml
[registries.internal]
index = "sparse+https://batlehub.example.com/proxy/internal/registry/"

[registry]
token = "<your-user-token>"   # or set CARGO_REGISTRIES_INTERNAL_TOKEN env var
```

Publish a crate:

```sh
cargo publish --registry internal
```

Depend on a privately published crate:

```toml
# Cargo.toml
[dependencies]
my-lib = { version = "0.1", registry = "internal" }
```

#### Hybrid registry (local crates + crates.io fallback)

Use this when you want to publish internal crates while still proxying the public crates.io registry through the same endpoint.

```toml
[[registries]]
type      = "cargo"
name      = "everything"
mode      = "hybrid"
upstreams = ["https://static.crates.io/crates"]
index_url = "https://index.crates.io"

[registries.rbac]
anonymous = ["source:read"]   # public crates readable without auth
user      = ["source:read"]
admin     = ["*"]
```

Client configuration:

```toml
[registries.everything]
index = "sparse+https://batlehub.example.com/proxy/everything/registry/"
token = "<your-user-token>"
```

In hybrid mode, `cargo fetch` and `cargo build` work transparently:
- A dependency that was published to BatleHub is served from local storage.
- Any other dependency falls back to crates.io through the configured upstream.

#### Endpoints exposed by local / hybrid registries

| Method | Path | Used by |
|--------|------|---------|
| `GET` | `/proxy/{registry}/registry/config.json` | `cargo` client on first connect |
| `GET` | `/proxy/{registry}/registry/{path}` | sparse index lookup |
| `GET` | `/proxy/{registry}/{name}/{version}/download` | `.crate` download |
| `PUT` | `/proxy/{registry}/api/v1/crates/new` | `cargo publish` |
| `DELETE` | `/proxy/{registry}/api/v1/crates/{name}/{version}/yank` | `cargo yank` |
| `PUT` | `/proxy/{registry}/api/v1/crates/{name}/{version}/unyank` | `cargo yank --undo` |
| `GET` | `/proxy/{registry}/api/v1/crates/{name}/owners` | `cargo owner --list` |

---

### 6.7 Private npm Registry (local / hybrid mode) {#67-private-npm-registry-local--hybrid-mode}

> For a step-by-step publishing walkthrough, see [`docs/publishing.md § npm`](publishing.md#3-npm).

#### Pure local npm registry (no upstream)

Use this when you want a completely private npm registry for internal packages.

```toml
[[registries]]
type = "npm"
name = "internal-npm"
mode = "local"

[registries.rbac]
anonymous = []
user      = ["source:read"]
admin     = ["*"]
```

Configure npm on the client side:

```sh
# ~/.npmrc or project .npmrc
@myorg:registry=https://batlehub.example.com/proxy/internal-npm/
//batlehub.example.com/proxy/internal-npm/:_authToken=<your-user-token>
```

Publish and install:

```sh
# publish
npm publish --registry https://batlehub.example.com/proxy/internal-npm/

# install a scoped package
npm install @myorg/my-package
```

#### Hybrid npm registry (local packages + upstream fallback)

```toml
[[registries]]
type      = "npm"
name      = "everything-npm"
mode      = "hybrid"
upstreams = ["https://registry.npmjs.org"]

[registries.rbac]
anonymous = ["releases:read"]
user      = ["releases:read", "source:read"]
admin     = ["*"]
```

In hybrid mode `npm install` transparently serves internal packages from local storage and public packages from the upstream registry.

#### Endpoints exposed by local / hybrid npm registries

| Method | Path | Used by |
|--------|------|---------|
| `GET` | `/proxy/{registry}/{package}` | packument (all versions) |
| `GET` | `/proxy/{registry}/{package}/{version}` | single version metadata |
| `GET` | `/proxy/{registry}/{package}/{version}/tarball` | tarball download |
| `PUT` | `/proxy/{registry}/{package}` | `npm publish` |
| `POST` | `/proxy/{registry}/-/npm/v1/audit/quick` | `npm audit` (proxied upstream) |

---

### 6.8 Private VS Code Extension Registry (local / hybrid mode) {#68-private-vs-code-extension-registry-local--hybrid-mode}

> For a step-by-step publishing walkthrough, see [`docs/publishing.md § VS Code Extensions`](publishing.md#5-vs-code-extensions-openvsx--vs-code-marketplace).

Use this when you want to distribute private VS Code extensions through a self-hosted registry.

#### Pure local extension registry

```toml
[[registries]]
type = "openvsx"     # or "vscode-marketplace"
name = "internal-ext"
mode = "local"

[registries.rbac]
anonymous = []
user      = ["source:read"]
admin     = ["*"]
```

Configure VS Code to use the registry (`.vscode/settings.json` or user settings):

```json
{
  "vscode-extension-marketplace.serviceUrl": "https://batlehub.example.com/proxy/internal-ext"
}
```

Upload an extension (raw VSIX bytes):

```sh
curl -X PUT \
  -H "Authorization: Bearer <your-user-token>" \
  -H "Content-Type: application/octet-stream" \
  --data-binary @my-org.my-ext-1.0.0.vsix \
  "https://batlehub.example.com/proxy/internal-ext/my-org.my-ext/1.0.0/vsix"
```

Download an extension:

```sh
curl -H "Authorization: Bearer <token>" \
  "https://batlehub.example.com/proxy/internal-ext/my-org.my-ext/1.0.0/vsix" \
  -o my-org.my-ext-1.0.0.vsix
```

#### Endpoints exposed by local / hybrid VS Code extension registries

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/proxy/{registry}/{extension_id}/{version}/vsix` | Download VSIX |
| `PUT` | `/proxy/{registry}/{extension_id}/{version}/vsix` | Upload VSIX |

Extension IDs follow the `{publisher}.{name}` convention (e.g. `my-org.my-ext`).

---

### 6.9 Private Go Module Proxy (local / hybrid mode) {#69-private-go-module-proxy-local--hybrid-mode}

> For a step-by-step publishing walkthrough, see [`docs/publishing.md § Go Modules`](publishing.md#6-go-modules).

#### Pure local Go module proxy (no upstream)

Use this to host private Go modules without exposing them to the public internet.

```toml
[[registries]]
type = "goproxy"
name = "internal-go"
mode = "local"

[registries.rbac]
anonymous = []
user      = ["source:read"]
admin     = ["*"]
```

**Upload a module** by pushing the Go module zip archive. BatleHub extracts `go.mod` automatically and generates version metadata from the upload timestamp:

```sh
# Build the module zip (standard Go module zip format)
go mod zip example.com/mymod@v1.0.0 . --mod-zip /tmp/mymod-v1.0.0.zip

# Upload to BatleHub
curl -X PUT -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/zip" \
  --data-binary @/tmp/mymod-v1.0.0.zip \
  "https://batlehub.example.com/proxy/internal-go/example.com/mymod/@v/v1.0.0.zip"
```

**Use the private proxy** in the go toolchain:

```sh
export GONOSUMCHECK="*"
export GONOSUMDB="*"
export GOPROXY="https://batlehub.example.com/proxy/internal-go,direct"
go get example.com/mymod@v1.0.0
```

Or add to `go.env`:

```sh
go env -w GONOSUMCHECK="*"
go env -w GONOSUMDB="*"
go env -w GOPROXY="https://batlehub.example.com/proxy/internal-go,direct"
```

#### Hybrid Go module proxy (local modules + upstream fallback)

```toml
[[registries]]
type      = "goproxy"
name      = "everything-go"
mode      = "hybrid"
upstreams = ["https://proxy.golang.org"]

[registries.rbac]
anonymous = ["releases:read", "source:read"]
user      = ["releases:read", "source:read"]
admin     = ["*"]
```

In hybrid mode, `go get` and `go mod download` transparently serve internal modules from local storage and public modules from `proxy.golang.org` (or whichever upstream you configure).

#### Endpoints exposed by local / hybrid Go module proxies

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/proxy/{registry}/{module}/@latest` | Latest version info JSON |
| `GET` | `/proxy/{registry}/{module}/@v/list` | Newline-separated version list |
| `GET` | `/proxy/{registry}/{module}/@v/{version}.info` | Version metadata JSON |
| `GET` | `/proxy/{registry}/{module}/@v/{version}.mod` | `go.mod` content |
| `GET` | `/proxy/{registry}/{module}/@v/{version}.zip` | Module source zip archive |
| `PUT` | `/proxy/{registry}/{module}/@v/{version}.zip` | Upload module zip (triggers publish) |

Module paths may contain slashes (e.g. `golang.org/x/text`).

---

### 6.10 Multi-Backend Storage {#610-multi-backend-storage}

Default filesystem backend for all registries, dedicated S3 backend for large GitHub release artifacts.

```toml
[server]
port = 8080

[database]
type = "postgresql"
url = "postgresql://batlehub:changeme@localhost:5432/batlehub"

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

### 6.11 Terraform Provider Cache {#611-terraform-provider-cache}

Cache Terraform provider binaries locally so `terraform init` doesn't hit `registry.terraform.io` on every CI run.

```toml
[[registries]]
type = "terraform"
name = "terraform"
# upstreams defaults to ["https://registry.terraform.io"]

[registries.rbac]
anonymous = []
user      = ["releases:read", "source:read"]
admin     = ["*"]

[registries.cache]
metadata_ttl_secs = 300   # re-check version lists every 5 min
# artifact_ttl_secs not set — provider binaries are cached forever
```

Configure each developer's or CI runner's Terraform CLI:

```hcl
# ~/.terraformrc  (or %APPDATA%/terraform.rc on Windows)
# CI: write this file during pipeline setup
provider_installation {
  network_mirror {
    url = "https://batlehub.example.com/proxy/terraform/"
  }
}
```

After the first `terraform init`, subsequent runs use the locally cached binaries. Provider checksums are cached alongside the download metadata, so Terraform's checksum verification still passes.

---

### 6.12 Private Maven Registry (local / hybrid mode) {#612-private-maven-registry-local--hybrid-mode}

Host private Maven/Gradle artifacts (`mvn deploy`, `gradle publish`) so teams never need an external Nexus or Artifactory instance.

```toml
[[registries]]
type = "maven"
name = "internal-maven"
mode = "local"          # BatleHub is the only source; no upstream needed

[registries.rbac]
user  = ["releases:read", "source:read"]
admin = ["*"]
```

For hybrid mode (serve private artifacts first, fall back to Maven Central for everything else):

```toml
[[registries]]
type      = "maven"
name      = "internal-maven"
mode      = "hybrid"
upstreams = ["https://repo1.maven.org/maven2"]

[registries.rbac]
user  = ["releases:read", "source:read"]
admin = ["*"]
```

#### Client setup — Maven

Add credentials to `~/.m2/settings.xml` (the `<id>` must match the `<distributionManagement>` `<id>` in your POM):

```xml
<settings>
  <servers>
    <server>
      <id>internal-maven</id>
      <username>your-user-id</username>
      <password>your-bearer-token</password>
    </server>
  </servers>
  <mirrors>
    <mirror>
      <id>internal-maven</id>
      <name>BatleHub Maven</name>
      <url>https://batlehub.example.com/proxy/internal-maven/maven2/</url>
      <mirrorOf>*</mirrorOf>
    </mirror>
  </mirrors>
</settings>
```

#### Publish setup — pom.xml

```xml
<distributionManagement>
  <repository>
    <id>internal-maven</id>
    <url>https://batlehub.example.com/proxy/internal-maven/maven2/</url>
  </repository>
</distributionManagement>
```

```sh
mvn deploy
```

#### Publish setup — Gradle (settings.gradle.kts)

```kotlin
dependencyResolutionManagement {
    repositories {
        maven {
            url = uri("https://batlehub.example.com/proxy/internal-maven/maven2/")
            credentials {
                username = "your-user-id"
                password = "your-bearer-token"
            }
        }
    }
}
```

#### How it works

Maven/Gradle upload `.jar` and checksum files **before** the `.pom`. BatleHub stores each non-POM file directly in object storage. When the `.pom` arrives, BatleHub parses it (extracting `groupId`, `artifactId`, `version`, `packaging`, `description`) and commits a `local_packages` row via the three-phase publish protocol. Subsequent `GET` requests for `maven-metadata.xml` return XML generated from the database rather than a cached file.

#### Endpoints exposed by local / hybrid Maven registries

| Endpoint | Method | Description |
|---|---|---|
| `/proxy/{registry}/maven2/{path}` | GET | Serve artifact from local storage (or proxy in hybrid mode) |
| `/proxy/{registry}/maven2/{group}/{artifact}/maven-metadata.xml` | GET | Generated from DB; never cached |
| `/proxy/{registry}/maven2/{path}` | PUT | Upload artifact (`.pom` commits version, other files stored directly) |

---

### 6.13 Private Terraform Registry (local / hybrid mode) {#613-private-terraform-registry-local--hybrid-mode}

Publish and serve private Terraform modules and providers without an external registry.

```toml
[[registries]]
type = "terraform"
name = "internal-tf"
mode = "local"

[registries.rbac]
user  = ["releases:read", "source:read"]
admin = ["*"]
```

For hybrid mode (serve private providers/modules first, proxy `registry.terraform.io` for everything else):

```toml
[[registries]]
type      = "terraform"
name      = "internal-tf"
mode      = "hybrid"
upstreams = ["https://registry.terraform.io"]

[registries.rbac]
user  = ["releases:read", "source:read"]
admin = ["*"]
```

#### Client setup — .terraformrc

```hcl
# ~/.terraformrc  (or %APPDATA%/terraform.rc on Windows)
provider_installation {
  network_mirror {
    url = "https://batlehub.example.com/proxy/internal-tf/"
  }
}

credentials "batlehub.example.com" {
  token = "your-bearer-token"
}
```

#### Publishing a private module

```sh
# Package your module as a tar.gz, then upload:
tar czf my-module.tar.gz -C ./module-dir .
curl -X POST \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/gzip" \
  --data-binary @my-module.tar.gz \
  "https://batlehub.example.com/proxy/internal-tf/v1/modules/namespace/name/provider/1.0.0"
```

The response includes an `X-Terraform-Get` header pointing to the stored artifact download URL.

#### Publishing a private provider

Step 1 — upload the version manifest (JSON describing protocols and available platforms):

```sh
curl -X POST \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{
    "version": "5.0.0",
    "protocols": ["5.0"],
    "platforms": [
      {"os": "linux",  "arch": "amd64",  "filename": "terraform-provider-mycloud_5.0.0_linux_amd64.zip",  "shasum": "abc123..."},
      {"os": "darwin", "arch": "arm64",  "filename": "terraform-provider-mycloud_5.0.0_darwin_arm64.zip", "shasum": "def456..."}
    ]
  }' \
  "https://batlehub.example.com/proxy/internal-tf/v1/providers/myorg/mycloud/versions"
```

Step 2 — upload each platform binary:

```sh
curl -X PUT \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/zip" \
  --data-binary @terraform-provider-mycloud_5.0.0_linux_amd64.zip \
  "https://batlehub.example.com/proxy/internal-tf/v1/providers/myorg/mycloud/5.0.0/artifact/linux/amd64"
```

#### Yank a version (admin)

```sh
curl -X POST \
  -H "Authorization: Bearer <admin-token>" \
  -H "Content-Type: application/json" \
  -d '{"packages":[{"name":"modules/namespace/name/provider","version":"1.0.0"}]}' \
  "https://batlehub.example.com/api/v1/admin/registries/internal-tf/bulk-yank"
```

---

### 6.14 Rate Limiting — Per-User + Per-Group {#614-rate-limiting}

Protect a public-facing npm registry: each user gets 200 requests per minute; CI bot group members share a higher 2000 req/min pool; free-tier group is limited to 50 req/min.

```toml
[[registries]]
type = "npm"
name = "npm"

[registries.rbac]
anonymous = []
user      = ["releases:read", "source:read"]
admin     = ["*"]

[registries.rate_limit]
requests_per_window = 200    # per authenticated user
window_secs         = 60
enforcement         = "block"

# CI bots share a single 2000/min pool across all members:
[[registries.rate_limit.groups]]
name                = "oidc:ci-bots"
requests_per_window = 2000
window_secs         = 60

# Free-tier users share a stricter 50/min pool:
[[registries.rate_limit.groups]]
name                = "oidc:free-tier"
requests_per_window = 50
window_secs         = 60
enforcement         = "warn"   # warn instead of block for free-tier
```

A CI bot that belongs to `oidc:ci-bots` consumes one token from both its personal 200/min bucket and the shared `oidc:ci-bots` 2000/min bucket on each request. If either is exhausted, the request is blocked (or warned, per the per-group enforcement override).

Response when a user exceeds their limit:

```
HTTP/1.1 429 Too Many Requests
X-RateLimit-Limit: 200
Retry-After: 42
X-RateLimit-Reset: 1716556842
Content-Type: application/json

{"error":"rate limit exceeded","retry_after_secs":42}
```

---

### 6.15 Private Composer Registry (local / hybrid mode) {#615-private-composer-registry-local--hybrid-mode}

Publish and serve private PHP packages without an external Packagist-compatible registry.

```toml
[[registries]]
type = "composer"
name = "internal-composer"
mode = "local"

[registries.rbac]
user  = ["releases:read", "source:read"]
admin = ["*"]
```

For hybrid mode (serve private packages first, proxy Packagist for everything else):

```toml
[[registries]]
type      = "composer"
name      = "internal-composer"
mode      = "hybrid"
upstreams = ["https://repo.packagist.org"]

[registries.rbac]
user  = ["releases:read", "source:read"]
admin = ["*"]
```

#### Client setup — composer.json

Add a repository entry to your project's `composer.json`:

```json
{
  "repositories": [
    {
      "type": "composer",
      "url": "https://batlehub.example.com/proxy/internal-composer/",
      "options": {
        "http": {
          "header": ["Authorization: Bearer your-token"]
        }
      }
    }
  ]
}
```

Alternatively, keep credentials out of `composer.json` by storing them in `auth.json`:

```json
{
  "http-basic": {
    "batlehub.example.com": {
      "username": "user",
      "password": "your-token"
    }
  }
}
```

#### Publishing a package

Create a ZIP archive containing a valid `composer.json` at its root or inside a single top-level directory (GitHub archive layout is also accepted). The `composer.json` must include `name` (in `vendor/package` format) and `version` fields:

```sh
# Create the archive
zip -r symfony-console-7.1.0.zip symfony-console-7.1.0/

# Publish
curl -X POST \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/zip" \
  --data-binary @symfony-console-7.1.0.zip \
  "https://batlehub.example.com/proxy/internal-composer/api/upload"
```

The `version` field in the uploaded `composer.json` determines the published version. It can be overridden by appending `?version=<version>` to the upload URL.

#### Yanking a version

```sh
curl -X DELETE \
  -H "Authorization: Bearer <token>" \
  "https://batlehub.example.com/proxy/internal-composer/api/packages/my-vendor/my-package/versions/1.0.0"
```

Yanked versions are hidden from `p2/` metadata and return 404 on download.

---

## 7. CLI Reference

```
batlehub --config config.toml          # start the server (default: config.toml)
batlehub dump-spec                     # print the OpenAPI JSON spec to stdout
batlehub hash-token <token>            # generate an Argon2id PHC hash for a static token
```

### `dump-spec`

Redirect the spec to a file for use with code generators:

```sh
batlehub dump-spec > openapi.json
```

### `hash-token`

Generates an Argon2id PHC hash that can be stored in `[[auth.tokens]].value` instead of a raw token string. The raw token is only required at generation time and does not need to be stored anywhere.

```sh
# Generate a hash
batlehub hash-token my-secret-token
# $argon2id$v=19$m=65536,t=3,p=4$<salt>$<hash>

# Paste the output directly into the config:
# [[auth.tokens]]
# value = "$argon2id$v=19$m=65536,t=3,p=4$..."
# role = "admin"
```

See [§3.3.1 Argon2id hashed token values](#argon2id-hashed-token-values-recommended-for-production) for full context.

---

## 8. User-Generated API Tokens

Users authenticated via OIDC can create personal long-lived API tokens without going through SSO each time. This is the recommended approach for CI/CD pipelines when Kubernetes service account auth is not available.

```sh
# Create a token (valid for 30 days, cannot exceed creator's role)
curl -X POST https://batlehub.example.com/api/v1/auth/tokens \
  -H "Authorization: Bearer <oidc-access-token>" \
  -H "Content-Type: application/json" \
  -d '{"name": "ci-token", "expires_in_days": 30}'

# List active tokens
curl https://batlehub.example.com/api/v1/auth/tokens \
  -H "Authorization: Bearer <oidc-access-token>"

# Revoke a token
curl -X DELETE https://batlehub.example.com/api/v1/auth/tokens/<token-id> \
  -H "Authorization: Bearer <oidc-access-token>"
```

Key properties:
- Token values are shown **once** at creation time; store them securely.
- A token's role cannot exceed the role of the user who created it.
- Token auth (`type = "token"`) in the config file and user-generated tokens are two separate mechanisms; user-generated tokens are always available to OIDC-authenticated users with no extra `[[auth]]` entry needed.

---

## 9. Hot Reload & Dynamic Config

BatleHub can reload its configuration at runtime without restarting the process. The following components are hot-swappable:

- Registry list (add, remove, or update a registry)
- Per-registry RBAC (`anonymous`, `user`, `admin`, group-based access)
- Per-registry policy rules (age gate, deny latest)
- Per-registry versioning, signing, and beta-channel configuration
- Artifact size limit

The following components **require a process restart**:
- Server host / port
- Database URL or connection pool size
- Auth providers (`[[auth]]`)
- Storage backends

### 9.1 File Watcher

When the config file changes on disk, BatleHub automatically validates the new config (schema check + connectivity probes) and stores a **pending reload**. The admin then confirms or discards it via the UI or API. Pending reloads expire after 10 minutes.

The file watcher is enabled by default. Disable it with:

```sh
BATLEHUB_DISABLE_HOT_RELOAD=1 batlehub --config config.toml
```

Use this when `config.toml` is mounted as a read-only Kubernetes ConfigMap.

### 9.2 API Endpoints

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/api/v1/admin/config/reload` | Immediate reload: validate + apply atomically |
| `GET` | `/api/v1/admin/config/pending` | Get pending reload diff (404 if none) |
| `POST` | `/api/v1/admin/config/pending/apply` | Apply the pending reload |
| `DELETE` | `/api/v1/admin/config/pending` | Discard the pending reload |
| `GET` | `/api/v1/admin/config/changes` | Paginated audit history (`?page=0&per_page=50`) |

```sh
# CI/CD: apply a new config atomically
curl -s -X POST \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  http://localhost:8080/api/v1/admin/config/reload

# Two-step flow: let the file watcher load a pending, then apply from CI
curl -s -X POST \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  http://localhost:8080/api/v1/admin/config/pending/apply
```

All reloads (applied or rejected) are written to the `config_changes` table with the diff, trigger source, and operator identity.

### 9.3 Global Admin Banner

Administrators can broadcast a message to all website visitors:

```sh
# Set a warning banner
curl -s -X PUT \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"message":"Maintenance window in 30 min","level":"warning"}' \
  http://localhost:8080/api/v1/admin/banner

# Clear it
curl -s -X DELETE \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  http://localhost:8080/api/v1/admin/banner
```

The frontend polls `GET /api/v1/banner` (no auth required) every 30 seconds. The banner backend uses the same infrastructure as the metadata cache:

| `[cache] type` | Banner storage |
|----------------|---------------|
| `"memory"` | In-process — not shared across replicas |
| `"redis"` | Redis — shared across all HA replicas |
| `"postgres"` | `system_kv` table — shared across all HA replicas |

---

## 10. Self-Hosted / Private Registries

Any registry can proxy a self-hosted or private upstream by combining `upstream_auth` and `tls` fields. Both are optional and independent of each other.

### Upstream authentication

Three schemes are available via `[registries.upstream_auth]`:

| `type` | Use case | Required fields |
|--------|----------|-----------------|
| `bearer` | Gitea, Forgejo, GitHub Enterprise, Artifactory API tokens | `token` |
| `basic` | Nexus, Artifactory (password), most HTTP-authenticated feeds | `username`, `password` |
| `header` | Any registry using a custom header (e.g. `X-API-Key`) | `name`, `value` |

Bearer tokens are sent as `Authorization: Bearer <token>`. Basic credentials are attached per-request as HTTP Basic auth. Custom headers are injected as default headers on every upstream request.

### Custom CA certificates

When the upstream serves a certificate signed by a private CA, add the CA certificate to the system trust store **or** point `tls.ca_cert_path` at a PEM file:

```toml
[registries.tls]
ca_cert_path = "/etc/ssl/corp-ca.pem"
```

This setting is per-registry, so you can mix public registries (no TLS config needed) with private registries that use a corporate CA — all in the same `config.toml`.

### Using `upstream_auth` and `tls` together

Both fields can appear on the same registry block:

```toml
[[registries]]
type      = "npm"
name      = "npm-private"
upstreams = ["https://nexus.corp.example.com/repository/npm-proxy/"]

[registries.upstream_auth]
type  = "header"
name  = "X-API-Key"
value = "my-api-key"

[registries.tls]
ca_cert_path = "/etc/ssl/corp-ca.pem"
```

### Supported registry types

All registry types support `upstream_auth` and `tls`: `github`, `npm`, `cargo`, `openvsx`, `vscode-marketplace`, `goproxy`, `maven`, `terraform`. For `cargo`, the sparse index proxy (the `index_url` endpoint) also uses the same credentials and TLS settings.

### Mixing a private upstream with a public fallback

`upstream_auth` is per-registry block, not per-URL. When `upstreams` lists multiple URLs, the configured credentials are sent to **every** entry in that list. This causes problems when you want a private upstream as the primary source and a public registry as the unauthenticated fallback: credentials forwarded to the public registry may produce `401 Unauthorized` rather than `404 Not Found`, and the fanout only advances to the next upstream on `404` — so a `401` stops the chain immediately.

The recommended pattern is:

1. A **private registry block** pointing at the authenticated upstream, with `upstream_auth` configured and anonymous reads enabled so BatleHub can reach it without a client token.
2. A **fanout registry block** that clients actually configure, whose `upstreams` list points at BatleHub's own proxy URL for the private registry first, then the public registry second.

BatleHub handles the credentials internally when it fetches from itself, so the fanout block never needs its own `upstream_auth`.

```toml
# Step 1 — private Gitea registry with credentials.
# anonymous source:read is required so the fanout block below can reach it
# without forwarding a client token.
[[registries]]
type      = "cargo"
name      = "internal-cargo"
upstreams = ["https://gitea.corp.example.com/api/packages/myorg/cargo"]
index_url = "https://gitea.corp.example.com/api/packages/myorg/cargo/index"

[registries.upstream_auth]
type  = "bearer"
token = "npat-xxxx"

[registries.rbac]
anonymous = ["source:read"]
user      = ["source:read"]
admin     = ["*"]

# Step 2 — fanout registry: private first (via BatleHub self-proxy), public fallback.
# Clients only configure this one.
[[registries]]
type      = "cargo"
name      = "cargo"
upstreams = [
  "http://localhost:8080/proxy/internal-cargo",  # BatleHub proxies with stored credentials
  "https://static.crates.io/crates",             # public fallback — no auth needed
]
index_url = "https://index.crates.io"

[registries.rbac]
anonymous = ["source:read"]
user      = ["source:read"]
admin     = ["*"]
```

Clients configure only the fanout registry:

```toml
# ~/.cargo/config.toml
[registries.cargo]
index = "sparse+https://batlehub.example.com/proxy/cargo/registry/"
```

When BatleHub resolves a crate through the `cargo` registry it first fetches `http://localhost:8080/proxy/internal-cargo/…`; that self-request is served by the `internal-cargo` registry which injects the Gitea bearer token on the way out. If the crate is not found (404), BatleHub falls through to `crates.io` without any credentials. The client never knows the private registry exists.

### Secret management

Credential values (`token`, `password`, `value`) are stored in the TOML config file. In production:
- Use a secrets manager (Vault, AWS Secrets Manager, Kubernetes Secrets) to inject values at runtime.
- Many deployment tools (Helm, Kustomize, systemd `EnvironmentFile`) support substituting environment variable references into config files before the process starts.

See [Worked Example 6.5](#65-self-hosted-private-registries) for a full multi-registry config.

---

## 11. SBOM Generation

BatleHub can automatically generate Software Bills of Materials (SBOMs) for every artifact it caches or hosts. SBOMs are produced in **SPDX 2.3** and **CycloneDX 1.4** formats and stored in the database alongside the artifact record.

Enable SBOM generation per registry with the `[registries.sbom]` block:

```toml
[[registries]]
type = "cargo"
name = "crates-io"

[registries.sbom]
enabled        = true
formats        = ["spdx", "cyclonedx"]   # default: both
fetch_upstream = true                    # try upstream APIs before extracting
required       = false                   # deny publish if no manifest found
```

### Options

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `enabled` | bool | `false` | Enable SBOM generation for this registry |
| `formats` | list | `["spdx", "cyclonedx"]` | Which formats to store. Either or both of `"spdx"`, `"cyclonedx"` |
| `fetch_upstream` | bool | `true` | Attempt to fetch a pre-built SBOM from the upstream before falling back to archive extraction or minimal generation |
| `required` | bool | `false` | Deny publish requests for local/hybrid registries when no dependency manifest can be extracted from the archive |

### SBOM source priority

For each artifact, BatleHub tries the following sources in order and uses the first one that succeeds:

1. **Upstream API** (when `fetch_upstream = true`) — GitHub dependency graph API for GitHub assets, npm `bom.json` for npm packages
2. **Archive extraction** — parse dependency manifests inside the downloaded archive: `Cargo.toml` (Cargo), `package.json` (npm), `pom.xml` (Maven), `go.mod` (Go), `requirements.txt` / `pyproject.toml` (PyPI)
3. **Minimal generation** — produce a document from package metadata (name, version, ecosystem PURL) with no dependency list

### API endpoints

| Endpoint | Auth | Description |
|----------|------|-------------|
| `GET /api/v1/sbom/{registry}/{name}/{version}?format=spdx\|cyclonedx` | Authenticated user | Retrieve the stored SBOM for one artifact version |
| `GET /api/v1/sbom/export?registry=…&from=…&to=…&format=spdx\|cyclonedx` | Admin | Export a merged SBOM covering all artifacts in a time range |

The export endpoint returns the document with `Content-Disposition: attachment` so browsers download it directly. The admin UI page at `/admin/sbom` provides a form for setting filters and downloading the export.

### Package URL (PURL) mapping

Each package in the generated SBOM is identified by a [PURL](https://github.com/package-url/purl-spec):

| Registry type | PURL scheme |
|---------------|-------------|
| `cargo` | `pkg:cargo/{name}@{version}` |
| `npm` | `pkg:npm/{name}@{version}` |
| `maven` | `pkg:maven/{group}/{artifact}@{version}` |
| `pypi` | `pkg:pypi/{name}@{version}` |
| `rubygems` | `pkg:gem/{name}@{version}` |
| `goproxy` | `pkg:golang/{name}@{version}` |
| `terraform` | `pkg:terraform/{name}@{version}` |
| `composer` | `pkg:composer/{name}@{version}` |
| `conda` | `pkg:conda/{name}@{version}` |
| everything else | `pkg:generic/{name}@{version}` |

### Worked example — Cargo proxy with SBOM

```toml
[[registries]]
type = "cargo"
name = "crates-io"

[registries.rbac]
anonymous = ["releases:read", "source:read"]

[registries.sbom]
enabled        = true
fetch_upstream = true   # try crates.io upstream SBOM first
```

Retrieve the SBOM for a specific crate:

```sh
curl -H "Authorization: Bearer $TOKEN" \
  "https://batlehub.example.com/api/v1/sbom/crates-io/serde/1.0.0?format=spdx" \
  | jq .

# Or CycloneDX:
curl -H "Authorization: Bearer $TOKEN" \
  "https://batlehub.example.com/api/v1/sbom/crates-io/serde/1.0.0?format=cyclonedx"
```

Export all SBOMs from the past 30 days as a single merged SPDX document:

```sh
FROM=$(date -u -d '30 days ago' +%Y-%m-%dT%H:%M:%SZ)
curl -H "Authorization: Bearer $ADMIN_TOKEN" \
  "https://batlehub.example.com/api/v1/sbom/export?from=${FROM}&format=spdx" \
  -o org-sbom.spdx.json
```

### Worked example — Private npm registry with required SBOM

```toml
[[registries]]
type = "npm"
name = "internal-npm"
mode = "local"

[registries.rbac]
user  = ["releases:read", "source:read"]
admin = ["*"]

[registries.sbom]
enabled  = true
required = true   # deny publish if package.json not found in the tarball
```

If a package tarball does not contain a `package.json`, `npm publish` will receive HTTP 422 and the error message `"no dependency manifest found"`.

For full SBOM API reference and tooling integration see [`docs/sbom.md`](sbom.md).

---

### 6.16 Corporate HTTP Proxy (air-gapped environments) {#616-corporate-http-proxy-air-gapped-environments}

Use this when BatleHub is deployed inside a network perimeter that requires all outbound HTTP/HTTPS traffic to route through a corporate proxy (e.g. Squid, Zscaler, Tinyproxy).

In this example, npm and Cargo packages are fetched through a Squid proxy that requires Basic authentication. A private internal Gitea npm registry is also configured — its traffic bypasses the proxy via `no_proxy` because it is reachable directly.

```toml
[server]
host = "0.0.0.0"
port = 8080

[database]
type = "postgresql"
url  = "postgresql://batlehub:changeme@db:5432/batlehub"

[[auth]]
type = "token"

[[auth.tokens]]
value   = "admin-token"
role    = "admin"
user_id = "admin"

[storage]
type = "filesystem"
path = "/data/cache"

# ── Public registries (routed through the corporate proxy) ────────────────────

[[registries]]
type = "npm"
name = "npm"

[registries.rbac]
anonymous = ["releases:read"]
user      = ["releases:read", "source:read"]
admin     = ["*"]

[registries.proxy]
url      = "http://squid.corp.example.com:3128"
username = "proxyuser"
password = "${PROXY_PASSWORD}"    # export PROXY_PASSWORD=s3cr3t

[[registries]]
type = "cargo"
name = "cargo"

[registries.rbac]
anonymous = ["source:read"]
user      = ["source:read"]
admin     = ["*"]

[registries.proxy]
url      = "http://squid.corp.example.com:3128"
username = "proxyuser"
password = "${PROXY_PASSWORD}"

# ── Internal Gitea registry (direct — bypasses the proxy) ────────────────────

[[registries]]
type      = "npm"
name      = "npm-internal"
upstreams = ["https://gitea.corp.example.com/api/packages/myorg/npm"]

[registries.upstream_auth]
type  = "bearer"
token = "${GITEA_TOKEN}"

[registries.proxy]
url      = "http://squid.corp.example.com:3128"
username = "proxyuser"
password = "${PROXY_PASSWORD}"
no_proxy = "gitea.corp.example.com"   # reach Gitea directly

[registries.rbac]
anonymous = []
user      = ["releases:read", "source:read"]
admin     = ["*"]
```

> **SOCKS5 proxy:** Replace `http://` with `socks5://` in the `url` field if your environment uses a SOCKS5 proxy (e.g. an SSH tunnel: `socks5://localhost:1080`).

> **Global proxy:** Instead of repeating `[registries.proxy]` on every registry, add a single `[proxy]` section at the top level — it applies to all registries at once. Per-registry `[registries.proxy]` blocks override the global value for that specific registry. The global proxy can also be set without touching the config file via `PROXY_CACHE__PROXY__URL` (and related env vars) — see [§3.8](#38-proxy-optional).

> **SOCKS5 proxy:** Replace `http://` with `socks5://` in the `url` field if your environment uses a SOCKS5 proxy (e.g. an SSH tunnel: `socks5://localhost:1080`).
