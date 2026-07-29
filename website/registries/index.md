# Registries

BatleHub proxies, caches, and privately hosts **21 registry types** — from language package managers to OS package repositories, editor extension marketplaces, and generic file mirrors.

Every registry type can run in one of three **modes**, set per registry in the config:

- **proxy** — a pure read-through cache in front of an upstream. The first request is fetched from upstream and stored; every later request is served from cache.
- **local** — a fully private registry. Nothing is fetched from upstream; you publish and serve your own artifacts.
- **hybrid** — local artifacts win, and anything not published locally falls through to the upstream proxy.

Five types are **proxy-only** (no private publish model): **GitHub**, **Forgejo**, **GitLab** (they host source/releases upstream) and **JetBrains** IDE archives + **Generic** file mirrors (path-only caches).

## The registries

### Source hosting

| Registry | `type` | What it proxies | Modes | Publish | Default upstream |
|----------|--------|-----------------|-------|:-------:|------------------|
| [GitHub](./github) | `github` | Releases, assets, tarballs, raw files | proxy-only | ❌ | `api.github.com` |
| [Forgejo / Gitea](./forgejo) | `forgejo` | Releases, assets, archives, raw (`/api/v1`) | proxy-only | ❌ | `codeberg.org` |
| [GitLab](./gitlab) | `gitlab` | Releases, link assets, archives (`/api/v4`) | proxy-only | ❌ | `gitlab.com` |

### Language package managers

| Registry | `type` | What it proxies | Modes | Publish | Default upstream |
|----------|--------|-----------------|-------|:-------:|------------------|
| [npm](./npm) | `npm` | Packument + tarballs | proxy · local · hybrid | ✅ | `registry.npmjs.org` |
| [Cargo](./cargo) | `cargo` | Sparse index + `.crate` | proxy · local · hybrid | ✅ | `crates.io` |
| [Go Modules](./goproxy) | `goproxy` | GOPROXY (`.info`/`.mod`/`.zip`) | proxy · local · hybrid | ✅ | `proxy.golang.org` |
| [Maven](./maven) | `maven` | Metadata XML + JAR/POM | proxy · local · hybrid | ✅ | `repo1.maven.org` |
| [PyPI](./pypi) | `pypi` | Simple API (PEP 503/691) + wheels | proxy · local · hybrid | ✅ | `pypi.org` |
| [Conda](./conda) | `conda` | `repodata.json` + `.conda`/`.tar.bz2` | proxy · local · hybrid | ✅ | `conda.anaconda.org` |
| [Composer (PHP)](./composer) | `composer` | Packagist v2 (p2 metadata + dist) | proxy · local · hybrid | ✅ | `repo.packagist.org` |
| [RubyGems](./rubygems) | `rubygems` | Gems + versions + info API | proxy · local · hybrid | ✅ | `rubygems.org` |
| [NuGet (.NET)](./nuget) | `nuget` | v3 index + flat + `.nupkg` | proxy · local · hybrid | ✅ | `api.nuget.org` |
| [Terraform](./terraform) | `terraform` | Providers + modules (v1 API) | proxy · local · hybrid | ✅ | `registry.terraform.io` |

### Editor extensions

| Registry | `type` | What it proxies | Modes | Publish | Default upstream |
|----------|--------|-----------------|-------|:-------:|------------------|
| [OpenVSX](./openvsx) | `openvsx` | Extension VSIX | proxy · local · hybrid | ✅ | `open-vsx.org` |
| [VS Code Marketplace](./vscode-marketplace) | `vscode-marketplace` | Extension VSIX (MS Gallery) | proxy · local · hybrid | ✅ | `marketplace.visualstudio.com` |
| [JetBrains Marketplace](./jetbrains-marketplace) | `jetbrains-marketplace` | Plugin API + downloads | proxy · local · hybrid | ✅ | `plugins.jetbrains.com` |

### OS / system packages <Badge type="tip" text="path-addressed" />

| Registry | `type` | What it proxies | Modes | Publish | Default upstream |
|----------|--------|-----------------|-------|:-------:|------------------|
| [Debian / APT](./deb) | `deb` | `Packages`/`Release` + `.deb` | proxy · local · hybrid | ✅ | none — set `upstreams` |
| [RPM / YUM / DNF](./rpm) | `rpm` | `repodata/` + `.rpm` | proxy · local · hybrid | ✅ | none — set `upstreams` |
| [Pacman / Arch](./pacman) | `pacman` | `<repo>.db` + `.pkg.tar.zst` | proxy · local · hybrid | ✅ | none — set `upstreams` |

### Binaries & mirrors <Badge type="tip" text="path-addressed" />

