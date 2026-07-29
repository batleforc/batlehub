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

The go toolchain has no `go mod zip` command — the canonical way to build a proxy-compatible module zip is [`golang.org/x/mod/zip`](https://pkg.go.dev/golang.org/x/mod/zip), the same package `go mod download` uses. A four-line helper is enough:

```go
// zipmod.go — go run zipmod.go <module> <version> <dir> <out.zip>
package main

import (
	"log"
	"os"

	"golang.org/x/mod/module"
	"golang.org/x/mod/zip"
)

func main() {
	mod, ver, dir, out := os.Args[1], os.Args[2], os.Args[3], os.Args[4]
	f, err := os.Create(out)
	if err != nil {
		log.Fatal(err)
	}
	defer f.Close()
	if err := zip.CreateFromDir(f, module.Version{Path: mod, Version: ver}, dir); err != nil {
		log.Fatal(err)
	}
}
```

Then build and upload it. The module path may contain slashes; BatleHub extracts `go.mod` from the zip and generates version metadata automatically:

```bash
go run zipmod.go example.com/mymod v1.0.0 . /tmp/mymod-v1.0.0.zip

curl -X PUT \
  -H "Authorization: Bearer $BATLEHUB_TOKEN" \
  -H "Content-Type: application/zip" \
  --data-binary @/tmp/mymod-v1.0.0.zip \
  "https://batlehub.example.com/proxy/<registry>/example.com/mymod/@v/v1.0.0.zip"
```

Every entry inside the zip must be prefixed with `{module}@{version}/` (here `example.com/mymod@v1.0.0/`) — `zip.CreateFromDir` produces exactly that layout, and also enforces the module-zip rules (no symlinks, no nested modules, size limits), so a zip it accepts is one the go tool will accept.

## Authentication

Uploads pass a BatleHub token as a Bearer header. For read access, put the token in `~/.netrc` so the go tool and `govulncheck` pick it up automatically:

```text
machine batlehub.example.com login user password $BATLEHUB_TOKEN
```

## Notes

BatleHub proxies the [Go Vulnerability Database](https://vuln.go.dev) so `govulncheck` works without reaching vuln.go.dev. Set `GOVULNDB` to the same base URL as `GOPROXY`:

```bash
export GOVULNDB="https://batlehub.example.com/proxy/<registry>"
govulncheck ./...
```

The upstream vuln DB URL defaults to `https://vuln.go.dev` and can be overridden per registry with `vuln_db_url` in the server config; setting it to `""` disables the endpoints. `@latest` and `@v/list` responses are cached — clear proxy storage to pick up newly published versions immediately.

## See also

- [User Guide → Go Modules](/guide/user#registries)
- [Registries overview](/registries/) · [Caching](/guide/caching) · [Access Control](/guide/access-control)
