# NuGet (.NET)

Proxy and cache the NuGet gallery for `dotnet`, or host private packages. BatleHub synthesises the v3 service index (`index.json`) so all resource URLs point back at the proxy, gated by RBAC and the release-age gate.

## At a glance

| | |
|---|---|
| **Config type** | `nuget` |
| **Default upstream** | `api.nuget.org` |
| **Modes** | proxy · local · hybrid |
| **Addressing** | per-package |
| **Private publish** | ✅ `dotnet nuget push` |

## Proxy setup

Add the source with the CLI. Replace `<registry>` with your configured registry name:

```sh
dotnet nuget add source \
  https://batlehub.example.com/proxy/<registry>/nuget/v3/index.json \
  --name batlehub \
  --username __token__ \
  --password $BATLEHUB_TOKEN
```

Then install as usual:

```sh
dotnet add package Newtonsoft.Json
dotnet restore
```

## Publishing (local / hybrid)

NuGet packages are `.nupkg` files (ZIP archives containing a `.nuspec` manifest). BatleHub implements the [NuGet v3 protocol](https://learn.microsoft.com/en-us/nuget/api/overview), compatible with `dotnet` CLI, `nuget.exe`, and any NuGet v3 client.

### Config

```toml
[[registries]]
type = "nuget"
name = "internal-nuget"
mode = "local"          # or "hybrid" to fall back to api.nuget.org

[registries.rbac]
user  = ["releases:read"]
admin = ["*"]
```

For hybrid mode add `upstreams = ["https://api.nuget.org"]`.

### Configure dotnet / nuget.config

**CLI (one-time):**
```bash
dotnet nuget add source \
  https://batlehub.example.com/proxy/internal-nuget/nuget/v3/index.json \
  --name internal-nuget \
  --username __token__ --password <api-token>
```

**`nuget.config` (project-level):**
```xml
<?xml version="1.0" encoding="utf-8"?>
<configuration>
  <packageSources>
    <add key="internal-nuget"
         value="https://batlehub.example.com/proxy/internal-nuget/nuget/v3/index.json" />
  </packageSources>
  <packageSourceCredentials>
    <internal-nuget>
      <add key="Username" value="__token__" />
      <add key="ClearTextPassword" value="<api-token>" />
    </internal-nuget>
  </packageSourceCredentials>
</configuration>
```

### Publish with dotnet nuget push

Pack your project first, then push:

```bash
dotnet pack MyLib.csproj -c Release

dotnet nuget push bin/Release/MyLib.1.0.0.nupkg \
  --api-key <api-token> \
  --source https://batlehub.example.com/proxy/internal-nuget/nuget/v3/index.json
```

The publish endpoint accepts `multipart/form-data` (as sent by `dotnet nuget push`). On success it returns **201 Created**.

### Yank a version

```bash
curl -X DELETE \
  -H "Authorization: Bearer <api-token>" \
  "https://batlehub.example.com/proxy/internal-nuget/nuget/v2/package/mylib/1.0.0"
```

### Consume a package

```bash
# Add the package — dotnet fetches the index, resolves the version, downloads the .nupkg
dotnet add package MyLib --version 1.0.0 --source internal-nuget

# Restore all project dependencies
dotnet restore
```

### Verify

```bash
# Service index should return JSON with "version": "3.0.0"
curl -s https://batlehub.example.com/proxy/internal-nuget/nuget/v3/index.json | jq '.version'

# Flat container version list after publish
curl -s https://batlehub.example.com/proxy/internal-nuget/nuget/v3/flat/mylib/index.json
# → {"versions":["1.0.0"]}
```

### Endpoint reference

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/proxy/{registry}/nuget/v3/index.json` | Generated service index |
| `GET` | `/proxy/{registry}/nuget/v3/flat/{id}/index.json` | Version list |
| `GET` | `/proxy/{registry}/nuget/v3/flat/{id}/{ver}/{file}` | Download `.nupkg` / `.nuspec` |
| `GET` | `/proxy/{registry}/nuget/v3/registration5/{id}/index.json` | Package metadata |
| `GET` | `/proxy/{registry}/nuget/v3/query` | Search |
| `PUT` | `/proxy/{registry}/nuget/api/v2/package` | Publish `.nupkg` |
| `DELETE` | `/proxy/{registry}/nuget/v2/package/{id}/{ver}` | Yank |

---

## Blocked versions

Both of NuGet's listing documents hide an administratively blocked version.

- The **flat index** (`/v3/flat/{id}/index.json`) — what `dotnet restore`
  resolves a version range against — drops the version outright.
- The **registration pages** drop the leaf and recompute each page's `count`,
  `lower` and `upper`; a page left empty is removed. Registrations whose pages
  are served by URL rather than inline pass through unfiltered and are logged.

Version spellings are folded before comparison, so a block recorded as
`1.0.0.0` hides a listing that spells the same release `1.0.0`.

The upstream document is cached for the registry's `metadata_ttl`; blocks are
applied on top of the cached copy on every request, so blocking a version takes
effect immediately rather than when the cache expires.

See [blocking a package version](/guide/admin-policies#block-a-package-version) for the two halves of a block, and [which listings are filtered](/guide/admin-policies#which-listings-are-filtered) for the full table.

## Authentication

Pass the BatleHub token as the source password (username `__token__`), or as `--api-key` on push. The `X-NuGet-ApiKey` header is normalised to `Authorization: Bearer` internally, so `--api-key $BATLEHUB_TOKEN` is accepted as a Bearer token.

## Notes

- `dotnet list package --vulnerable` works automatically — BatleHub advertises a `VulnerabilitiesUrl` resource in the v3 service index and proxies the upstream vulnerability catalogue. See [Using BatleHub → security auditing](/use/#security-audit).
- A `401` on push usually means the token lacks `releases:write` (or admin) on the registry.

## See also

- [Using BatleHub](/use/) — tokens, publishing prerequisites, the CLI
- [Registries overview](/registries/) · [Caching](/guide/caching) · [Access Control](/guide/access-control)
