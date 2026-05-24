# BatleHub - Proxy Cache

PREV session : claude --resume 14b43612-5297-4250-8b77-4b05b6f706b5

A self-hosted smart proxy and cache for package registries. It sits between your build tools and the internet, caches artifacts after the first download, and enforces access-control rules before any package reaches a developer or CI pipeline.

## Supported registries

| Registry | Protocol | Default upstream |
|----------|----------|-----------------|
| **GitHub** | Releases, assets, tarballs, raw files | `api.github.com` |
| **npm** | Full packument + tarball proxy | `registry.npmjs.org` |
| **Cargo** | Sparse index + `.crate` download | `crates.io` / `index.crates.io` |
| **OpenVSX** | VS Code extension VSIX download | `open-vsx.org` |
| **VS Code Marketplace** | VS Code extension VSIX download via Microsoft Gallery API | `marketplace.visualstudio.com` |
| **Go** | GOPROXY protocol (`.info`, `.mod`, `.zip`, `@latest`, `@v/list`) | `proxy.golang.org` |

Multiple instances of the same registry type can run in parallel (e.g. a private npm registry and the public one as fallback).

### Feature matrix

| Feature | GitHub | npm | Cargo | OpenVSX | VS Code Marketplace | Go |
|---------|:------:|:---:|:-----:|:-------:|:-------------------:|:--:|
| Version listing | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| Latest version resolution | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| Version metadata | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| Source archive download | ✓ | ✓ | ✓ | — | — | ✓ |
| Binary / extension download | ✓ | — | — | ✓ | ✓ | — |
| Raw file access | ✓ | — | — | — | — | — |
| Sparse index proxy | — | — | ✓ | — | — | — |
| Module definition file | — | — | — | — | — | ✓ |
| Publish timestamp | ⚠ ² | ✓ | ✓ | ✓ | ✓ | ✓ |
| Signed release detection | — | — | — | ✓ | — | — |
| Release age gate rule | ⚠ ² | ✓ | ✓ | ✓ | ✓ | ✓ |
| Deny latest tag rule | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| Multi-upstream fanout | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| **Private publish** (`mode = local/hybrid`) | — | ✓ ³ | ✓ ³ | ✓ ³ | ✓ ³ | ✓ ³ |

