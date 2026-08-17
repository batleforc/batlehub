# Worked examples

## 6.1 Local Development

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

## 6.2 Production with OIDC (Authentik)

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

## 6.3 Kubernetes Deployment

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

## 6.4 Go Module Proxy

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

## 6.5 Self-Hosted Private Registries

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

## 6.6 Private Cargo Registry (local / hybrid mode) {#66-private-cargo-registry-local--hybrid-mode}

> For a step-by-step publishing walkthrough, see [`docs/use/publishing.md § Cargo`](/registries/cargo#publishing-local-hybrid).

### Pure local registry (no upstream)

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

### Hybrid registry (local crates + crates.io fallback)

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

### Endpoints exposed by local / hybrid registries

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

## 6.7 Private npm Registry (local / hybrid mode) {#67-private-npm-registry-local--hybrid-mode}

> For a step-by-step publishing walkthrough, see [`docs/use/publishing.md § npm`](/registries/npm#publishing-local-hybrid).

### Pure local npm registry (no upstream)

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

### Hybrid npm registry (local packages + upstream fallback)

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

### Endpoints exposed by local / hybrid npm registries

| Method | Path | Used by |
|--------|------|---------|
| `GET` | `/proxy/{registry}/{package}` | packument (all versions) |
| `GET` | `/proxy/{registry}/{package}/{version}` | single version metadata |
| `GET` | `/proxy/{registry}/{package}/{version}/tarball` | tarball download |
| `PUT` | `/proxy/{registry}/{package}` | `npm publish` |
| `POST` | `/proxy/{registry}/-/npm/v1/audit/quick` | `npm audit` (proxied upstream) |

---

## 6.8 Private VS Code Extension Registry (local / hybrid mode) {#68-private-vs-code-extension-registry-local--hybrid-mode}

> For a step-by-step publishing walkthrough, see [`docs/use/publishing.md § VS Code Extensions`](/registries/openvsx#publishing-local-hybrid).

Use this when you want to distribute private VS Code extensions through a self-hosted registry.

### Pure local extension registry

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

Point the editor at the registry's gallery, in `product.json`:

```jsonc
{
  "extensionsGallery": {
    "serviceUrl": "https://batlehub.example.com/proxy/internal-ext/vscode/gallery",
    "itemUrl": "https://batlehub.example.com/proxy/internal-ext/vscode/item",
    "resourceUrlTemplate": "https://batlehub.example.com/proxy/internal-ext/vscode/unpkg/{publisher}/{name}/{version}/{path}"
  }
}
```

The editor sends no credentials to its gallery, so the `anonymous` grant above
is what makes this work — see [OpenVSX](/registries/openvsx#use-batlehub-as-your-extension-gallery).

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

### Endpoints exposed by local / hybrid VS Code extension registries

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/proxy/{registry}/{extension_id}/{version}/vsix` | Download VSIX |
| `PUT` | `/proxy/{registry}/{extension_id}/{version}/vsix` | Upload VSIX |

Extension IDs follow the `{publisher}.{name}` convention (e.g. `my-org.my-ext`).

---

## 6.9 Private Go Module Proxy (local / hybrid mode) {#69-private-go-module-proxy-local--hybrid-mode}

> For a step-by-step publishing walkthrough, see [`docs/use/publishing.md § Go Modules`](/registries/goproxy#publishing-local-hybrid).

### Pure local Go module proxy (no upstream)

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

### Hybrid Go module proxy (local modules + upstream fallback)

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

### Endpoints exposed by local / hybrid Go module proxies

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

## 6.10 Multi-Backend Storage {#610-multi-backend-storage}

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

## 6.11 Terraform Provider Cache {#611-terraform-provider-cache}

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

## 6.12 Private Maven Registry (local / hybrid mode) {#612-private-maven-registry-local--hybrid-mode}

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

### Client setup — Maven

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

### Publish setup — pom.xml

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

### Publish setup — Gradle (settings.gradle.kts)

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

### How it works

Maven/Gradle upload `.jar` and checksum files **before** the `.pom`. BatleHub stores each non-POM file directly in object storage. When the `.pom` arrives, BatleHub parses it (extracting `groupId`, `artifactId`, `version`, `packaging`, `description`) and commits a `local_packages` row via the three-phase publish protocol. Subsequent `GET` requests for `maven-metadata.xml` return XML generated from the database rather than a cached file.

### Endpoints exposed by local / hybrid Maven registries

| Endpoint | Method | Description |
|---|---|---|
| `/proxy/{registry}/maven2/{path}` | GET | Serve artifact from local storage (or proxy in hybrid mode) |
| `/proxy/{registry}/maven2/{group}/{artifact}/maven-metadata.xml` | GET | Generated from DB; never cached |
| `/proxy/{registry}/maven2/{path}` | PUT | Upload artifact (`.pom` commits version, other files stored directly) |

---

## 6.13 Private Terraform Registry (local / hybrid mode) {#613-private-terraform-registry-local--hybrid-mode}

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

### Client setup — .terraformrc

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

### Publishing a private module

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

### Publishing a private provider

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

### Yank a version (admin)

```sh
curl -X POST \
  -H "Authorization: Bearer <admin-token>" \
  -H "Content-Type: application/json" \
  -d '{"packages":[{"name":"modules/namespace/name/provider","version":"1.0.0"}]}' \
  "https://batlehub.example.com/api/v1/admin/registries/internal-tf/bulk-yank"
```

---

## 6.14 Rate Limiting — Per-User + Per-Group {#614-rate-limiting}

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

## 6.15 Private Composer Registry (local / hybrid mode) {#615-private-composer-registry-local--hybrid-mode}

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

### Client setup — composer.json

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

### Publishing a package

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

### Yanking a version

```sh
curl -X DELETE \
  -H "Authorization: Bearer <token>" \
  "https://batlehub.example.com/proxy/internal-composer/api/packages/my-vendor/my-package/versions/1.0.0"
```

Yanked versions are hidden from `p2/` metadata and return 404 on download.

---

## 6.16 Corporate HTTP Proxy (air-gapped environments) {#616-corporate-http-proxy-air-gapped-environments}

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

> **Global proxy:** Instead of repeating `[registries.proxy]` on every registry, add a single `[proxy]` section at the top level — it applies to all registries at once. Per-registry `[registries.proxy]` blocks override the global value for that specific registry. The global proxy can also be set without touching the config file via `PROXY_CACHE__PROXY__URL` (and related env vars) — see [§3.8](/guide/configuration#_3-8-proxy-optional).

> **SOCKS5 proxy:** Replace `http://` with `socks5://` in the `url` field if your environment uses a SOCKS5 proxy (e.g. an SSH tunnel: `socks5://localhost:1080`).



Every example here is a complete `config.toml`. The field-by-field
reference they draw on is [Configuration](/guide/configuration).
