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

Point the go toolchain at your registry. The `,direct` fallback lets the go tool reach the internet when BatleHub returns a 404:

```bash
export GONOSUMCHECK="*"
export GONOSUMDB="*"
export GOPROXY="https://batlehub.example.com/proxy/<registry>,direct"

go get golang.org/x/text@v0.3.7
```

To persist these, use `go env -w GOPROXY="https://batlehub.example.com/proxy/<registry>,direct"` (and likewise for `GONOSUMCHECK` / `GONOSUMDB`, which disable the public checksum database — required for private modules).

## Publishing (local / hybrid)

Build a standard Go module zip, then upload it. The module path may contain slashes; BatleHub extracts `go.mod` from the zip and generates version metadata automatically:

```bash
go mod zip example.com/mymod@v1.0.0 . --mod-zip /tmp/mymod-v1.0.0.zip

curl -X PUT \
  -H "Authorization: Bearer $BATLEHUB_TOKEN" \
  -H "Content-Type: application/zip" \
  --data-binary @/tmp/mymod-v1.0.0.zip \
  "https://batlehub.example.com/proxy/<registry>/example.com/mymod/@v/v1.0.0.zip"
```

Every entry inside the zip must be prefixed with `{module}@{version}/` — `go mod zip` produces this layout automatically.

## Authentication

Uploads pass a BatleHub token as a Bearer header. For read access, put the token in `~/.netrc` so the go tool and `govulncheck` pick it up automatically:

```
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
