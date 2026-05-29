# Installation

BatleHub is a single binary backed by PostgreSQL. Choose the installation method that fits your environment.

[[toc]]

---

## Prerequisites

All installation methods require a **PostgreSQL 14+** database. The server creates its schema automatically on first start.

---

## Pre-built releases

Every tagged release publishes ready-to-use artifacts to GitHub:

### Container image (recommended for production)

A multi-arch image (`linux/amd64` + `linux/arm64`) is pushed to the GitHub Container Registry:

```sh
docker pull ghcr.io/batleforc/batlehub:<version>

# Or always pull the latest tagged version (not :latest — pin to a specific version in production)
docker pull ghcr.io/batleforc/batlehub:1.0.0
```

Run it:

```sh
docker run -p 8080:8080 \
  -v /path/to/config.toml:/etc/batlehub/config.toml:ro \
  -v /path/to/cache:/var/cache/batlehub \
  ghcr.io/batleforc/batlehub:<version>
```

### Pre-built binary

A statically linked `batlehub` binary for Linux is attached to each [GitHub Release](https://github.com/batleforc/batlehub/releases). Download it, make it executable, and run:

```sh
curl -L -o batlehub https://github.com/batleforc/batlehub/releases/download/<version>/batlehub
chmod +x batlehub
./batlehub --config config.toml
```

---

## Docker Compose (quickest path)

The fastest way to get a running instance for local development or evaluation.

**1. Clone the repository:**

```sh
git clone https://github.com/batleforc/batlehub
cd batlehub
```

**2. Copy and edit the example config:**

```sh
cp config.example.toml config.toml
# Edit config.toml: set database URL, admin token, and at least one registry
```

**3. Start PostgreSQL and the server:**

```sh
podman compose up -d   # or docker compose up -d
```

The server listens on `http://localhost:8080`. The Swagger UI is at `http://localhost:8080/swagger-ui/`.

**4. Verify:**

```sh
curl http://localhost:8080/api/openapi.json
```

### With S3 storage (RustFS)

A separate Compose file adds a RustFS (S3-compatible) storage backend and Authentik OIDC:

```sh
podman compose -f docker-compose.s3.yml up -d postgres rustfs
# Then run the server with the S3 config:
task run:s3
```

---

## Binary from source

**Prerequisites:** Rust 1.87+, Node 24+, PostgreSQL

**1. Build the backend:**

```sh
cargo build --release -p batlehub-server
```

**2. Build the frontend SPA (optional — embeds the UI into the server):**

```sh
cd ui
npm ci
npm run build
cd ..
```

**3. Generate the OpenAPI spec and TypeScript client (required if building the UI):**

```sh
cargo run -p batlehub-server -- --config config.example.toml dump-spec > ui/openapi.json
cd ui && npm run generate && npm run build && cd ..
```

**4. Create a config file and run:**

```sh
cp config.example.toml config.toml
./target/release/batlehub --config config.toml
```

### Task shortcuts

If you have [Task](https://taskfile.dev) installed:

```sh
task compose:db    # start only PostgreSQL
task run           # cargo run with example config
task ui:dev        # Vite dev server, proxies /api and /proxy to :8080
task dev           # backend + frontend together
task test          # cargo test --workspace
```

---

## Helm chart

Deploy BatleHub on Kubernetes using the bundled Helm chart.

**Prerequisites:** Helm 3+, a running Kubernetes cluster, PostgreSQL accessible from the cluster.

### Quick install

```sh
# Clone the repo (chart is bundled in helm/batlehub/)
git clone https://github.com/batleforc/batlehub
cd batlehub

helm install batlehub ./helm/batlehub \
  --namespace batlehub \
  --create-namespace \
  --set database.url="postgresql://batlehub:changeme@postgres-svc:5432/batlehub" \
  --set "auth.tokens[0].value=my-admin-token" \
  --set "auth.tokens[0].role=admin" \
  --set "auth.tokens[0].userId=admin"
```

### Recommended: values file

Create a `my-values.yaml` for a reproducible installation:

```yaml
database:
  url: "postgresql://batlehub:changeme@postgres-svc:5432/batlehub"

auth:
  tokens:
    - value: "my-admin-token"
      role: admin
      userId: admin

registriesRaw: |
  [[registries]]
  type = "npm"
  name = "npm"

  [registries.rbac]
  anonymous = ["releases:read", "source:read"]
  user      = ["releases:read", "source:read"]
  admin     = ["*"]

  [[registries]]
  type = "cargo"
  name = "internal"
  mode = "local"

  [registries.rbac]
  user  = ["source:read"]
  admin = ["*"]

ingress:
  enabled: true
  className: nginx
  host: batlehub.example.com
  tls:
    - secretName: batlehub-tls
      hosts:
        - batlehub.example.com

persistence:
  enabled: true
  size: 50Gi
```

```sh
helm install batlehub ./helm/batlehub \
  --namespace batlehub \
  --create-namespace \
  -f my-values.yaml
```

### Upgrade

```sh
helm upgrade batlehub ./helm/batlehub \
  --namespace batlehub \
  -f my-values.yaml
```

Any change to the values that affects the rendered `config.toml` will automatically trigger a Pod rollout via the `checksum/secret` annotation on the Deployment.

### S3 storage

```yaml
storage:
  type: s3
  s3:
    bucket: batlehub-artifacts
    region: us-east-1
    accessKeyId: "AKIAIOSFODNN7EXAMPLE"
    secretAccessKey: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"

persistence:
  enabled: false   # PVC not needed with S3
```

### Key values reference

| Key | Default | Description |
|-----|---------|-------------|
| `image.repository` | `ghcr.io/batleforc/batlehub` | Container image |
| `image.tag` | Chart appVersion | Image tag |
| `replicaCount` | `1` | Pod replicas |
| `database.url` | — | PostgreSQL connection string |
| `storage.type` | `filesystem` | `filesystem` or `s3` |
| `auth.tokens` | `[]` | Static token list |
| `auth.oidc` | `[]` | OIDC provider list |
| `registriesRaw` | npm example | Raw TOML `[[registries]]` blocks |
| `ingress.enabled` | `false` | Create an Ingress resource |
| `persistence.enabled` | `true` | Create a PVC for cache |
| `persistence.size` | `10Gi` | PVC capacity |
| `existingSecret` | `""` | Use a pre-existing Secret for config |

### Using an external secret (GitOps / Sealed Secrets)

If you manage secrets externally (Sealed Secrets, External Secrets Operator, Vault), create the Secret yourself:

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: batlehub-config
  namespace: batlehub
type: Opaque
stringData:
  config.toml: |
    [server]
    host = "0.0.0.0"
    port = 8080

    [database]
    type = "postgresql"
    url  = "postgresql://..."

    [[auth]]
    type = "token"

    [[auth.tokens]]
    value = "my-token"
    role  = "admin"
    user_id = "admin"

    [[registries]]
    type = "npm"
    name = "npm"

    [registries.rbac]
    anonymous = ["releases:read", "source:read"]
```

Then install the chart with `existingSecret`:

```sh
helm install batlehub ./helm/batlehub \
  --namespace batlehub \
  --set existingSecret=batlehub-config
```

---

## First-time setup

Regardless of installation method, once the server is running:

**1. Verify the health endpoint:**

```sh
curl -H "Authorization: Bearer my-admin-token" \
  http://localhost:8080/api/v1/admin/health
```

**2. Open the Web UI and Setup Guide:**

Navigate to `http://localhost:8080` — the Setup Guide page (`/setup`) generates client config snippets for all registered tools.

**3. Point a client at the proxy:**

```sh
# npm
npm install --registry http://localhost:8080/proxy/npm/ some-package

# Go
GOPROXY=http://localhost:8080/proxy/go,direct go get golang.org/x/text@latest

# Cargo — add to .cargo/config.toml
# [source.crates-io]
# replace-with = "batlehub"
# [source.batlehub]
# registry = "sparse+http://localhost:8080/proxy/cargo/registry/"
```
