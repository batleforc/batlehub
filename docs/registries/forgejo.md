# Forgejo / Gitea

Proxy and cache release assets, source archives, and raw files from a [Forgejo](https://forgejo.org) or Gitea instance. It reuses the GitHub-style URL scheme, and also proxies the Forgejo/Gitea package registry under `/api/packages/`.

## At a glance

| | |
|---|---|
| **Config type** | `forgejo` |
| **Default upstream** | `codeberg.org` |
| **Modes** | proxy-only |
| **Addressing** | per-package |
| **Private publish** | ❌ proxy-only |

## Proxy setup

Set `upstreams` to the instance root (e.g. `https://codeberg.org`). Address a repository by `<owner>/<repo>`; replace `<registry>` with your configured registry name and add `-H "Authorization: Bearer $BATLEHUB_TOKEN"` when required:

```bash
REG="https://batlehub.example.com/proxy/<registry>"

# List releases / get a release by tag
curl $REG/<owner>/<repo>/releases
curl $REG/<owner>/<repo>/releases/tags/v1.0.0

# Download a release asset by filename
curl -L -O $REG/<owner>/<repo>/releases/download/v1.0.0/app.tar.gz

# Source tarball / zip for a tag, branch, or commit
curl -L -O $REG/<owner>/<repo>/tarball/v1.0.0
curl -L -O $REG/<owner>/<repo>/zipball/v1.0.0

# Raw file
curl -L $REG/<owner>/<repo>/raw/main/README.md
```

A `forgejo` registry also transparently caches the Forgejo/Gitea **package registry** at `/api/packages/{owner}/…` — ideal for the **generic** package registry:

```bash
curl -L -O https://batlehub.example.com/proxy/<registry>/api/packages/<owner>/generic/<name>/<version>/<file>
```

For **ecosystem** registries (npm, Maven, PyPI, Composer, NuGet, …), point the matching typed adapter at the package endpoint instead so metadata URLs are rewritten and cached.

## Blocked versions

The release listing drops a blocked release, newest-first order intact, so a
client never selects a release whose assets it will then be refused.

A release is identified by its **tag**, and the same release is tagged `1.2.3`
in one repository and `v1.2.3` in the next. A block matches either spelling, so
it does not depend on which habit the repository follows.

The upstream document is cached for the registry's `metadata_ttl`; blocks are
applied on top of the cached copy on every request, so blocking a version takes
effect immediately rather than when the cache expires.

See [blocking a package version](/guide/admin-policies#block-a-package-version) for the two halves of a block, and [which listings are filtered](/guide/admin-policies#which-listings-are-filtered) for the full table.

## Authentication

Pass a BatleHub token as a Bearer header (`-H "Authorization: Bearer $BATLEHUB_TOKEN"`) when the registry's RBAC requires it. For **private instances**, configure a bearer token as the registry's upstream auth in the server config.

## Notes

- Proxy/cache only: the first request is streamed from upstream and cached.
- The URL scheme is identical to [GitHub](/registries/github).

## See also

- [Using BatleHub](/use/) — tokens, publishing prerequisites, the CLI
- [Registries overview](/registries/) · [Caching](/guide/caching) · [Access Control](/guide/access-control)
