# npm

Proxy and cache the npm registry, or host private npm packages. BatleHub serves the full packument metadata plus tarball downloads, gated by RBAC and the release-age gate. `npm audit` works through the proxy.

## At a glance

| | |
|---|---|
| **Config type** | `npm` |
| **Default upstream** | `registry.npmjs.org` |
| **Modes** | proxy · local · hybrid |
| **Addressing** | per-package |
| **Private publish** | ✅ `npm publish` |

## Proxy setup

Point npm at your registry. Replace `<registry>` with your configured registry name and set `BATLEHUB_TOKEN` if the registry requires auth:

```ini
# .npmrc (project root or ~/.npmrc)
registry=https://batlehub.example.com/proxy/<registry>/
//batlehub.example.com/proxy/<registry>/:_authToken=${BATLEHUB_TOKEN}
```

To route only a specific scope through the proxy, use `@myorg:registry=https://batlehub.example.com/proxy/<registry>/` instead. **pnpm** reads the same `.npmrc` keys unchanged.

**Yarn Berry** (Yarn 2+) does not read `.npmrc` — configure it in `.yarnrc.yml` with its own key names:

```yaml
# .yarnrc.yml
npmRegistryServer: "https://batlehub.example.com/proxy/<registry>/"
npmAuthToken: "${BATLEHUB_TOKEN}"

# Or, to route only one scope:
npmScopes:
  myorg:
    npmRegistryServer: "https://batlehub.example.com/proxy/<registry>/"
    npmAuthToken: "${BATLEHUB_TOKEN}"
```

## Publishing (local / hybrid)

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

The packument is BatleHub's own answer, not a copy of the upstream's. Two things
are rewritten before it reaches the client:

- **`dist.tarball` points back at BatleHub**, so downloads go through the proxy —
  its cache, its audit trail and its policy gates — instead of straight to the
  upstream CDN.
- **Blocked versions are removed**, and `dist-tags.latest` is recomputed to the
  newest version that is still allowed. See
  [blocking a package version](/guide/admin-policies#block-a-package-version).

The upstream document is cached for the registry's `metadata_ttl`; blocks are
applied on top of the cached copy on every request, so blocking a version takes
effect immediately rather than when the cache expires.

---

## Authentication

Pass a BatleHub token as the npm auth token (`_authToken`). Anonymous access works only when the registry's RBAC grants the `anonymous` role read access.

## Notes

`npm audit` works automatically once the registry is configured — both the quick and bulk audit modes are proxied through BatleHub to the upstream advisory database:

```bash
npm audit
npm audit --fix
```

## See also

- [Using BatleHub](/use/) — tokens, publishing prerequisites, the CLI
- [Registries overview](/registries/) · [Caching](/guide/caching) · [Access Control](/guide/access-control)
