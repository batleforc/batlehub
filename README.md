# proxy-cache

A self-hosted smart proxy and cache for package registries. It sits between your build tools and the internet, caches artifacts after the first download, and enforces access-control rules before any package reaches a developer or CI pipeline.

## Supported registries

| Registry | Protocol | Default upstream |
|----------|----------|-----------------|
| **GitHub** | Releases, assets, tarballs, raw files | `api.github.com` |
| **npm** | Full packument + tarball proxy | `registry.npmjs.org` |
| **Cargo** | Sparse index + `.crate` download | `crates.io` / `index.crates.io` |
| **OpenVSX** | VS Code extension VSIX download | `open-vsx.org` |
| **Go** | GOPROXY protocol (`.info`, `.mod`, `.zip`, `@latest`, `@v/list`) | `proxy.golang.org` |

Multiple instances of the same registry type can run in parallel (e.g. a private npm registry and the public one as fallback).

### Feature matrix

| Feature | GitHub | npm | Cargo | OpenVSX | Go |
|---------|:------:|:---:|:-----:|:-------:|:--:|
| Version listing | ✓ | ✓ | ✓ | ✓ | ✓ |
| Latest version resolution | ✓ | ✓ | ✓ | ✓ | ✓ |
| Version metadata | ✓ | ✓ | ✓ | ✓ | ✓ |
| Source archive download | ✓ | ✓ | ✓ | — | ✓ |
| Binary / extension download | ✓ | — | — | ✓ | — |
| Raw file access | ✓ | — | — | — | — |
| Sparse index proxy | — | — | ✓ | — | — |
| Module definition file | — | — | — | — | ✓ |
| Publish timestamp | ⚠ ² | ✓ | ✓ | ✓ | ✓ |
| Signed release detection | — | — | — | ✓ | — |
| Release age gate rule | ⚠ ² | ✓ | ✓ | ✓ | ✓ |
| Multi-upstream fanout | ✓ | ✓ | ✓ | ✓ | ✓ |

> ² **GitHub**: publish timestamp (and therefore the age gate) is only populated for specific-tag release requests. Raw file, source tarball, and release-listing requests return no timestamp and the rule is skipped.

## Key features

- **Artifact caching** — first download is fetched from upstream and stored; subsequent requests are served from local or S3 storage.
- **RBAC** — per-registry permissions for `anonymous`, `user`, and `admin` roles, plus group-based access from OIDC or Kubernetes claims.
- **Release age gate** — block packages published less than N seconds ago (supply-chain delay window).
- **Fanout / failover** — list multiple upstreams per registry; 404 from one falls through to the next.
- **Auth providers** — static tokens, OIDC (Authentik, Keycloak, Dex, …), Kubernetes service account tokens.
- **Storage backends** — filesystem or S3-compatible (AWS S3, MinIO, RustFS). Different registries can use different backends.
- **Audit log** — every allow and deny decision is recorded in PostgreSQL.
- **OpenTelemetry** — optional distributed tracing via OTLP/gRPC.
- **Web UI** — a Vue 3 SPA for browsing packages, managing blocks, and generating client config snippets.
- **OpenAPI** — full Swagger UI at `/swagger-ui/` and spec dump via `proxy-cache dump-spec`.

---

## Quick start

### With Docker Compose

```sh
# Clone and start PostgreSQL + the server
git clone https://github.com/your-org/proxy-cache
cd proxy-cache
cp config.example.toml config.toml   # edit as needed
podman compose up -d                 # or docker compose up -d
```

The server listens on `http://localhost:8080`. The admin token from `config.example.toml` is `change-me-admin-token`.

### Build from source

**Prerequisites:** Rust 1.87+, Node 24+, PostgreSQL

```sh
# Backend
cargo build --release -p proxy-cache-server

# Frontend (optional — embeds the SPA into the server)
cd ui && npm ci && npm run build && cd ..

# Generate the OpenAPI spec and TypeScript client
cargo run -p proxy-cache-server -- --config config.example.toml dump-spec > ui/openapi.json
cd ui && npm run generate && npm run build && cd ..

# Run
./target/release/proxy-cache --config config.toml
```

