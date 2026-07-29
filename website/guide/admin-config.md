# Server configuration

## Configuration {#configuration}

BatleHub reads a single TOML file, defaulting to `config.toml` in the working directory. Override the path with `--config /path/to/config.toml`.

### Loading order

1. TOML file is read from disk.
2. `${VAR_NAME}` placeholders inside string values are replaced with their environment variable values.
3. The resulting TOML is parsed.
4. Named `PROXY_CACHE__*` environment variable overrides are applied on top.
5. Registry names and types are validated.

### Secret injection with `${VAR_NAME}` {#env-inline}

Write `${VAR_NAME}` inside any TOML string value. BatleHub replaces the placeholder with the named environment variable before parsing. This works for **every field** — auth secrets, upstream tokens, passwords, and more.

::: danger Missing variable = startup failure
If a referenced variable is not set, BatleHub exits immediately with a clear error message naming the missing variable. There is no silent fallback or empty-string default.
:::

**OIDC client secret:**

```toml
[[auth]]
type          = "oidc"
issuer_url    = "https://sso.example.com/application/o/batlehub/"
client_id     = "batlehub"
client_secret = "${OIDC_CLIENT_SECRET}"   # export OIDC_CLIENT_SECRET=...
redirect_uri  = "https://hub.example.com/api/v1/auth/oidc/callback"
```

**Upstream registry credentials:**

```toml
# Bearer token (GitHub PAT, Gitea token, npm auth token)
[registries.upstream_auth]
type  = "bearer"
token = "${REGISTRY_TOKEN}"

# Basic auth (Nexus, Artifactory)
[registries.upstream_auth]
type     = "basic"
username = "deploy"
password = "${REGISTRY_PASSWORD}"

# Custom header (X-API-Key, etc.)
[registries.upstream_auth]
type  = "header"
name  = "X-API-Key"
value = "${REGISTRY_API_KEY}"
```

**Kubernetes / Docker Compose injection:**

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

To write a literal `${...}` string (no variable lookup), escape the first `$`:

```toml
# Stores the literal string "${MY_VAR}" — no substitution performed:
some_field = "$${MY_VAR}"
```

### Named environment variable overrides {#env-named}

A fixed set of top-level fields can also be overridden with named env vars. Useful for tweaking infrastructure addresses (host, port, DB URL) in containerised deployments without modifying the config file.

| Variable | Config field |
|----------|-------------|
| `PROXY_CACHE__SERVER__PORT` | `server.port` |
| `PROXY_CACHE__SERVER__HOST` | `server.host` |
| `PROXY_CACHE__SERVER__STATIC_DIR` | `server.static_dir` |
| `PROXY_CACHE__DATABASE__URL` | `database.url` |
| `PROXY_CACHE__DATABASE__MAX_CONNECTIONS` | `database.max_connections` |
| `PROXY_CACHE__STORAGE__PATH` | `storage.path` (single filesystem backend) |
| `PROXY_CACHE__STORAGE__BUCKET` | `storage.bucket` (single S3 backend) |
| `PROXY_CACHE__STORAGE__REGION` | `storage.region` (single S3 backend) |
| `PROXY_CACHE__STORAGE__ENDPOINT_URL` | `storage.endpoint_url` (single S3 backend) |
| `PROXY_CACHE__OTEL__ENDPOINT` | `otel.endpoint` |
| `PROXY_CACHE__OTEL__SERVICE_NAME` | `otel.service_name` |

::: tip When to use which
Use **`${VAR_NAME}` placeholders** for secrets (auth tokens, passwords, client secrets) — they work for any field and keep credentials out of the TOML file entirely.

Use **`PROXY_CACHE__*` variables** for infrastructure addresses (database URL, storage path, host/port) where the value is not secret but varies between environments.
:::

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
type          = "oidc"
issuer_url    = "https://sso.example.com/application/o/batlehub/"
client_id     = "batlehub"
client_secret = "${OIDC_CLIENT_SECRET}"   # inject from env — never commit secrets
redirect_uri  = "https://batlehub.example.com/api/v1/auth/oidc/callback"
scopes        = ["openid", "profile", "email", "groups"]

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

