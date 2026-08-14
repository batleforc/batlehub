# GitHub

Proxy and cache the GitHub REST API — release listings and metadata, release assets, source tarballs/zipballs, and raw repository files. The first request is streamed from upstream and cached, gated by RBAC and the release-age gate; there is no private publish.

## At a glance

| | |
|---|---|
| **Config type** | `github` |
| **Default upstream** | `api.github.com` |
| **Modes** | proxy-only |
| **Addressing** | per-package |
| **Private publish** | ❌ proxy-only |

## Proxy setup

Address a repository by `<owner>/<repo>`. Replace `<registry>` with your configured registry name; add `-H "Authorization: Bearer $BATLEHUB_TOKEN"` when the registry requires auth:

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

## Authentication

Pass a BatleHub token as a Bearer header (`-H "Authorization: Bearer $BATLEHUB_TOKEN"`) when the registry's RBAC requires it. For **private GitHub repositories** or to raise GitHub's rate limits, configure a GitHub token as the registry's upstream auth in the server config — the client never sees it.

## Notes

- Proxy/cache only: the first request is streamed from upstream and cached; later requests are served locally.
- Forgejo/Gitea registries reuse this same URL scheme — see [Forgejo](/registries/forgejo).
- Tools like `mise` (aqua, ubi backends) can be pointed here with URL replacements; see the Setup Guide.

## See also

- [User Guide → GitHub](/guide/user#registries)
- [Registries overview](/registries/) · [Caching](/guide/caching) · [Access Control](/guide/access-control)
