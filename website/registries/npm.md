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

The registry must be in `local` or `hybrid` mode — ask your administrator.

```bash
npm publish --registry https://batlehub.example.com/proxy/<registry>/
```

With `.npmrc` already pointed at the registry, plain `npm publish` works too.

## Authentication

Pass a BatleHub token as the npm auth token (`_authToken`). Anonymous access works only when the registry's RBAC grants the `anonymous` role read access.

## Notes

`npm audit` works automatically once the registry is configured — both the quick and bulk audit modes are proxied through BatleHub to the upstream advisory database:

```bash
npm audit
npm audit --fix
```

## See also

- [User Guide → npm](/guide/user#registries)
- [Registries overview](/registries/) · [Caching](/guide/caching) · [Access Control](/guide/access-control)
