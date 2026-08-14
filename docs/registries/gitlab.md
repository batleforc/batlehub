# GitLab

Proxy and cache releases, release-link assets, and source archives from a GitLab instance. Project paths may include nested groups; the release sub-path is separated by `/-/`, mirroring GitLab's own URLs. It also proxies the GitLab Packages API under `/api/v4/`.

## At a glance

| | |
|---|---|
| **Config type** | `gitlab` |
| **Default upstream** | `gitlab.com` |
| **Modes** | proxy-only |
| **Addressing** | per-package |
| **Private publish** | ❌ proxy-only |

## Proxy setup

Set `upstreams` to the instance root (e.g. `https://gitlab.com`). Replace `<registry>` with your configured registry name; add `-H "Authorization: Bearer $BATLEHUB_TOKEN"` when required:

```bash
REG="https://batlehub.example.com/proxy/<registry>"

# List releases / get a release by tag (nested groups allowed)
curl $REG/<group>/<project>/-/releases
curl $REG/<group>/<subgroup>/<project>/-/releases/v1.0.0

# Download a release link asset (matched by link name)
curl -L -O $REG/<group>/<project>/-/releases/v1.0.0/downloads/app.bin

# Source archive for a tag (format inferred from the extension)
curl -L -O $REG/<group>/<project>/-/archive/v1.0.0/source.tar.gz

# Raw file from the repository
curl -L $REG/<group>/<project>/-/raw/main/README.md
```

A `gitlab` registry also transparently caches the GitLab **Packages API** under `/api/v4/…` — ideal for the **generic** package registry:

```bash
curl -L -O https://batlehub.example.com/proxy/<registry>/api/v4/projects/<id>/packages/generic/<name>/<version>/<file>
```

For **ecosystem** registries (npm, Maven, PyPI, NuGet, Composer, …), point the matching typed adapter at the GitLab package endpoint instead so metadata URLs are rewritten and cached.

## Authentication

Pass a BatleHub token as a Bearer header (`-H "Authorization: Bearer $BATLEHUB_TOKEN"`) when the registry's RBAC requires it. GitLab personal access tokens use the `PRIVATE-TOKEN` header — configure it as a custom upstream auth header on the registry to reach private projects.

## Notes

- Proxy/cache only: the first request is streamed from upstream and cached.
- The release sub-path is separated by `/-/`, exactly as in GitLab's own URLs; nested group paths are supported.

## See also

- [Using BatleHub](/use/) — tokens, publishing prerequisites, the CLI
- [Registries overview](/registries/) · [Caching](/guide/caching) · [Access Control](/guide/access-control)