## Hot reload {#hot-reload}

BatleHub can reload its configuration at runtime — add or remove registries, update RBAC rules, or change policy settings — without restarting the process. In-flight requests finish with the old configuration before the new one takes effect.

### How it works

1. When `config.toml` changes on disk, the built-in file watcher validates the new config, runs connectivity probes against upstream URLs, and stores a **pending reload** in memory.
2. An administrator reviews the pending diff in the **Config Reload** admin page (`/admin/config-reload`) and clicks **Apply** — or discards it.
3. Alternatively, the `POST /api/v1/admin/config/reload` endpoint applies a reload immediately (load + validate + apply atomically), which is useful in CI/CD pipelines.

```sh
# Immediate reload (no confirmation step)
curl -s -X POST \
  -H "Authorization: Bearer <admin-token>" \
  http://localhost:8080/api/v1/admin/config/reload

# Check for a pending reload loaded by the file watcher
curl -s -H "Authorization: Bearer <admin-token>" \
  http://localhost:8080/api/v1/admin/config/pending

# Apply the pending reload
curl -s -X POST \
  -H "Authorization: Bearer <admin-token>" \
  http://localhost:8080/api/v1/admin/config/pending/apply

# Discard without applying
curl -s -X DELETE \
  -H "Authorization: Bearer <admin-token>" \
  http://localhost:8080/api/v1/admin/config/pending
```

Pending reloads expire after **10 minutes** if not applied or discarded.

### What can be hot-reloaded

| Component | Hot-reloadable |
|-----------|---------------|
| Registry list (add / remove / update) | ✅ |
| Per-registry RBAC (`anonymous`, `user`, `admin`, groups) | ✅ |
| Per-registry rules (age gate, deny latest) | ✅ |
| Per-registry versioning / signing / beta-channel | ✅ |
| Artifact size limit | ✅ |
| Server host / port | ❌ requires restart |
| Database URL | ❌ requires restart |
| Auth providers | ❌ requires restart |
| Storage backends | ❌ requires restart |

### Audit trail

Every reload (applied or rejected) is written to the `config_changes` table and visible in the admin page change history:

```sh
curl -s -H "Authorization: Bearer <admin-token>" \
  "http://localhost:8080/api/v1/admin/config/changes?per_page=20"
```

### Disabling hot reload

Set `BATLEHUB_DISABLE_HOT_RELOAD=1` in the server environment to disable the file watcher and all reload endpoints with a `503 Service Unavailable`. This is recommended when `config.toml` is mounted as a read-only Kubernetes ConfigMap, where the file will not change at runtime.

```yaml
# Kubernetes Deployment env
- name: BATLEHUB_DISABLE_HOT_RELOAD
  value: "1"
```

---

## Global banner {#global-banner}

Administrators can broadcast a short message to all website visitors — authenticated or not. Common uses: maintenance windows, reload-in-progress notices, and policy announcements.

The banner is automatically set to "Configuration reload in progress…" when a hot reload starts and cleared when it completes.

### Set the banner

From the **Config Reload** admin page, fill in the message and select a level (info / warning / error), then click **Set Banner**.

```sh
# Set via API
curl -s -X PUT \
  -H "Authorization: Bearer <admin-token>" \
  -H "Content-Type: application/json" \
  -d '{"message":"Scheduled maintenance in 30 min","level":"warning"}' \
  http://localhost:8080/api/v1/admin/banner

# Clear
curl -s -X DELETE \
  -H "Authorization: Bearer <admin-token>" \
  http://localhost:8080/api/v1/admin/banner
```

The frontend polls `GET /api/v1/banner` every 30 seconds (no authentication required) and displays the banner as a dismissible bar at the top of every page.

### High-availability banner propagation

The banner backend is selected from the same pool as the metadata cache:

| `[cache] type` | Banner storage |
|----------------|---------------|
| `"memory"` (default) | In-process — not shared across replicas |
| `"redis"` | Redis key `batlehub:system:banner` — shared across all replicas |
| `"postgres"` | `system_kv` table — shared across all replicas |

In an HA deployment, use `"redis"` or `"postgres"` so that all replicas show the same banner regardless of which instance the client reaches.
