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

The registry must be in `local` or `hybrid` mode. Pack and push:

```sh
dotnet pack MyLib.csproj -c Release

dotnet nuget push bin/Release/MyLib.1.0.0.nupkg \
  --api-key $BATLEHUB_TOKEN \
  --source https://batlehub.example.com/proxy/<registry>/nuget/v3/index.json
```

`dotnet nuget push` sends `multipart/form-data`; BatleHub returns **201 Created** on success and **409 Conflict** if the version already exists. Yank a version with `DELETE …/nuget/v2/package/{id}/{version}`.

## Authentication

Pass the BatleHub token as the source password (username `__token__`), or as `--api-key` on push. The `X-NuGet-ApiKey` header is normalised to `Authorization: Bearer` internally, so `--api-key $BATLEHUB_TOKEN` is accepted as a Bearer token.

## Notes

- `dotnet list package --vulnerable` works automatically — BatleHub advertises a `VulnerabilitiesUrl` resource in the v3 service index and proxies the upstream vulnerability catalogue. See [User Guide → Check for vulnerable packages](/guide/user#registries).
- A `401` on push usually means the token lacks `releases:write` (or admin) on the registry.

## See also

- [User Guide → NuGet (.NET)](/guide/user#registries)
- [Registries overview](/registries/) · [Caching](/guide/caching) · [Access Control](/guide/access-control)
