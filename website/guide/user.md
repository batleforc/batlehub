# User Guide

This guide covers how to set up your local development environment to use BatleHub as a registry proxy, and how to publish private packages when your administrator has enabled `local` or `hybrid` mode.

[[toc]]

---

## Getting a token {#getting-a-token}

Most BatleHub endpoints require a Bearer token. Ask your administrator for a token or, if OIDC login is enabled, generate one yourself:

**Via the Web UI:** log in at `https://batlehub.example.com`, open Settings → Tokens, and click "New token".

**Via the API:**

```sh
# Exchange your OIDC session token for a long-lived API token
curl -X POST \
  -H "Authorization: Bearer <oidc-session-token>" \
  -H "Content-Type: application/json" \
  -d '{"name": "my-laptop", "expires_in_days": 90, "role": "user"}' \
  https://batlehub.example.com/api/v1/auth/tokens
```

The raw token value is shown **once** — save it to a password manager or environment variable.

```sh
export BATLEHUB_TOKEN=bh_xxxxxxxxxxxxxxxxxxxx
```

### Authenticating from GitHub / Forgejo Actions {#ci-actions-oidc}

If your administrator has configured an `actions-oidc` auth provider, GitHub and Forgejo workflow jobs can authenticate **without any long-lived secret**. The workflow requests a short-lived OIDC token from the runner and passes it directly as a Bearer token.

Enable OIDC token minting in your workflow:

```yaml
jobs:
  publish:
    permissions:
      id-token: write   # required — lets the runner mint an OIDC token
      contents: read
```

Then exchange the token at the start of any step that calls BatleHub:

```sh
# In a GitHub Actions "run:" step:
BATLEHUB_TOKEN=$(curl -s -H "Authorization: bearer $ACTIONS_ID_TOKEN_REQUEST_TOKEN" \
  "${ACTIONS_ID_TOKEN_REQUEST_URL}&audience=batlehub" | jq -r '.value')

# Use it exactly like any other Bearer token
curl -H "Authorization: Bearer $BATLEHUB_TOKEN" \
  https://batlehub.example.com/api/v1/...
```

The token is valid for the duration of the job. It carries claims like `repository`, `ref`, `environment`, and `actor`, which the `actions-oidc` provider uses to assign you to one or more groups — for example `"github-actions/myorg-my-repo/main"` — so you automatically receive the right RBAC permissions without any manual user management.

Ask your administrator which groups are mapped and what permissions they carry.

---

## Setup Guide UI

The built-in **Setup Guide** at `https://batlehub.example.com/setup` generates ready-to-paste config snippets for every registered tool. The snippets are pre-filled with your server's address and available registries — use them as a starting point for the manual steps below.

---

## npm {#npm}

### Point npm at BatleHub

```ini
# .npmrc (project root or ~/.npmrc)
registry=https://batlehub.example.com/proxy/npm/
//batlehub.example.com/proxy/npm/:_authToken=${BATLEHUB_TOKEN}
```

For scoped packages only:

```ini
@myorg:registry=https://batlehub.example.com/proxy/npm/
//batlehub.example.com/proxy/npm/:_authToken=${BATLEHUB_TOKEN}
```

### Install packages

```sh
npm install lodash
npm install @myorg/my-private-package
```

### Publish a private package (local/hybrid mode)

The registry must be in `local` or `hybrid` mode — ask your administrator.

```sh
npm publish --registry https://batlehub.example.com/proxy/internal-npm/
```

Or, with `.npmrc` configured for `internal-npm`:

```sh
npm publish
```

### Verify

```sh
npm view lodash --registry https://batlehub.example.com/proxy/npm/
```

---

## Cargo {#cargo}

### Point Cargo at BatleHub (proxy mode)

Replace the default crates.io source so all `cargo add` / `cargo build` requests go through BatleHub:

```toml
# .cargo/config.toml
[source.crates-io]
replace-with = "batlehub"

[source.batlehub]
registry = "sparse+https://batlehub.example.com/proxy/cargo/registry/"
```

### Private registry (local/hybrid mode)

Configure an additional named registry for private crates:

```toml
# .cargo/config.toml
[registries.internal]
index = "sparse+https://batlehub.example.com/proxy/internal/registry/"
token = "<your-token>"
```

Or export the token as an environment variable (useful in CI):

```sh
export CARGO_REGISTRIES_INTERNAL_TOKEN=$BATLEHUB_TOKEN
```

### Publish a crate

```sh
cargo publish --registry internal
```

### Depend on a privately published crate

```toml
# Cargo.toml
[dependencies]
my-lib = { version = "0.1", registry = "internal" }
```

