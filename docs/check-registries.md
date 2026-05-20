# Registry Health Check

`scripts/check-registries.sh` validates that a running proxy-cache instance is working correctly for each registry type. It goes beyond HTTP status codes by using real package manager tooling — `npm install`, `cargo add`, `go get` — so you catch misconfigurations that a simple `curl` would miss.

---

## Table of Contents

1. [Prerequisites](#1-prerequisites)
2. [Usage](#2-usage)
3. [What Each Check Does](#3-what-each-check-does)
   - [npm](#31-npm)
   - [Cargo](#32-cargo)
   - [Go](#33-go)
   - [GitHub](#34-github)
   - [OpenVSX](#35-openvsx)
   - [VS Code Marketplace](#36-vs-code-marketplace)
4. [Authentication](#4-authentication)
5. [Exit Codes and CI Use](#5-exit-codes-and-ci-use)
6. [Common Failures](#6-common-failures)

---

## 1. Prerequisites

The script itself only requires `bash` and `curl`. Tool checks are skipped gracefully when the corresponding tool is not installed:

| Registry | Required tool | Minimum version |
| --- | --- | --- |
| npm | `npm` | any |
| Cargo | `cargo` | 1.62 (for `cargo add`) |
| Go | `go` | 1.21 (for `NETRC` env var) |
| GitHub | `curl` | any |
| OpenVSX | `curl` | any |
| VS Code Marketplace | `curl` | any |

JSON field validation uses `jq` when available; without it the script falls back to `grep`-based checks.

---

## 2. Usage

```sh
./scripts/check-registries.sh [options]

  --url <url>        Base URL of the running proxy (default: http://localhost:8080)
  --token <tok>      Bearer token for authenticated endpoints (optional)
  --npm <name>       Test the npm registry named <name>
  --cargo <name>     Test the cargo registry named <name>
  --go <name>        Test the go registry named <name>
  --github <name>    Test the github registry named <name>
  --openvsx <name>              Test the openvsx registry named <name>
  --vscode-marketplace <name>   Test the vscode-marketplace registry named <name>
```

The `<name>` value for each flag is the `name` field you assigned that registry in your `config.toml`, not the `type`. Only the registries you specify are tested.

**Test all registry types against a local instance:**

```sh
./scripts/check-registries.sh \
  --npm npm \
  --cargo cargo \
  --go go \
  --github github \
  --openvsx openvsx \
  --vscode-marketplace vscode
```

**Test a remote instance with custom registry names and auth:**

```sh
./scripts/check-registries.sh \
  --url https://registry.example.com \
  --token mytoken \
  --npm public-npm \
  --cargo internal-crates
```

**Test only npm and cargo:**

```sh
./scripts/check-registries.sh --npm npm --cargo cargo
```

---

## 3. What Each Check Does

Each registry gets two checks: an **HTTP check** (direct `curl` against the proxy endpoint) and a **tool check** (real package manager invocation in an isolated temp directory). Both must pass for the registry to be considered healthy.

### 3.1 npm

The proxy streams the raw package tarball (`.tgz`) for every npm endpoint — it is a binary download cache, not a packument-serving npm registry. The npm endpoints do not return JSON.

**HTTP check** — downloads the `ms` package tarball and verifies the gzip magic bytes (`1f 8b`):

```text
GET /proxy/<name>/ms  →  200, binary .tgz
```

**Tool check** — downloads a versioned tarball (`ms@2.1.3`) and validates its tar structure:

```text
GET /proxy/<name>/ms/2.1.3/tarball  →  200, valid .tgz (verified with tar tzf)
```

This exercises both the metadata resolution path (`/ms`) and the versioned tarball download path (`/ms/2.1.3/tarball`), which requires `source:read` permission.

### 3.2 Cargo

**HTTP check** — fetches the sparse index configuration:

```text
GET /proxy/<name>/registry/config.json  →  200, { "dl": "...", ... }
```

**Tool check** — creates a minimal Rust project with a `.cargo/config.toml` that points at the proxy, then resolves `serde` through it:

```sh
cargo add serde --registry <name>
```

The `.cargo/config.toml` written by the script:

```toml
[registries.<name>]
index = "sparse+http://HOST/proxy/<name>/registry/"

[source.crates-io]
replace-with = "<name>"

[source.<name>]
registry = "sparse+http://HOST/proxy/<name>/registry/"
```

This validates both the sparse index endpoint and the crate download path.

### 3.3 Go

**HTTP check** — fetches the latest version info for `golang.org/x/text`:

```text
GET /proxy/<name>/golang.org/x/text/@latest  →  200, { "Version": "v0.x.y", ... }
```

**Tool check** — initializes a temporary Go module and fetches the pinned version through the proxy:

```sh
GOPROXY=http://HOST/proxy/<name>,off \
GONOSUMDB=* \
GONOSUMCHECK=* \
go get golang.org/x/text@<version-from-http-check>
```

The exact version is taken from the `@latest` HTTP response (e.g. `v0.37.0`). Using a pinned version avoids the `/@v/list` endpoint, which is not required for versioned lookups. Using `,off` means the test fails clearly if the proxy can't reach upstream, rather than silently falling back to the internet.

### 3.4 GitHub

Both checks use the asset download endpoint rather than the JSON release metadata endpoint. The metadata path calls the GitHub REST API (rate-limited to 60 unauthenticated requests per hour), while the asset download path is cached by the proxy and served without an API call after the first request.

**HTTP check** — verifies that a known release asset is accessible through the proxy:

```text
GET /proxy/<name>/cli/cli/releases/download/v2.48.0/gh_2.48.0_linux_amd64.tar.gz  →  200
```

**Tool check** — downloads the asset and verifies the gzip magic bytes (`1f 8b`).

### 3.5 OpenVSX

**HTTP check** — requests a VS Code extension VSIX and accepts any non-5xx response (a 404 from the upstream open-vsx.org is acceptable):

```text
GET /proxy/<name>/redhat.java/1.26.0/vsix  →  non-5xx
```

**Tool check** — downloads the VSIX to a temp file and verifies the ZIP magic bytes (`PK\x03\x04`), confirming the proxy returned a valid VSIX archive rather than an error page. A 404 from upstream causes this check to be skipped rather than failed.

### 3.6 VS Code Marketplace

**HTTP check** — requests a VS Code extension VSIX and accepts any non-5xx response (a 404 from upstream is acceptable):

```text
GET /proxy/<name>/ms-python.python/2024.2.1/vsix  →  non-5xx
```

**Tool check** — downloads the VSIX to a temp file and verifies the ZIP magic bytes (`PK\x03\x04`), confirming the proxy returned a valid VSIX archive rather than an error page. A 404 from upstream causes this check to be skipped rather than failed.

---

## 4. Authentication

Pass `--token <tok>` to send a `Bearer` token on all requests. The token is also threaded through to each tool:

| Tool | Mechanism |
| --- | --- |
| `curl` (all HTTP checks) | `Authorization: Bearer <tok>` header |
| `npm` | `.npmrc` entry: `//HOST/proxy/<name>/:_authToken=<tok>` |
| `cargo` | `CARGO_REGISTRIES_<NAME>_TOKEN` environment variable |
| `go` | Temp `.netrc` file; `NETRC` env var points at it |

---

## 5. Exit Codes and CI Use

| Code | Meaning |
| --- | --- |
| `0` | All checks passed (skipped checks do not count as failures) |
| `1` | One or more checks failed |

The script is safe to use in CI pipelines. It respects `NO_COLOR` and produces clean output when stdout is not a TTY.

Example GitHub Actions step:

```yaml
- name: Check registries
  run: |
    ./scripts/check-registries.sh \
      --url ${{ vars.PROXY_URL }} \
      --token ${{ secrets.PROXY_TOKEN }} \
      --npm npm \
      --cargo cargo \
      --go go
```

---

## 6. Common Failures

**`cargo:http — HTTP 404`**
The sparse index path is wrong. Verify the registry `name` in your config and that the `type` is `"cargo"`.

**`cargo:tool — cargo add failed` with "no matching package"**
The proxy returned a valid index response but the crate wasn't found upstream. Check that the proxy can reach `index.crates.io`.

**`go:http — HTTP 200` but `go:tool` fails with "disabled by GOPROXY=...off"**
The HTTP endpoint works but `go get` cannot fetch the module zip. This usually means the proxy's upstream (`proxy.golang.org`) is unreachable from where the proxy is running.

**`github:http — HTTP 403`**
The proxy's upstream GitHub auth is not configured, and the anonymous GitHub API rate limit has been hit. Add a GitHub token to the registry's `upstream_auth` in `config.toml`.

**`npm:tool — SKIPPED` / `cargo:tool — SKIPPED`**
The tool is not installed in the environment running the script. Install it, or treat the HTTP check result alone as sufficient for your use case.

**`openvsx:tool — SKIP (HTTP 404)`**
The extension version requested (`redhat.java/1.26.0`) does not exist on open-vsx.org. This is expected on some deployments; the HTTP check (non-5xx) is the authoritative signal.

**`vscode-marketplace:tool — SKIP (HTTP 404)`**
The extension version requested (`ms-python.python/2024.2.1`) was not found on marketplace.visualstudio.com. Try a different extension or version, or check that the proxy can reach the upstream.