Or use the [Task](https://taskfile.dev) shortcuts:

```sh
task compose:db    # start only postgres
task run           # cargo run with example config
task ui:dev        # vite dev server (proxies /api and /proxy to :8080)
task test          # cargo test --workspace
```

---

## Configuration at a glance

The server is configured with a single TOML file (`config.toml` by default, override with `--config`). See [`docs/configuration.md`](docs/configuration.md) for the full reference and worked examples.

### Minimal example

```toml
[server]
port = 8080

[database]
type = "postgresql"
url  = "postgresql://proxy_cache:changeme@localhost:5432/proxy_cache"

[[auth]]
type = "token"

[[auth.tokens]]
value = "my-admin-token"
role  = "admin"

[storage]
type = "filesystem"
path = "./cache"

[[registries]]
type = "npm"
name = "npm"

[registries.rbac]
anonymous = ["releases:read", "source:read"]
```

### Go module proxy example

```toml
[[registries]]
type = "goproxy"
name = "go"
# upstreams defaults to ["https://proxy.golang.org"]

[registries.rbac]
anonymous = ["releases:read", "source:read"]
```

Then point the go toolchain at the proxy:

```sh
export GONOSUMCHECK="*"
export GONOSUMDB="*"
export GOPROXY="http://localhost:8080/proxy/go,direct"
go get golang.org/x/text@latest
```

---

## Client tool configuration

The built-in **Setup Guide** page (`/setup`) generates ready-to-paste config snippets for all supported tools. The snippets below are illustrative; the UI generates them pre-filled with your server's actual address.

### npm / yarn / pnpm

```sh
# .npmrc
registry=http://localhost:8080/proxy/npm/
```

### Cargo

```toml
# .cargo/config.toml
[source.crates-io]
replace-with = "proxy-cache"

[source.proxy-cache]
registry = "sparse+http://localhost:8080/proxy/cargo/registry/"
```

### Go

```sh
export GOPROXY="http://localhost:8080/proxy/go,direct"
export GONOSUMCHECK="*"
export GONOSUMDB="*"
```

### GitHub (mise)

```toml
# ~/.config/mise/config.toml
[settings.url_replacements]
"regex:^https://api\\.github\\.com/repos/(.+)" = "http://localhost:8080/proxy/github/$1"
"regex:^https://github\\.com/([^/]+)/([^/]+)/releases/download/([^/]+)/(.+)" = "http://localhost:8080/proxy/github/$1/$2/releases/download/$3/$4"
```

---

## Architecture

```
config.toml
  └─ [[registries]]  type = "npm" | "cargo" | "github" | "openvsx" | "goproxy"
         │
         ▼
server/src/main.rs         — builds registry clients, policies, services
         │
         ▼
ProxyService               — orchestrates caching, rules, streaming
  ├── resolve_metadata()   → registry adapter (fetches version info from upstream)
  ├── evaluate rules       → RBAC, block list, release age gate
  ├── storage cache        → filesystem or S3
  └── fetch_artifact()     → registry adapter (streams bytes from upstream)
         │
         ▼
HTTP handlers (actix-web)  — one module per registry type
```

### Crate structure

| Crate | Purpose |
|-------|---------|
| `crates/core` | Domain entities, ports (traits), rules, `ProxyService`, `AdminService` |
| `crates/adapters` | Registry clients, auth providers, storage backends, database layer |
| `crates/config` | TOML schema and validation |
| `crates/web` | actix-web handlers, middleware, OpenAPI definitions |
| `server` | Binary entry point — wires everything together |
| `ui` | Vue 3 + Tailwind SPA (package browser, setup guide, admin panel) |

---

## Permissions

| Permission | Meaning |
|-----------|---------|
| `releases:read` | List versions and download release assets / metadata |
| `source:read` | Download source archives (tarballs, `.crate`, `.zip`) |
| `*` | All permissions |

Role inheritance: `admin` ⊃ `user` ⊃ `anonymous`. Group permissions from OIDC or Kubernetes claims are additive on top of role permissions.

---

## Development

```sh
task build        # cargo build --workspace
task test         # cargo test --workspace
task lint         # cargo clippy --workspace
task fmt          # cargo fmt --all
task dump-spec    # regenerate ui/openapi.json
task ui:generate  # regenerate TypeScript client from openapi.json
```

### Adding a new registry type

See [`docs/adding-a-registry.md`](docs/adding-a-registry.md) for a step-by-step guide with code templates.

---

## Deployment

### Docker image

```sh
docker build -t proxy-cache .
docker run -p 8080:8080 \
  -v /path/to/config.toml:/etc/proxy-cache/config.toml \
  -v /path/to/cache:/var/cache/proxy-cache \
  proxy-cache
```

The image uses a multi-stage build (Rust builder → Node UI builder → Debian slim runtime). The compiled binary and built SPA are copied into the final stage.

### Environment variable overrides

Key settings can be overridden at runtime without editing the config file:

| Variable | Config field |
|----------|-------------|
| `PROXY_CACHE__SERVER__PORT` | `server.port` |
| `PROXY_CACHE__DATABASE__URL` | `database.url` |
| `PROXY_CACHE__STORAGE__PATH` | `storage.path` (single filesystem backend) |
| `PROXY_CACHE__OTEL__ENDPOINT` | `otel.endpoint` |

Full list in [`docs/configuration.md § Environment Variable Overrides`](docs/configuration.md#5-environment-variable-overrides).

---

## Documentation

| Document | Contents |
|----------|---------|
| [`docs/configuration.md`](docs/configuration.md) | Full TOML reference, permissions, worked examples |
| [`docs/adding-a-registry.md`](docs/adding-a-registry.md) | Step-by-step guide for implementing a new registry adapter |
| `/swagger-ui/` (runtime) | Interactive API docs |