### Yank / restore a version

```sh
cargo yank --registry internal my-lib@0.1.0
cargo yank --undo --registry internal my-lib@0.1.0
```

### Verify

```sh
cargo add serde              # via proxy (replaces crates-io)
cargo add my-lib --registry internal   # private registry
```

---

## Go Modules {#go-modules}

### Point the go toolchain at BatleHub

```sh
export GONOSUMCHECK="*"
export GONOSUMDB="*"
export GOPROXY="https://batlehub.example.com/proxy/go,direct"
```

To make this permanent:

```sh
go env -w GONOSUMCHECK="*"
go env -w GONOSUMDB="*"
go env -w GOPROXY="https://batlehub.example.com/proxy/go,direct"
```

`GONOSUMCHECK` / `GONOSUMDB` disable the public checksum database — required for private modules. The `,direct` fallback lets the go tool reach the internet if BatleHub returns a 404.

### Fetch a module

```sh
go get golang.org/x/text@v0.3.7
```

### Publish a private module (local/hybrid mode)

**1. Build the module zip** (standard Go module zip format):

```sh
# From the root of your module (where go.mod lives)
go mod zip example.com/mymod@v1.0.0 . --mod-zip /tmp/mymod-v1.0.0.zip
```

**2. Upload:**

```sh
curl -X PUT \
  -H "Authorization: Bearer $BATLEHUB_TOKEN" \
  -H "Content-Type: application/zip" \
  --data-binary @/tmp/mymod-v1.0.0.zip \
  "https://batlehub.example.com/proxy/internal-go/example.com/mymod/@v/v1.0.0.zip"
```

The module path may contain slashes (e.g. `example.com/org/mymod`). BatleHub extracts `go.mod` from the zip and generates version metadata automatically.

**3. Point the toolchain at the private proxy:**

```sh
export GOPROXY="https://batlehub.example.com/proxy/internal-go,direct"
go get example.com/mymod@v1.0.0
```

### Zip format requirements

All entries inside the zip must be prefixed with `{module}@{version}/`. The `go mod zip` command produces this layout automatically. If you build the zip manually, ensure every file path starts with `example.com/mymod@v1.0.0/`.

---

## VS Code Extensions {#vs-code-extensions}

### Point VS Code at BatleHub (OpenVSX)

Add to `.vscode/settings.json` or user settings:

```json
{
  "vscode-extension-marketplace.serviceUrl": "https://batlehub.example.com/proxy/openvsx"
}
```

### Download and install an extension

```sh
curl -H "Authorization: Bearer $BATLEHUB_TOKEN" \
  "https://batlehub.example.com/proxy/vscode/ms-python.python/2024.2.1/vsix" \
  -o ms-python.python-2024.2.1.vsix

code --install-extension ms-python.python-2024.2.1.vsix
```

### Publish a private extension (local mode)

Both `openvsx` and `vscode-marketplace` registry types support the same upload endpoint. Extension IDs follow the `{publisher}.{name}` convention.

```sh
curl -X PUT \
  -H "Authorization: Bearer $BATLEHUB_TOKEN" \
  -H "Content-Type: application/octet-stream" \
  --data-binary @my-org.my-extension-1.0.0.vsix \
  "https://batlehub.example.com/proxy/internal-ext/my-org.my-extension/1.0.0/vsix"
```

### Download a private extension

```sh
curl -H "Authorization: Bearer $BATLEHUB_TOKEN" \
  "https://batlehub.example.com/proxy/internal-ext/my-org.my-extension/1.0.0/vsix" \
  -o my-org.my-extension-1.0.0.vsix

code --install-extension my-org.my-extension-1.0.0.vsix
```

---

## Composer (PHP) {#composer}

### Point Composer at BatleHub

Add a repository entry to `composer.json` in your project. BatleHub implements the Packagist v2 protocol (`packages.json` + `p2/` metadata endpoints), so Composer treats it as a native Composer repository.

```json
{
  "repositories": [
    {
      "type": "composer",
      "url": "https://batlehub.example.com/proxy/packagist/",
      "options": {
        "http": {
          "header": ["Authorization: Bearer ${BATLEHUB_TOKEN}"]
        }
      }
    }
  ]
}
```

For credentials, store them in `auth.json` (in `~/.config/composer/` or the project root — never commit this file):

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

When `auth.json` is in place, the `options.http.header` entry in `composer.json` is not needed.

### Install packages

```sh
composer install
composer require symfony/console
```

### Publish a private package (local/hybrid mode)

The registry must be in `local` or `hybrid` mode — ask your administrator.

