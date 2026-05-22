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
7. [Troubleshooting](#7-troubleshooting)

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

## 7. Troubleshooting

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