| Registry | `type` | What it proxies | Modes | Publish | Default upstream |
|----------|--------|-----------------|-------|:-------:|------------------|
| [JetBrains IDEs](./jetbrains) | `jetbrains` | IDE installer archives | proxy-only | ❌ | `download.jetbrains.com` |
| [Generic mirror](./generic) | `generic` | Any HTTP file tree | proxy-only | ❌ | none — set `upstreams` + `path_allow` |

## Feature matrix

Every registry and how its capabilities map across BatleHub's features. The
**path-addressed** types (Deb, RPM, Pacman, JetBrains IDEs, Generic) have no
per-package version model, so the structural axes (version listing, source
archive, binary, age gate, warming, search) show `—`. They still get
registry-level RBAC and multi-upstream fanout, and Deb/RPM/Pacman support signed
private hosting. **Forgejo** and **GitLab** mirror **GitHub**'s behaviour.

Legend: **Ver.** version listing · **Src** source archive · **Bin** binary/extension asset · **Pub** private publish · **Fan** multi-upstream fanout · **Age** release age gate · **Warm** cache warming (version enumeration) · **Search** Package Explorer upstream search. ✓ supported · `—` not applicable · ⚠ partial.

| Registry | Ver. | Src | Bin | Pub | Fan | Age | RBAC | Warm | Search |
|----------|:----:|:---:|:---:|:---:|:---:|:---:|:----:|:----:|:------:|
| GitHub | ✓ | ✓ | ✓ | — | ✓ | ✓ | ✓ | ✓ | — |
| Forgejo / Gitea | ✓ | ✓ | ✓ | — | ✓ | ✓ | ✓ | ✓ | — |
| GitLab | ✓ | ✓ | ✓ | — | ✓ | ✓ | ✓ | ✓ | — |
| npm | ✓ | ✓ | — | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| Cargo | ✓ | ✓ | — | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| Go Modules | ✓ | ✓ | — | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| Maven | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | — | ✓ |
| PyPI | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| Conda | ✓ ¹ | ✓ | ✓ | ✓ | ✓ | ⚠ ² | ✓ | ✓ ¹ | — |
| Composer | ✓ | ✓ | — | ✓ | ✓ | ✓ | ✓ | — | ✓ |
| RubyGems | ✓ | ✓ | — | ✓ | ✓ | ✓ | ✓ | — | ✓ |
| NuGet | ✓ | — | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| Terraform | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | — | ✓ |
| OpenVSX | ✓ | — | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| VS Code Marketplace | ✓ | — | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | — |
| JetBrains Marketplace | ✓ | — | ✓ | ✓ | ✓ | ✓ | ✓ | — | ✓ |
| Debian / APT ³ | — | — | — | ✓ | ✓ | — | ✓ | — | — |
| RPM / YUM / DNF ³ | — | — | — | ✓ | ✓ | — | ✓ | — | — |
| Pacman / Arch ³ | — | — | — | ✓ | ✓ | — | ✓ | — | — |
| JetBrains IDEs ³ | — | — | — | — | ✓ | — | ✓ | — | — |
| Generic ³ | — | — | — | — | ✓ | — | ✓ | — | — |

> ¹ Conda has no dedicated per-package version listing API. BatleHub synthesises one by scanning `repodata.json` across `noarch`, `linux-64`, `osx-64`, `osx-arm64`, and `win-64`; results are the union of versions found on all available platforms.
>
> ² Conda timestamps come from the `timestamp` field in `repodata.json` (ms since epoch). Most packages carry it; packages without one skip the gate unless you set `deny_missing_timestamp = true` on the rule.
>
> ³ **Path-addressed** type: artifacts are fetched by file path with no per-package version model, so the structural axes show `—`. These types don't enumerate versions but can pre-warm specific files via `cache.warm_paths`, and are gated with a mandatory `path_allow` allowlist. Deb/RPM/Pacman additionally support signed private hosting (`local`/`hybrid`); JetBrains IDE archives and Generic are proxy-only.
>
> Package Explorer upstream ("Not Yet Proxied") search: Go uses pkg.go.dev; PyPI is exact-name lookup; Terraform combines module search with namespace/exact provider lookup. The release proxies (GitHub/Forgejo/GitLab), VS Code Marketplace, Conda, and the path-addressed types have no upstream search API — see the [Package Explorer guide](/guide/package-explorer-search#upstream-search).

## See also

- [User Guide](/guide/user) — task-oriented walkthroughs for the most common registries.
- [Administration → Configuration](/guide/admin-config#configuration) — how to declare registries in `config.toml`.
- [Caching](/guide/caching) · [Access Control](/guide/access-control) · [Roadmap](/guide/roadmap#new-registries).
