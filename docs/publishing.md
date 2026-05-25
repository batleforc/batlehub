# Publishing Packages to BatleHub

This guide walks through publishing packages to a BatleHub private registry for each supported registry type. Publishing requires the registry to be running in `local` or `hybrid` mode and a token with sufficient permissions.

---

## Table of Contents

1. [Prerequisites](#1-prerequisites)
2. [Getting an API token](#2-getting-an-api-token)
3. [npm](#3-npm)
4. [Cargo](#4-cargo)
5. [VS Code Extensions (OpenVSX / VS Code Marketplace)](#5-vs-code-extensions-openvsx--vs-code-marketplace)
6. [Go Modules](#6-go-modules)
7. [RubyGems](#7-rubygems)
8. [Maven](#8-maven)
9. [Terraform](#9-terraform)
10. [Troubleshooting](#10-troubleshooting)

---

## 1. Prerequisites

Publishing is only available when the registry is configured with `mode = "local"` or `mode = "hybrid"`. In `proxy` mode (the default), all write requests are rejected.

| Mode | Behaviour |
|------|-----------|
| `local` | BatleHub is the only source. No upstream needed. |
| `hybrid` | Local packages take priority; unknown packages fall back to upstream. |

See [`docs/configuration.md` § Registry modes](configuration.md#registry-modes) for the full configuration reference.

---

## 2. Getting an API token

All publish requests require a `Bearer` token in the `Authorization` header.

### Static tokens (config.toml)

The simplest option for CI pipelines or single-user setups:

```toml
[[auth]]
type = "token"

[[auth.tokens]]
value   = "my-publish-token"
role    = "admin"
user_id = "ci"
```

### User-generated API tokens (OIDC sessions)

If you use OIDC login, you can generate short-lived tokens from the Web UI (Settings → Tokens) or via the API:

```sh
curl -s -X POST \
  -H "Authorization: Bearer <oidc-session-token>" \
  -H "Content-Type: application/json" \
  -d '{"name": "ci-publish", "expires_in_days": 30, "role": "user"}' \
  https://batlehub.example.com/api/v1/auth/tokens
```

The response contains the raw token value — save it, it is shown only once.

```json
{
  "id": "...",
  "name": "ci-publish",
  "token": "bh_xxxxxxxxxxxxxxxxxxxx",
  "expires_at": "2026-06-21T00:00:00Z"
}
```

---

## 3. npm

### Server configuration

```toml
[[registries]]
type = "npm"
name = "internal-npm"
mode = "local"          # or "hybrid" to fall back to registry.npmjs.org

[registries.rbac]
anonymous = []
user      = ["source:read"]
admin     = ["*"]
```

For hybrid mode add `upstreams = ["https://registry.npmjs.org"]` under the registry block.

### Client setup

Create or update `.npmrc` (per-project or `~/.npmrc`):

```ini
# Scope all @myorg packages to the private registry
@myorg:registry=https://batlehub.example.com/proxy/internal-npm/

# Auth token for that registry host
//batlehub.example.com/proxy/internal-npm/:_authToken=<your-token>
```

To use the registry for all packages (unscoped), set the global registry:

```ini
registry=https://batlehub.example.com/proxy/internal-npm/
//batlehub.example.com/proxy/internal-npm/:_authToken=<your-token>
```

### Publish

```sh
npm publish --registry https://batlehub.example.com/proxy/internal-npm/
# or, with .npmrc configured:
npm publish
```

### Verify

```sh
npm view @myorg/my-package --registry https://batlehub.example.com/proxy/internal-npm/
npm install @myorg/my-package
```

### Endpoint reference

| Method | Path | Description |
|--------|------|-------------|
| `PUT` | `/proxy/{registry}/{package}` | `npm publish` |
| `GET` | `/proxy/{registry}/{package}` | Packument (all versions) |
| `GET` | `/proxy/{registry}/{package}/{version}/tarball` | Tarball download |

---

## 4. Cargo

### Server configuration

```toml
[[registries]]
type = "cargo"
name = "internal"
mode = "local"          # or "hybrid" to fall back to crates.io

[registries.rbac]
anonymous = []
user      = ["source:read"]
admin     = ["*"]
```

For hybrid mode add:
```toml
upstreams = ["https://static.crates.io/crates"]
index_url = "https://index.crates.io"
```

### Client setup

Edit `~/.cargo/config.toml` or `.cargo/config.toml` in the project root:

```toml
[registries.internal]
index = "sparse+https://batlehub.example.com/proxy/internal/registry/"
token = "<your-token>"
```

Alternatively export the token as an environment variable (useful in CI):

```sh
export CARGO_REGISTRIES_INTERNAL_TOKEN=<your-token>
```

### Publish

```sh
cargo publish --registry internal
```

Cargo serialises crate metadata + the `.crate` archive into a single binary payload and sends it to `PUT /proxy/internal/api/v1/crates/new`. The checksum is verified server-side.

### Depend on a privately published crate

```toml
# Cargo.toml
[dependencies]
my-lib = { version = "0.1", registry = "internal" }
```

### Yank / unyank a version

```sh
cargo yank --registry internal my-lib@0.1.0
cargo yank --undo --registry internal my-lib@0.1.0
```

### Verify

```sh
cargo add my-lib --registry internal
```

### Endpoint reference

| Method | Path | Description |
|--------|------|-------------|
| `PUT` | `/proxy/{registry}/api/v1/crates/new` | `cargo publish` |
| `DELETE` | `/proxy/{registry}/api/v1/crates/{name}/{version}/yank` | `cargo yank` |
| `PUT` | `/proxy/{registry}/api/v1/crates/{name}/{version}/unyank` | `cargo yank --undo` |
| `GET` | `/proxy/{registry}/registry/config.json` | Sparse index config |
| `GET` | `/proxy/{registry}/registry/{path}` | Sparse index entries |
| `GET` | `/proxy/{registry}/{name}/{version}/download` | `.crate` download |

---

## 5. VS Code Extensions (OpenVSX / VS Code Marketplace)

Both registry types (`openvsx` and `vscode-marketplace`) use the same upload endpoint. There is no dedicated CLI tool — extensions are published with a plain `PUT` request carrying the raw VSIX bytes.

### Server configuration

```toml
[[registries]]
type = "openvsx"        # or "vscode-marketplace"
name = "internal-ext"
mode = "local"

[registries.rbac]
anonymous = []
user      = ["source:read"]
admin     = ["*"]
```

### Extension ID convention

Extension IDs follow the `{publisher}.{name}` format used by the VS Code Marketplace, e.g. `my-org.my-extension`.

### Upload

```sh
curl -X PUT \
  -H "Authorization: Bearer <your-token>" \
  -H "Content-Type: application/octet-stream" \
  --data-binary @my-org.my-extension-1.0.0.vsix \
  "https://batlehub.example.com/proxy/internal-ext/my-org.my-extension/1.0.0/vsix"
```

The server reads the publisher and extension name from the URL path. The `{extension_id}` segment is the full `{publisher}.{name}` identifier.

### Download / install

```sh
# Download the VSIX
curl -H "Authorization: Bearer <your-token>" \
  "https://batlehub.example.com/proxy/internal-ext/my-org.my-extension/1.0.0/vsix" \
  -o my-org.my-extension-1.0.0.vsix

# Install into VS Code
code --install-extension my-org.my-extension-1.0.0.vsix
```

### Verify

```sh
# Confirm the ZIP magic bytes (PK\x03\x04) to validate the upload was accepted
curl -s -H "Authorization: Bearer <your-token>" \
  "https://batlehub.example.com/proxy/internal-ext/my-org.my-extension/1.0.0/vsix" \
  | xxd | head -1
# Should show: 50 4b 03 04 ...
```

### Endpoint reference

| Method | Path | Description |
|--------|------|-------------|
| `PUT` | `/proxy/{registry}/{extension_id}/{version}/vsix` | Upload VSIX |
| `GET` | `/proxy/{registry}/{extension_id}/{version}/vsix` | Download VSIX |

---

## 6. Go Modules

Go modules are published by uploading a module zip archive. BatleHub extracts `go.mod` from the zip and generates version metadata automatically — there is no separate metadata upload step.

### Server configuration

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

For hybrid mode add `upstreams = ["https://proxy.golang.org"]`.

### Build the module zip

Use the standard `go mod zip` command from the module's source directory:

```sh
# From the root of your module (where go.mod lives)
go mod zip example.com/mymod@v1.0.0 . --mod-zip /tmp/mymod-v1.0.0.zip
```

The zip must contain every file under a single top-level directory named `{module}@{version}/` (e.g. `example.com/mymod@v1.0.0/`). `go mod zip` produces this layout automatically. If you build the zip manually, all entry paths must use this prefix.

### Upload

```sh
curl -X PUT \
  -H "Authorization: Bearer <your-token>" \
  -H "Content-Type: application/zip" \
  --data-binary @/tmp/mymod-v1.0.0.zip \
  "https://batlehub.example.com/proxy/internal-go/example.com/mymod/@v/v1.0.0.zip"
```

Module paths may contain slashes — the URL pattern captures everything before `/@v/` as the module path.

### Configure the go toolchain

```sh
export GONOSUMCHECK="*"
export GONOSUMDB="*"
export GOPROXY="https://batlehub.example.com/proxy/internal-go,direct"
```

Or save permanently with `go env -w`:

```sh
go env -w GONOSUMCHECK="*"
go env -w GONOSUMDB="*"
go env -w GOPROXY="https://batlehub.example.com/proxy/internal-go,direct"
```

`GONOSUMCHECK` and `GONOSUMDB` disable the checksum database for private modules. The `,direct` fallback tells the go tool to reach the internet directly if the proxy returns a 404 — remove it if BatleHub should be the only source.

### Verify

```sh
go get example.com/mymod@v1.0.0
```

### Endpoint reference

| Method | Path | Description |
|--------|------|-------------|
| `PUT` | `/proxy/{registry}/{module}/@v/{version}.zip` | Upload module zip |
| `GET` | `/proxy/{registry}/{module}/@latest` | Latest version info JSON |
| `GET` | `/proxy/{registry}/{module}/@v/list` | All version list |
| `GET` | `/proxy/{registry}/{module}/@v/{version}.info` | Version metadata JSON |
| `GET` | `/proxy/{registry}/{module}/@v/{version}.mod` | `go.mod` content |
| `GET` | `/proxy/{registry}/{module}/@v/{version}.zip` | Module source zip |

---

## 7. RubyGems

### Server configuration

```toml
[[registries]]
type = "rubygems"
name = "internal-gems"
mode = "local"          # or "hybrid" to fall back to rubygems.org

[registries.rbac]
anonymous = []
user      = ["source:read"]
admin     = ["*"]
```

For hybrid mode add `upstreams = ["https://rubygems.org"]`.

### Client setup

Add to `~/.gem/credentials` (create if absent, `chmod 600` after):

```yaml
---
:internal-gems: <your-token>
```

Or pass the token directly on the command line.

### Publish

```sh
# gem push reads credentials from ~/.gem/credentials
gem push my-gem-1.0.0.gem --host https://batlehub.example.com/proxy/internal-gems

# Or pass the token directly
gem push my-gem-1.0.0.gem --host https://batlehub.example.com/proxy/internal-gems \
  --key <your-token>
```

### Install

```sh
gem install my-gem --source https://batlehub.example.com/proxy/internal-gems
```

Or in a `Gemfile`:

```ruby
source "https://batlehub.example.com/proxy/internal-gems" do
  gem "my-gem"
end
```

### Yank / unyank

```sh
# Yank
curl -X DELETE \
  -H "Authorization: Bearer <your-token>" \
  "https://batlehub.example.com/proxy/internal-gems/api/v1/gems/yank?gem_name=my-gem&version=1.0.0"

# Unyank
curl -X PUT \
  -H "Authorization: Bearer <your-token>" \
  "https://batlehub.example.com/proxy/internal-gems/api/v1/gems/unyank?gem_name=my-gem&version=1.0.0"
```

### Endpoint reference

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/proxy/{registry}/api/v1/gems` | `gem push` |
| `DELETE` | `/proxy/{registry}/api/v1/gems/yank` | Yank version |
| `PUT` | `/proxy/{registry}/api/v1/gems/unyank` | Unyank version |
| `GET` | `/proxy/{registry}/gems/{name}-{version}.gem` | Download gem |
| `GET` | `/proxy/{registry}/api/v1/gems/{name}.json` | Gem info |
| `GET` | `/proxy/{registry}/api/v1/versions/{name}.json` | All versions |

---

## 8. Maven

Maven artifacts are published by uploading individual files (`PUT`) using the Maven 2 repository layout. When the `.pom` file is uploaded, BatleHub parses it and creates a version record — subsequent GET requests will include it in `maven-metadata.xml`.

### Server configuration

```toml
[[registries]]
type = "maven"
name = "internal-maven"
mode = "local"          # or "hybrid" to fall back to repo1.maven.org

[registries.rbac]
anonymous = []
user      = ["source:read"]
admin     = ["*"]
```

For hybrid mode add `upstreams = ["https://repo1.maven.org/maven2"]`.

### Client setup — Maven (`~/.m2/settings.xml`)

```xml
<settings>
  <servers>
    <server>
      <id>internal-maven</id>
      <username>token</username>
      <password>YOUR_TOKEN</password>
    </server>
  </servers>

  <!-- Optional: use as a download mirror for all artifacts -->
  <mirrors>
    <mirror>
      <id>internal-maven</id>
      <mirrorOf>*</mirrorOf>
      <url>https://batlehub.example.com/proxy/internal-maven/maven2</url>
    </mirror>
  </mirrors>
</settings>
```

### Client setup — Gradle (`build.gradle.kts`)

```kotlin
repositories {
    maven {
        name = "internalMaven"
        url  = uri("https://batlehub.example.com/proxy/internal-maven/maven2")
        credentials {
            username = "token"
            password = System.getenv("BATLEHUB_TOKEN") ?: ""
        }
    }
}
```

### Publish — Maven

Add to your project's `pom.xml`:

```xml
<distributionManagement>
  <repository>
    <id>internal-maven</id>
    <url>https://batlehub.example.com/proxy/internal-maven/maven2</url>
  </repository>
  <snapshotRepository>
    <id>internal-maven</id>
    <url>https://batlehub.example.com/proxy/internal-maven/maven2</url>
  </snapshotRepository>
</distributionManagement>
```

Then deploy:

```sh
mvn deploy
# or, overriding the repository URL without editing pom.xml:
mvn deploy -DaltDeploymentRepository=internal-maven::default::https://batlehub.example.com/proxy/internal-maven/maven2
```

Maven uploads the `.jar`, `-sources.jar`, `.pom`, and checksum files individually. BatleHub accepts all of them and records the version when the `.pom` arrives.

### Publish — Gradle

Add to `build.gradle.kts`:

```kotlin
publishing {
    repositories {
        maven {
            name = "internalMaven"
            url  = uri("https://batlehub.example.com/proxy/internal-maven/maven2")
            credentials {
                username = "token"
                password = System.getenv("BATLEHUB_TOKEN") ?: ""
            }
        }
    }
}
```

Then publish:

```sh
./gradlew publish
```

### Verify

```sh
# Download maven-metadata.xml (should list the published version)
curl -H "Authorization: Bearer <your-token>" \
  "https://batlehub.example.com/proxy/internal-maven/maven2/com/example/mylib/maven-metadata.xml"

# Resolve the artifact (Maven)
mvn dependency:get -Dartifact=com.example:mylib:1.0.0
```

### Endpoint reference

| Method | Path | Description |
|--------|------|-------------|
| `PUT` | `/proxy/{registry}/maven2/{group}/{artifact}/{version}/{file}` | Upload artifact (`.pom` triggers version record) |
| `GET` | `/proxy/{registry}/maven2/{group}/{artifact}/maven-metadata.xml` | Generated version list XML |
| `GET` | `/proxy/{registry}/maven2/{group}/{artifact}/{version}/{file}` | Download artifact |

`{group}` uses path segments: `com/example` maps to groupId `com.example`.

---

## 9. Terraform

BatleHub supports both **provider** and **module** private registries. Modules use a simple tarball upload. Providers follow a two-step process: upload a version manifest (JSON describing platforms and checksums), then upload each platform binary.

### Server configuration

```toml
[[registries]]
type = "terraform"
name = "internal-tf"
mode = "local"          # or "hybrid" to fall back to registry.terraform.io

[registries.rbac]
anonymous = []
user      = ["source:read"]
admin     = ["*"]
```

For hybrid mode add `upstreams = ["https://registry.terraform.io"]`.

### Publishing modules

A Terraform module is a `.tar.gz` archive of the module directory.

```sh
# Build the archive
tar -czf consul-aws-0.1.0.tar.gz -C /path/to/module .

# Upload
curl -X POST \
  -H "Authorization: Bearer <your-token>" \
  -H "Content-Type: application/gzip" \
  --data-binary @consul-aws-0.1.0.tar.gz \
  "https://batlehub.example.com/proxy/internal-tf/v1/modules/hashicorp/consul/aws/0.1.0"
```

### Using a private module

Add credentials to `~/.terraformrc`:

```hcl
credentials "batlehub.example.com" {
  token = "<your-token>"
}
```

Reference the module in Terraform:

```hcl
module "consul" {
  source  = "batlehub.example.com/proxy/internal-tf/hashicorp/consul/aws"
  version = "0.1.0"
}
```

### Publishing providers

**Step 1 — Upload version manifest** (JSON describing the version and its platforms):

```sh
curl -X POST \
  -H "Authorization: Bearer <your-token>" \
  -H "Content-Type: application/json" \
  -d '{
    "version": "1.0.0",
    "protocols": ["5.0"],
    "platforms": [
      {
        "os": "linux", "arch": "amd64",
        "filename": "terraform-provider-mycloud_1.0.0_linux_amd64.zip",
        "shasum": "<sha256-hex>"
      }
    ]
  }' \
  "https://batlehub.example.com/proxy/internal-tf/v1/providers/myorg/mycloud/versions"
```

**Step 2 — Upload platform binaries**:

```sh
curl -X PUT \
  -H "Authorization: Bearer <your-token>" \
  -H "Content-Type: application/zip" \
  --data-binary @terraform-provider-mycloud_1.0.0_linux_amd64.zip \
  "https://batlehub.example.com/proxy/internal-tf/v1/providers/myorg/mycloud/1.0.0/artifact/linux/amd64"
```

Repeat the binary upload for each supported platform.

### Using a private provider

```hcl
# ~/.terraformrc
credentials "batlehub.example.com" {
  token = "<your-token>"
}
```

```hcl
# main.tf
terraform {
  required_providers {
    mycloud = {
      source  = "batlehub.example.com/proxy/internal-tf/myorg/mycloud"
      version = "~> 1.0"
    }
  }
}
```

### Yank a version (admin)

Use the admin bulk-operations API (see [Administration guide](../website/guide/administration.md)):

```sh
curl -X POST \
  -H "Authorization: Bearer <admin-token>" \
  -H "Content-Type: application/json" \
  -d '{"packages": [{"name": "modules/hashicorp/consul/aws", "versions": ["0.1.0"]}]}' \
  "https://batlehub.example.com/api/v1/admin/registries/internal-tf/bulk-yank"
```

### Endpoint reference

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/proxy/{registry}/v1/modules/{ns}/{name}/{provider}/{version}` | Upload module tarball |
| `GET` | `/proxy/{registry}/v1/modules/{ns}/{name}/{provider}/{version}/artifact` | Download module tarball |
| `GET` | `/proxy/{registry}/v1/modules/{ns}/{name}/{provider}/versions` | List module versions |
| `GET` | `/proxy/{registry}/v1/modules/{ns}/{name}/{provider}/{version}/download` | Download redirect (`X-Terraform-Get`) |
| `POST` | `/proxy/{registry}/v1/providers/{ns}/{type}/versions` | Upload provider manifest |
| `PUT` | `/proxy/{registry}/v1/providers/{ns}/{type}/{version}/artifact/{os}/{arch}` | Upload platform binary |
| `GET` | `/proxy/{registry}/v1/providers/{ns}/{type}/{version}/artifact/{os}/{arch}` | Download platform binary |
| `GET` | `/proxy/{registry}/v1/providers/{ns}/{type}/versions` | List provider versions |
| `GET` | `/proxy/{registry}/v1/providers/{ns}/{type}/{version}/download/{os}/{arch}` | Provider download info JSON |

---

## 10. Troubleshooting

### `403 Forbidden` on publish

- The token is missing, expired, or does not have the required role. Publish is restricted to `admin` role by default. Check the `[registries.rbac]` block — the role that should publish needs `"*"` (or at minimum write access).
- Pass the token explicitly: `-H "Authorization: Bearer <token>"`.

### `403 Forbidden` — "registry is not in local or hybrid mode"

The registry `mode` is set to `proxy` (the default). Change it to `"local"` or `"hybrid"` in `config.toml` and restart the server.

### `409 Conflict`

The version already exists in the registry. Bump the version in your package manifest and republish.

### `400 Bad Request` (Go)

The module zip structure is invalid. Every entry inside the zip must be prefixed with `{module}@{version}/`. Rebuild with `go mod zip` to get the correct layout.

### `400 Bad Request` (Cargo)

Cargo uses a binary wire format (length-prefixed metadata JSON followed by the `.crate` bytes). Only `cargo publish` produces this format — do not attempt to hand-craft the request.

### Token accepted but `cargo publish` fails with "invalid token"

Cargo expects the sparse index `config.json` to match the token endpoint. Verify the `index` URL in `.cargo/config.toml` ends with `/registry/`:

```
sparse+https://batlehub.example.com/proxy/internal/registry/
```

### `400 Bad Request` (Maven) — "POM missing groupId"

The uploaded `.pom` file is missing `<groupId>` or `<artifactId>`. These are required fields. Check that your `pom.xml` or Gradle `build.gradle.kts` sets `group` and `archivesName`/`rootProject.name` before publishing.

### `mvn deploy` succeeds but `maven-metadata.xml` is not updated

BatleHub generates `maven-metadata.xml` dynamically from the database. A successful `.pom` upload (HTTP 201) means the version was recorded. If GET returns 404, the `.pom` upload may have failed — check the response status for each uploaded file in verbose output (`mvn deploy -X`).

### Terraform `terraform init` fails — "registry does not have a provider"

Verify the `source` address in `terraform required_providers` matches the registry hostname and path exactly:
```
batlehub.example.com/proxy/{registry}/namespace/type
```
Ensure credentials for `batlehub.example.com` are set in `~/.terraformrc`.

### Terraform provider download fails — "no matching binary"

The provider manifest was uploaded without a binary for the requested platform. Upload the binary via:
```
PUT /proxy/{registry}/v1/providers/{ns}/{type}/{version}/artifact/{os}/{arch}
```
