---
layout: home

hero:
  image:
    src: /logo.svg
    alt: BatleHub
  name: BatleHub
  text: Smart proxy and cache for package registries
  tagline: Sit between your build tools and the internet. Cache artifacts, enforce access control, and publish private packages — all from one self-hosted server.
  actions:
    - theme: brand
      text: Get Started
      link: /guide/installation
    - theme: alt
      text: View on GitHub
      link: https://github.com/batleforc/batlehub

features:
  - icon: ⚡
    title: Artifact Caching
    details: First download is fetched from upstream and stored locally or in S3. Every subsequent request is served from cache — fast and bandwidth-free.
  - icon: 🔒
    title: Private Registries
    details: Publish private npm packages, Cargo crates, Go modules, and VS Code extensions directly to BatleHub. Use local or hybrid mode per registry.
  - icon: 🛡️
    title: Role-Based Access Control
    details: Per-registry permissions for anonymous, user, and admin roles. Group-based access from OIDC claims or Kubernetes service accounts.
  - icon: ⏱️
    title: Release Age Gate
    details: Block packages published less than N seconds ago. Creates a delay window against supply-chain attacks without blocking known-good versions.
  - icon: 🔀
    title: Multi-Upstream Fanout
    details: List multiple upstreams per registry. A 404 from one automatically falls through to the next — no single point of failure.
  - icon: 📊
    title: OpenTelemetry
    details: Optional distributed tracing via OTLP/gRPC. Works out of the box with Jaeger, Tempo, or any OTLP-compatible backend.
---

## Supported registries

BatleHub proxies six registry types. Every registry type can run as a pure cache (proxy mode), a fully private registry (local mode), or a hybrid of both.

| Registry | Protocol | Default upstream |
|----------|----------|-----------------|
| **GitHub** | Releases, assets, tarballs, raw files | `api.github.com` |
| **npm** | Full packument + tarball proxy | `registry.npmjs.org` |
| **Cargo** | Sparse index + `.crate` download | `crates.io` |
| **OpenVSX** | VS Code extension VSIX | `open-vsx.org` |
| **VS Code Marketplace** | VS Code extension VSIX via Microsoft Gallery API | `marketplace.visualstudio.com` |
| **Go** | GOPROXY protocol (`.info`, `.mod`, `.zip`) | `proxy.golang.org` |

| Feature | GitHub | npm | Cargo | OpenVSX | VS Code | Go |
|---------|:------:|:---:|:-----:|:-------:|:-------:|:--:|
| Version listing | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| Source archive | ✓ | ✓ | ✓ | — | — | ✓ |
| Binary / extension | ✓ | — | — | ✓ | ✓ | — |
| **Private publish** | — | ✓ | ✓ | ✓ | ✓ | ✓ |
| Multi-upstream fanout | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| Release age gate | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| RBAC | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