> ² **GitHub**: publish timestamp (and therefore the age gate) is only populated for specific-tag release requests. Raw file, source tarball, and release-listing requests return no timestamp and the rule is skipped.
>
> ³ **Private publish**: set `mode = "local"` to use BatleHub as the authoritative registry (no upstream needed), or `mode = "hybrid"` to serve locally published packages first and fall through to an upstream for everything else. See [Private registries](#private-registries-local--hybrid-mode) below.

## Key features

- **Artifact caching** — first download is fetched from upstream and stored; subsequent requests are served from local or S3 storage.
- **Private / local registry** — `npm`, `cargo`, `openvsx`, `vscode-marketplace`, and `goproxy` registries can be set to `mode = "local"` (fully private, no upstream) or `mode = "hybrid"` (local-first with upstream fallback). Teams publish packages directly to BatleHub using standard tools (`npm publish`, `cargo publish`, raw VSIX upload, Go module zip upload).
- **RBAC** — per-registry permissions for `anonymous`, `user`, and `admin` roles, plus group-based access from OIDC or Kubernetes claims.
- **Release age gate** — block packages published less than N seconds ago (supply-chain delay window).
- **Deny latest tag** — reject requests that use `"latest"` as a version, forcing consumers to pin exact versions. Configurable bypass roles (e.g. admins may still use `latest`).
- **Fanout / failover** — list multiple upstreams per registry; 404 from one falls through to the next.
- **Self-hosted registry support** — upstream auth (Bearer token, Basic, or custom header) and custom CA certificates per registry, for air-gapped or corporate environments.
- **Auth providers** — static tokens, OIDC (Authentik, Keycloak, Dex, …), Kubernetes service account tokens.
- **Storage backends** — filesystem or S3-compatible (AWS S3, MinIO, RustFS). Different registries can use different backends.
- **Audit log** — every allow and deny decision is recorded in PostgreSQL.
- **OpenTelemetry** — optional distributed tracing via OTLP/gRPC.
- **Web UI** — a Vue 3 SPA for browsing packages, managing blocks, and generating client config snippets.
- **OpenAPI** — full Swagger UI at `/swagger-ui/` and spec dump via `batlehub dump-spec`.

---

## Quick start

### With Docker Compose

```sh
# Clone and start PostgreSQL + the server
git clone https://github.com/your-org/batlehub
cd batlehub
cp config.example.toml config.toml   # edit as needed
podman compose up -d                 # or docker compose up -d
```

The server listens on `http://localhost:8080`. The admin token from `config.example.toml` is `change-me-admin-token`.

### Build from source

**Prerequisites:** Rust 1.87+, Node 24+, PostgreSQL

```sh
# Backend
cargo build --release -p batlehub-server

# Frontend (optional — embeds the SPA into the server)
cd ui && npm ci && npm run build && cd ..

# Generate the OpenAPI spec and TypeScript client
cargo run -p batlehub-server -- --config config.example.toml dump-spec > ui/openapi.json
cd ui && npm run generate && npm run build && cd ..

# Run
./target/release/batlehub --config config.toml
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
url  = "postgresql://batlehub:changeme@localhost:5432/batlehub"

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

### Self-hosted / private registry example

Bearer token and custom CA for a corporate Gitea instance:

```toml
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
user  = ["releases:read", "source:read"]
admin = ["*"]
```

Three auth schemes are supported: `bearer`, `basic`, and `header` (custom header such as `X-API-Key`). See [docs/configuration.md § Self-Hosted / Private Registries](docs/configuration.md#9-self-hosted--private-registries) for the full reference.

```sh
export GONOSUMCHECK="*"
export GONOSUMDB="*"
export GOPROXY="http://localhost:8080/proxy/go,direct"
go get golang.org/x/text@latest
```

---

## Private registries (local / hybrid mode)

`npm`, `cargo`, `openvsx`, and `vscode-marketplace` registries can act as authoritative private registries — not just caches. Set the `mode` field on any registry entry:

| Mode | Behaviour |
|------|-----------|
| `proxy` | Default. Forwards to upstream; publishing is rejected. |
| `local` | BatleHub is the only source. No upstream needed. Clients publish directly to BatleHub. |
| `hybrid` | Local-first. Serves locally published packages; falls back to upstream for anything else. |

### Cargo (private crate registry)

```toml
[[registries]]
type = "cargo"
name = "internal"
mode = "local"

[registries.rbac]
user  = ["source:read"]
admin = ["*"]
```

```toml
# ~/.cargo/config.toml
[registries.internal]
index = "sparse+https://batlehub.example.com/proxy/internal/registry/"
token = "<your-token>"
```

```sh
cargo publish --registry internal
```

### npm (private package registry)

```toml
[[registries]]
type = "npm"
name = "internal-npm"
mode = "local"

[registries.rbac]
user  = ["source:read"]
admin = ["*"]
```

```ini
# .npmrc
@myorg:registry=https://batlehub.example.com/proxy/internal-npm/
//batlehub.example.com/proxy/internal-npm/:_authToken=<your-token>
```

```sh
npm publish --registry https://batlehub.example.com/proxy/internal-npm/
```

### VS Code extensions (private VSIX registry)

```toml
[[registries]]
type = "openvsx"     # or "vscode-marketplace"
name = "internal-ext"
mode = "local"

[registries.rbac]
user  = ["source:read"]
admin = ["*"]
```

```sh
# Upload
curl -X PUT -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/octet-stream" \
  --data-binary @my-org.my-ext-1.0.0.vsix \
  "https://batlehub.example.com/proxy/internal-ext/my-org.my-ext/1.0.0/vsix"

# Download
curl -H "Authorization: Bearer <token>" \
  "https://batlehub.example.com/proxy/internal-ext/my-org.my-ext/1.0.0/vsix" \
  -o my-org.my-ext-1.0.0.vsix
```

### Go (private module proxy)

```toml
[[registries]]
type = "goproxy"
name = "internal-go"
mode = "local"

[registries.rbac]
user  = ["source:read"]
admin = ["*"]
```

Upload a module (PUT the zip archive — `go.mod` is extracted automatically):

```sh
curl -X PUT -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/zip" \
  --data-binary @path/to/module-v1.0.0.zip \
  "https://batlehub.example.com/proxy/internal-go/example.com/mymod/@v/v1.0.0.zip"
```

Point the go tool at the private proxy:

```sh
export GONOSUMCHECK="*"
export GONOSUMDB="*"
export GOPROXY="https://batlehub.example.com/proxy/internal-go,direct"
go get example.com/mymod@v1.0.0
```

See [`docs/configuration.md § Registry modes`](docs/configuration.md#registry-modes) for the full reference including hybrid mode and client-side setup.

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
replace-with = "batlehub"

[source.batlehub]
registry = "sparse+http://localhost:8080/proxy/cargo/registry/"
```

### Go

```sh
export GOPROXY="http://localhost:8080/proxy/go,direct"
export GONOSUMCHECK="*"
export GONOSUMDB="*"
```

### VS Code Marketplace

```sh
# Download and install an extension via the proxy
curl -sL "http://localhost:8080/proxy/vscode/ms-python.python/latest/vsix" \
  -o extension.vsix && code --install-extension extension.vsix

# Pin a specific version
curl -H "Authorization: Bearer <token>" \
  "http://localhost:8080/proxy/vscode/ms-python.python/2024.2.1/vsix" \
  -o ms-python.python-2024.2.1.vsix
```

The proxy URL pattern is `/proxy/{registry}/{publisher}.{name}/{version}/vsix`.

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
  └─ [[registries]]  type = "npm" | "cargo" | "github" | "openvsx" | "vscode-marketplace" | "goproxy"
         │
         ▼
server/src/main.rs         — builds registry clients, policies, services
         │
         ▼
ProxyService               — orchestrates caching, rules, streaming
  ├── resolve_metadata()   → registry adapter (fetches version info from upstream)
  ├── evaluate rules       → RBAC, block list, release age gate, deny latest
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

### Fuzzing

The `fuzz/` directory contains libFuzzer targets for the most security-sensitive code paths:

| Target | What it covers |
|--------|---------------|
| `fuzz_rbac_evaluate` | RBAC group-string parsing (colon-based provider prefix splitting) |
| `fuzz_package_id_cache_key` | `PackageId::cache_key()` with arbitrary registry/name/version strings |
| `fuzz_deny_latest` | Version-string comparison — verifies only exact `"latest"` is blocked |
| `fuzz_release_age` | Chrono timestamp arithmetic with arbitrary past/future dates |

Requires a nightly Rust toolchain and `cargo-fuzz`:

```sh
rustup install nightly
cargo install cargo-fuzz

# Run a specific target (30 s by default)
task fuzz TARGET=fuzz_deny_latest

# Run longer or with a different target
task fuzz TARGET=fuzz_rbac_evaluate MAX_TIME=300
```

Corpus and crash inputs are saved under `fuzz/corpus/<target>/` and `fuzz/artifacts/<target>/` respectively.

### Adding a new registry type

See [`docs/adding-a-registry.md`](docs/adding-a-registry.md) for a step-by-step guide with code templates.

---

## Deployment

### Docker image

```sh
docker build -t batlehub .
docker run -p 8080:8080 \
  -v /path/to/config.toml:/etc/batlehub/config.toml \
  -v /path/to/cache:/var/cache/batlehub \
  batlehub
```

### Helm chart

A Helm chart is available in `helm/batlehub/` for Kubernetes deployments:

```sh
helm install batlehub ./helm/batlehub \
  --namespace batlehub --create-namespace \
  --set database.url="postgresql://batlehub:changeme@postgres:5432/batlehub" \
  --set "auth.tokens[0].value=my-admin-token" \
  --set "auth.tokens[0].role=admin"
```

See [`website/guide/installation.md`](website/guide/installation.md) for the full Helm reference including values, S3 storage, and GitOps patterns.

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
| [`website/`](website/) | VitePress documentation site — run `task website:dev` to browse locally |
| [`website/guide/installation.md`](website/guide/installation.md) | Installation guide: Docker Compose, binary, Helm chart |
| [`website/guide/administration.md`](website/guide/administration.md) | Administration: config, auth, S3, health, package management |
| [`website/guide/user.md`](website/guide/user.md) | User guide: client setup and publishing for all registry types |
| [`docs/configuration.md`](docs/configuration.md) | Full TOML reference, permissions, worked examples |
| [`docs/configuration.md § Registry modes`](docs/configuration.md#registry-modes) | Private registry modes (local / hybrid) for Cargo, npm, and VS Code extensions |
| [`docs/configuration.md § Self-Hosted`](docs/configuration.md#9-self-hosted--private-registries) | Upstream auth (Bearer / Basic / header) and custom CA certificates |
| [`docs/publishing.md`](docs/publishing.md) | Step-by-step guide for publishing packages (npm, Cargo, VSIX, Go modules) |
| [`docs/adding-a-registry.md`](docs/adding-a-registry.md) | Step-by-step guide for implementing a new registry adapter |
| `/swagger-ui/` (runtime) | Interactive API docs |

## Roadmap

See [`ROADMAP.md`](ROADMAP.md) for the full list of planned features, or browse the [Roadmap page on the documentation site](website/guide/roadmap.md).
