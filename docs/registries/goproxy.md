# Go Modules

Proxy and cache Go modules via the [GOPROXY protocol](https://go.dev/ref/mod#goproxy-protocol), or host private modules. BatleHub caches module zips permanently after the first download and also proxies the Go vulnerability database so `govulncheck` works without direct internet access.

## At a glance

| | |
|---|---|
| **Config type** | `goproxy` |
| **Default upstream** | `proxy.golang.org` |
| **Modes** | proxy · local · hybrid |
| **Addressing** | per-package |
| **Private publish** | ✅ (module zip upload) |

## Proxy setup

Point the go toolchain at your registry:

```bash
export GOPROXY="https://batlehub.example.com/proxy/<registry>"

go get golang.org/x/text@v0.3.7
```

There is no `,direct` fallback here on purpose: every module then resolves through BatleHub, so the proxy stays the single ingress and a miss fails loudly instead of silently reaching the internet — usually the reason for running a proxy at all. Opt in explicitly when your build hosts *are* allowed to fetch directly and you want that fallback on a 404:

```bash
export GOPROXY="https://batlehub.example.com/proxy/<registry>,direct"
```

For **private modules**, list their path prefixes so the go tool stops checking them against the public checksum database (`sum.golang.org`), which has never seen them:

```bash
# Private and served by BatleHub: still proxied, but not sum-checked.
export GONOSUMDB="example.com/internal/*"

# Private and fetched straight from the VCS: GOPRIVATE implies both
# GONOPROXY and GONOSUMDB, so these bypass BatleHub entirely.
export GOPRIVATE="example.com/internal/*"
```

To persist any of these, use `go env -w`, e.g. `go env -w GOPROXY="https://batlehub.example.com/proxy/<registry>"`.

## Publishing (local / hybrid)

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

## Blocked versions

`@v/list` drops the blocked version's line, and `@latest` is **re-resolved**
against what survives rather than filtered — it names one version and carries no
list. The rebuilt `@latest` carries `Version` and omits `Time`, because the
timestamp belonged to the release being hidden. With no version left to name,
`@latest` answers `404`, which is what the Go client already handles for a
module with no releases.

A leading `v` and a `+incompatible` suffix name the same release either way, so
a block recorded in any of those spellings hides the listed one.

The upstream document is cached for the registry's `metadata_ttl`; blocks are
applied on top of the cached copy on every request, so blocking a version takes
effect immediately rather than when the cache expires.

See [blocking a package version](/guide/admin-policies#block-a-package-version) for the two halves of a block, and [which listings are filtered](/guide/admin-policies#which-listings-are-filtered) for the full table.

## Authentication

Uploads pass a BatleHub token as a Bearer header. For read access, put the token in `~/.netrc` so the go tool and `govulncheck` pick it up automatically:

```bash
cat >> ~/.netrc <<EOF
machine batlehub.example.com login user password $BATLEHUB_TOKEN
EOF
chmod 600 ~/.netrc
```

## Notes

BatleHub proxies the [Go Vulnerability Database](https://vuln.go.dev) so `govulncheck` works without reaching vuln.go.dev. Set `GOVULNDB` to the same base URL as `GOPROXY`:

```bash
export GOVULNDB="https://batlehub.example.com/proxy/<registry>"
govulncheck ./...
```

The upstream vuln DB URL defaults to `https://vuln.go.dev` and can be overridden per registry with `vuln_db_url` in the server config; setting it to `""` disables the endpoints. `@latest` and `@v/list` responses are cached — clear proxy storage to pick up newly published versions immediately.

## See also

- [Using BatleHub](/use/) — tokens, publishing prerequisites, the CLI
- [Registries overview](/registries/) · [Caching](/guide/caching) · [Access Control](/guide/access-control)