Create a ZIP archive that contains a `composer.json` at its root (or inside a single top-level directory, like a GitHub archive):

```sh
# Create the ZIP — composer.json must have "name" and "version" fields
zip -r my-vendor-my-pkg-1.0.0.zip my-vendor-my-pkg-1.0.0/

# Upload
curl -X POST \
  -H "Authorization: Bearer $BATLEHUB_TOKEN" \
  -H "Content-Type: application/zip" \
  --data-binary @my-vendor-my-pkg-1.0.0.zip \
  "https://batlehub.example.com/proxy/internal-composer/api/upload"
```

The `name` field in `composer.json` must follow the `vendor/package` format (e.g. `"name": "my-vendor/my-pkg"`). The `version` field is used as the package version; it can be overridden by the `?version=` query parameter on the upload URL.

### Yank a version

```sh
curl -X DELETE \
  -H "Authorization: Bearer $BATLEHUB_TOKEN" \
  "https://batlehub.example.com/proxy/internal-composer/api/packages/my-vendor/my-pkg/versions/1.0.0"
```

Yanked versions are hidden from version listings and return 404 on download attempts.

### Verify

```sh
# List available versions of a package
curl -s "https://batlehub.example.com/proxy/packagist/p2/symfony/console.json" \
  -H "Authorization: Bearer $BATLEHUB_TOKEN" | jq '.packages | keys'
```

---

## PyPI (Python packages) {#pypi}

### Point pip at BatleHub

Add to `~/.pip/pip.conf` (Linux/macOS) or `%APPDATA%\pip\pip.ini` (Windows):

```ini
[global]
index-url = https://batlehub.example.com/proxy/my-pypi/simple/
```

For **uv**, add to `pyproject.toml`:

```toml
[[tool.uv.index]]
name = "batlehub"
url = "https://batlehub.example.com/proxy/my-pypi/simple/"
default = true
```

Both tools read credentials from `~/.netrc` automatically:

```
machine batlehub.example.com
login <your-user-id>
password <your-token>
```

Alternatively, embed credentials in the URL:

```ini
index-url = https://__token__:<your-token>@batlehub.example.com/proxy/my-pypi/simple/
```

### Install packages

```sh
pip install requests
uv pip install requests
poetry add requests   # after configuring the source in pyproject.toml
```

### Publish a private package (local/hybrid mode)

The registry must be in `local` or `hybrid` mode — ask your administrator.

Build and upload with `twine`:

```sh
# Build wheel and source distribution
python -m build

# Upload via twine
twine upload \
  --repository-url https://batlehub.example.com/proxy/my-private-pypi/legacy/ \
  --username __token__ \
  --password $BATLEHUB_TOKEN \
  dist/*
```

Or configure `~/.pypirc` for convenience:

```ini
[distutils]
index-servers = batlehub

[batlehub]
repository = https://batlehub.example.com/proxy/my-private-pypi/legacy/
username = __token__
password = <your-token>
```

Then: `twine upload --repository batlehub dist/*`

### Browse published packages

After publishing, the package appears in the Simple index immediately:

```sh
curl -s "https://batlehub.example.com/proxy/my-private-pypi/simple/my-package/" \
  -H "Authorization: Bearer $BATLEHUB_TOKEN"
```

---

## Conda {#conda}

### Point conda at BatleHub

Add to `~/.condarc` (or a `.condarc` in the project root):

```yaml
channels:
  - https://batlehub.example.com/proxy/my-conda
  - nodefaults
```

Conda reads credentials from `~/.netrc` automatically:

```
machine batlehub.example.com
login <your-user-id>
password <your-token>
```

### Install packages

```sh
conda install numpy
conda env create -f environment.yml
```

An `environment.yml` with the BatleHub channel:

```yaml
name: myenv
channels:
  - https://batlehub.example.com/proxy/my-conda
  - nodefaults
dependencies:
  - python=3.11
  - numpy
```

### Publish a private conda package (local/hybrid mode)

The registry must be in `local` or `hybrid` mode — ask your administrator.

Build the package with `conda build`, then upload:

```sh
# Build
conda build my-recipe/

# Upload (.tar.bz2 or .conda format both accepted)
curl -X POST \
  -H "Authorization: Bearer $BATLEHUB_TOKEN" \
  -H "Content-Type: application/octet-stream" \
  --data-binary @my-pkg-1.0.0-py311h0_0.tar.bz2 \
  "https://batlehub.example.com/proxy/my-private-conda/linux-64/"
```

