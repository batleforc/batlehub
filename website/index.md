---
layout: home

hero:
  image:
    src: /logo.svg
    alt: BatleHub
  name: BatleHub
  text: Your package hub. Proxy, cache, and host.
  tagline: Sit between your build tools and the internet. Cache artifacts, enforce access control, and publish private packages — all from one self-hosted server.
  actions:
    - theme: brand
      text: Get Started
      link: /guide/installation
    - theme: alt
      text: View on Forgejo
      link: https://git.batle.dev/batlehub/batlehub

features:
  - icon: ⚡
    title: Artifact Caching
    details: First download is fetched from upstream and stored locally or in S3. Every subsequent request is served from cache — fast and bandwidth-free.
  - icon: 🔒
    title: Private Registries
    details: Publish private npm packages, Cargo crates, Go modules, and VS Code extensions directly to BatleHub. Use local or hybrid mode per registry.
  - icon: 🛡️
    title: Role-Based Access Control
    details: Per-registry permissions for anonymous, user, and admin roles. Group-based access from OIDC claims, Kubernetes service accounts, or GitHub/Forgejo Actions OIDC tokens.
  - icon: 🤖
    title: Actions OIDC Auth
    details: Validate GitHub and Forgejo workflow JWTs without long-lived secrets. Map any claim — repo, branch, environment — to groups and roles via glob/regex rules. Dynamic group names like "{name}/{repository}/{ref_name}" enable wildcard RBAC grants across all CI jobs.
  - icon: ⏱️
    title: Release Age Gate
    details: Block packages published less than N seconds ago. Creates a delay window against supply-chain attacks without blocking known-good versions.
  - icon: 🔀
    title: Multi-Upstream Fanout
    details: List multiple upstreams per registry. A 404 from one automatically falls through to the next — no single point of failure.
  - icon: 🚦
    title: Distributed Rate Limiting
    details: Fixed-window per-user and per-group rate limits. Back counters with InMemory, PostgreSQL, or Redis — shared limits survive restarts and scale across replicas.
  - icon: 📊
    title: OpenTelemetry
    details: Optional distributed tracing via OTLP/gRPC. Works out of the box with Jaeger, Tempo, or any OTLP-compatible backend.
  - icon: 🔥
    title: Cache Warming & Eviction
    details: Pre-fetch packages at startup to eliminate cold-start latency. Evict by TTL, idle time, version count, or storage size cap — mix and match per registry.
  - icon: 🧪
    title: Beta/Pre-Release Channel
    details: Gate pre-release versions (e.g. 1.0.0-beta.1) to approved users or groups. Non-members see only stable versions — no separate publish step needed.
  - icon: 🚫
    title: IP-Based Blocking
    details: Fail2ban-style auto-blocking. IPs that exceed a violation threshold (rate-limit hits, auth failures) are blocked automatically. Manual ban/unban via admin API.
  - icon: 🗄️
    title: Storage Deduplication
    details: Identical artifact bytes are stored once, regardless of how many registries or package names reference them. Ref-counted and backwards-compatible.
  - icon: 🔑
    title: Hashed Static Tokens
    details: Store Argon2id PHC hashes in config instead of raw token strings. Run `batlehub hash-token <value>` to generate a hash. Plain-text tokens keep working — both formats coexist.
---

## Supported registries

BatleHub proxies ten registry types. Every registry type can run as a pure cache (proxy mode), a fully private registry (local mode), or a hybrid of both.

| Registry | Protocol | Default upstream |
|----------|----------|-----------------|
| **GitHub** | Releases, assets, tarballs, raw files | `api.github.com` |
| **npm** | Full packument + tarball proxy | `registry.npmjs.org` |
| **Cargo** | Sparse index + `.crate` download | `crates.io` |
| **OpenVSX** | VS Code extension VSIX | `open-vsx.org` |
| **VS Code Marketplace** | VS Code extension VSIX via Microsoft Gallery API | `marketplace.visualstudio.com` |
| **Go** | GOPROXY protocol (`.info`, `.mod`, `.zip`) | `proxy.golang.org` |
| **Maven** | Maven Central-compatible metadata XML + JAR / POM downloads | `repo1.maven.org` |
| **Terraform** | Provider and module proxy protocol (v1 API) | `registry.terraform.io` |
| **RubyGems** | Gem downloads, version listing, REST info API | `rubygems.org` |
| **Composer** | Packagist v2 protocol (`packages.json`, p2 metadata, dist downloads) | `repo.packagist.org` |

| Feature | GitHub | npm | Cargo | OpenVSX | VS Code | Go | Maven | Terraform | RubyGems | Composer |
|---------|:------:|:---:|:-----:|:-------:|:-------:|:--:|:-----:|:---------:|:--------:|:--------:|
| Version listing | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| Source archive | ✓ | ✓ | ✓ | — | — | ✓ | ✓ | ✓ | ✓ | ✓ |
| Binary / extension | ✓ | — | — | ✓ | ✓ | — | ✓ | ✓ | — | — |
| **Private publish** | — | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| Multi-upstream fanout | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| Release age gate | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| RBAC | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| Cache warming (version enumeration) | — | ✓ | ✓ | ✓ | — | ✓ | — | — | — | — |