The package is extracted automatically — name, version, build, and dependencies are read from `info/index.json` inside the archive. The channel's `repodata.json` is updated immediately.

### Verify

```sh
# Check repodata.json for your package
curl -s "https://batlehub.example.com/proxy/my-conda/linux-64/repodata.json" \
  -H "Authorization: Bearer $BATLEHUB_TOKEN" \
  | python3 -c "import sys,json; d=json.load(sys.stdin); print(list(d['packages'].keys())[:10])"
```

---

## Team Namespace dashboard {#team-namespace}

If your administrator has assigned namespace claims to your group, the **Team Namespace** page at `/my-namespace` gives you a single place to view your ownership, browse published packages, manage visibility, and upload new packages without needing CLI access.

### Your groups {#ns-groups}

The top card lists every auth-provider group you belong to. These are the values your administrator uses when creating namespace claims. Spaces are stripped from group names because package prefixes cannot contain spaces — `"oidc:my team"` is shown and matched as `"oidc:myteam"`.

### Your namespaces {#ns-namespaces}

The **My namespaces** table shows every namespace prefix claimed for your groups, across all registries. Each row shows:

| Column | Description |
|--------|-------------|
| Registry | The registry this claim applies to |
| Prefix | Package name prefix your group owns |
| Group | The group identifier (spaces stripped) |

Click any row to load the packages published under that namespace.

### Browsing and managing packages {#ns-packages}

After clicking a namespace row, the **Packages** card shows all published versions under that prefix. Columns include package name, version, visibility, publisher, and publication date.

**Changing visibility inline:**

Click the visibility badge on any row (or the "Edit visibility" button) to open an inline dropdown. Choose the new level and click **Save**:

| Level | Who can download |
|-------|-----------------|
| `public` | Everyone, including unauthenticated |
| `internal` | Any authenticated user |
| `team` | Members of your group only |

Results are paginated (50 per page). Use the Previous / Next buttons to navigate.

### Uploading packages {#ns-upload}

The **Upload package** card lets you publish directly from the browser for registry types that accept binary file uploads. Only registries in `local` or `hybrid` mode appear in the selector.

#### File upload (browser)

| Registry type | Accepted file | Extra fields |
|--------------|---------------|--------------|
| RubyGems | `.gem` | None — name and version are read from the gem |
| Composer | `.zip` | None — name and version are read from `composer.json` inside the archive |
| OpenVSX / VS Code Marketplace | `.vsix` | Extension ID (`publisher.name`) and version |
| Go modules | `.zip` | Module path (e.g. `github.com/org/repo`) and version (e.g. `v1.0.0`) |
| PyPI | `.whl`, `.tar.gz`, `.zip` | None — name and version are parsed from the filename |
| Conda | `.tar.bz2`, `.conda` | Platform (e.g. `linux-64`) — name, version, and build are read from `info/index.json` |

Select the registry, fill in any extra fields, choose the file, and click **Upload**.

::: tip Go module zip format
The zip must follow the standard Go module layout — every entry must be prefixed with `{module}@{version}/`. Running `go mod zip` produces this layout automatically.
:::

#### CLI (npm, Cargo, Maven, Terraform)

For registry types without a browser-friendly binary format, the **CLI instructions** tab shows ready-to-paste commands pre-filled with your registry name. See [the full publishing guide](#npm) for each ecosystem's complete setup steps.

---

## Permissions

| Permission | What it grants |
|-----------|----------------|
| `releases:read` | List versions, download release assets and metadata |
| `source:read` | Download source archives (tarballs, `.crate`, module `.zip`) |
| `*` | All permissions (admin) |

Role inheritance: `admin` ⊃ `user` ⊃ `anonymous`. Your administrator can assign additional permissions to OIDC groups or Kubernetes service account namespaces on top of your role.

---

## Troubleshooting

**`403 Forbidden` on download:** Your token is missing or your role doesn't have `releases:read` or `source:read` for this registry. Check with your administrator.

**`403 Forbidden` on publish — "registry is not in local or hybrid mode":** Publishing is disabled on this registry. Ask your administrator to enable `mode = "local"` or `mode = "hybrid"`.

**`409 Conflict` on publish:** The version already exists. Bump the version in your package manifest.

**`cargo publish` fails with "invalid token":** Verify the `index` URL in `.cargo/config.toml` ends with `/registry/`:
```
sparse+https://batlehub.example.com/proxy/internal/registry/
```

**Go: `disabled by GOPROXY=...off`:** The proxy can't reach the upstream or the module doesn't exist there. Remove `,off` from `GOPROXY` to allow direct fallback, or check that the upstream is reachable from the BatleHub server.
