# Changelog

All notable changes to BatleHub will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

---

## [Unreleased]

## [1.0.0] - 2026-07-17

First stable release.

### Security

- **SSRF hardening** across registry adapters, including OpenVSX upstream requests
- **Signed-release enforcement** (`RequireSignedReleaseRule`) — optionally require GitHub/OpenVSX/VS Code Marketplace releases to carry verifiable signatures before they're served, with a role-based bypass
- Open-source release housekeeping: `LICENSE` (Apache-2.0), `SECURITY.md`, `CONTRIBUTING.md`

### Reliability

- Fixed the config hot-reload watcher retriggering without a real change; a reload loop that fires more than a few times within 30s without settling now stops and surfaces a warning instead of looping forever
- Large hardening/bug-fix pass across handlers and services following an in-depth code review

### Developer experience

- Frontend lint job added to CI (`front-test.yaml`)
- Dependency upgrades across the Rust workspace (including `sqlx`) and the UI toolchain
- Continued UI rework (routing, navigation) and codebase health cleanup (dead code, duplication)

## [0.5.0] - 2026-06-29

### Registry adapters

- **Arch Linux / Pacman** (`type = "pacman"`) — proxy upstream Arch mirrors **and** private hosting in `local`/`hybrid` mode: `.pkg.tar.{zst,xz,gz}` publish (metadata read from `.PKGINFO`), per-arch `<repo>.db`/`<repo>.files` database regeneration, Ed25519 OpenPGP-signed database (`<repo>.db.sig`) and packages (`.sig` + embedded `%PGPSIG%`) so `SigLevel = Required` works. Signing reuses the hand-rolled Ed25519 signer (the `rsa` crate is banned)

### Vulnerability management

- **OSV vulnerability scanning** — per-registry `cve_gate` rule (`min_severity`, `block`/warn-only, `bypass_roles`); periodic background re-scan via `[vulnerability_scan]` task; findings stored in `artifact_vulnerabilities` DB table; per-version CVE status surfaced in the Package Explorer and admin views
- **Go module vulnerability database proxy** — GOPROXY vuln endpoint (`/proxy/{reg}/goproxy/vuln/`) proxied so `govulncheck` and related tooling can query BatleHub directly without reaching the public database
- **NuGet vulnerability endpoint proxy** — NuGet v3 vulnerability endpoint wired into the service index so `dotnet restore` vulnerability checks flow through the proxy cache
- **Vulnerability scanner extension point** — documented API for adding custom vulnerability scanners (`docs/adding-a-vulnerability-scanner.md`); `docs/vulnerability-proxy.md` covers the proxy-side configuration

### Admin & security

- **User block management** — DB-backed user block list (`028_user_blocks` migration); `UserBlockMiddleware` evaluates the block list before any request handler and returns 403; admin API (`GET/POST/DELETE /api/v1/admin/users/blocks`); Admin Users page in the UI lists OIDC, Kubernetes, and static-token identities with block/unblock actions; fails open on DB errors to avoid locking out admins

### Developer experience

- **Eclipse Che workspace login** — login page detects Eclipse Che environment variables and displays pre-configured connection instructions for workspace-hosted instances
- **CLI download command** (`batlehub-cli download`) — downloads an artifact from any configured registry to a local file; auto-detects registry type and constructs the correct download URL
- **SonarCloud integration** — `.github/workflows/sonar.yaml` runs frontend (Vitest LCOV) and backend (cargo-llvm-cov LCOV, with Postgres/MinIO/Redis services) coverage and uploads both reports to SonarCloud on every push to `main`

### Bug fixes

- JetBrains artifact post-copy path handling corrected; improved `docs/path-mapper.md` to clarify URL routing for large IDE archives
- TOCTOU race condition fixes and general code-review hardening across several handler paths
- Correct handling of unreachable match arms and unused assigned values flagged by the compiler

### Code quality

- Code duplication reduced below 5% (tracked via SonarCloud)
- Container image updated to TiKV-based build; `Containerfile` and `Containerfile.hardened` both updated

---

## [0.2.0] - 2026-06-14

### Registry adapters

- **npm** — proxy with scoped package support; local/hybrid publish
- **Cargo** — sparse index proxy compatible with `cargo` sparse protocol; local/hybrid publish
- **GitHub Releases** — artifact download proxy for GitHub release assets
- **OpenVSX** — VS Code extension proxy for the open-source marketplace
- **VS Code Marketplace** — VSIX download proxy for the official marketplace
- **Go modules (GOPROXY)** — Go module proxy protocol (`$GOPROXY`); multi-segment module path routing via `{module:[^@]+}` pattern
- **Maven / Gradle** — Maven Central-compatible metadata XML + JAR / POM downloads; private publishing via `mvn deploy` (three-phase POM + JAR + checksum upload); dynamically generated `maven-metadata.xml` from DB; local/hybrid mode
- **Terraform** — provider and module proxy protocol; private module (tar.gz + `X-Terraform-Get` redirect) and provider (version manifest + per-platform binary) publishing; local/hybrid mode
- **RubyGems** — gem download and version listing; local/hybrid mode with yank / unyank
- **Composer** — Packagist v2 protocol (`packages.json`, p2 metadata, dist downloads); private package ZIP upload; local/hybrid mode
- **PyPI** — Simple API proxy with URL rewriting; private wheel / sdist publishing via `twine`; Simple API served from DB; local/hybrid mode
- **Conda / Anaconda** — `repodata.json` proxy and channel merging; `.tar.bz2` and `.conda` package parsing; private channel publishing; local/hybrid mode
- **NuGet** — NuGet v3 service index + flat container proxy; `.nupkg` and `.nuspec` downloads; private publishing via `dotnet nuget push`; `X-NuGet-ApiKey` normalised to `Authorization: Bearer`; local/hybrid mode

### Authentication

- **Static tokens** — plain-text Bearer tokens and Argon2id PHC hashes in `config.toml`; `batlehub hash-token <token>` CLI helper
- **OIDC** — JWT validation via OIDC discovery + JWKS; browser SSO (Authorization Code flow); role and group mapping from claims; namespaced group prefixes for multi-provider setups
- **Kubernetes service accounts** — TokenReview API validation; role and group mapping; in-cluster defaults
- **GitHub / Forgejo Actions OIDC** (`type = "actions-oidc"`) — short-lived JWT validation for workflow jobs; claim-to-group mapping (`repository`, `ref`, `environment`, `actor`, …) with static and dynamic templates; glob and regex pattern matching; AND / OR condition logic per rule

### Access control & policy

- **RBAC engine** — role/group rules per registry with `pull` / `push` / `admin` actions; evaluated by `RbacRule`
- **Built-in policy rules** — `DenyLatestRule` (block floating `latest` tags), `BlockListRule` (explicit package/version deny list), `ReleaseAgeGateRule` (reject versions younger than a configured age)
- **Rate limiting** — per-user and per-registry token-bucket rate limits; per-group shared pools; hard block or soft warn enforcement; `Retry-After` and `X-RateLimit-*` response headers; state resets on restart
- **IP blocking** — fail2ban-style blocking via `IpBlockStore`; configurable block duration and thresholds; outermost `actix_web::middleware::Condition` middleware
- **Publish quota** — per-user, per-group, and per-registry quotas on storage usage and package count; `X-Quota-*` response headers; admin API for viewing and resetting quotas; enforcement policies: block or warn

### Cache

- **Cache-Control honouring** — respects `no-cache`, `max-age`, and `no-store` from upstream responses
- **Eviction policies** — TTL-based expiry, "not accessed for N days", garbage-collect all versions except the latest N, storage-size cap with LRU eviction
- **Content-addressable deduplication** — identical artifact bytes stored once; ref-counted via `artifact_dedup_index` / `artifact_dedup_refs`; backwards-compatible with pre-dedup artifacts
- **Proactive cache warming** — pre-fetch known versions on startup and on demand via `POST /api/v1/admin/registries/{registry}/warm`; configurable `warm_packages`, `warm_latest_n`, `warm_concurrency`
- **Explore cache** — 10-minute in-memory cache for the explore list and stats; stale-on-DB-error fallback; admin invalidation via `POST /api/v1/admin/explore/invalidate`; auto-invalidated on local publish

### Private registry features

- Local and hybrid operating modes for all supported registry types
- **Ownership management** — per-package owner table (user/group; admin/maintainer roles); `initialize_owner` on first publish; `can_publish` check on subsequent publishes; admin API to list / add / remove owners
- **Versioning policies** — `enforce_semver`, `allow_prerelease`, `version_pattern` (regex) per registry; enforced at publish time with HTTP 422
- **Beta / pre-release channel** — per-registry allow-list of users or groups who may access unpublished versions (`BetaChannelPort`, DB-backed)
- **Artifact signing** — `X-Artifact-Signature` / `X-Signature-Type` headers at publish; signature stored in DB and returned on download; optional `signing.required` enforcement
- **Bulk operations** — `POST /api/v1/admin/registries/{registry}/bulk-yank|bulk-unyank|bulk-delete`

### SBOM

- Per-artifact SPDX 2.3 and CycloneDX 1.4 generation at proxy time and at publish time; archive manifest extraction (Cargo.toml, package.json, pom.xml, go.mod, requirements.txt, …)
- Upstream SBOM fetch from GitHub dependency graph API and npm `bom.json`
- Org-level SBOM export — all artifacts served in a time range as a single merged document (`GET /api/v1/sbom/export?from=…&to=…&format=spdx|cyclonedx`); admin UI at `/admin/sbom`
- `required = true` policy option in `[registries.sbom]` — deny publishing a private package when no manifest is found in the archive
- Per-artifact SBOM download buttons (SPDX and CycloneDX) in the Package Explorer version detail view

### Hot reload & dynamic config

- `HotConfig` behind `Arc<RwLock<HotConfig>>` — in-flight requests finish with the old snapshot; config swap is atomic
- File watcher (`notify` crate) — loads a pending reload; admin confirms via `POST /api/v1/admin/config/pending/apply` or discards with `DELETE /api/v1/admin/config/pending`
- Schema validation and upstream connectivity probes before storing a pending reload
- Config audit trail — every reload is recorded in `config_changes` table; retrievable via `GET /api/v1/admin/config/changes`
- **Global admin banner** — broadcast info / warning / error to all visitors; backed by in-memory, Redis, or PostgreSQL; `PUT/DELETE /api/v1/admin/banner`
- `BATLEHUB_DISABLE_HOT_RELOAD=1` env var — disables the file watcher and all reload endpoints (for read-only Kubernetes ConfigMap mounts)

### Webhooks & notifications

- Outbound notification channels: email (via `lettre`), Slack, Microsoft Teams, HTTP webhooks
- DB-backed subscriptions — subscribe to events per package, version, or registry (new version published, version deprecated, package removed)
- Fire-and-forget dispatch integrated into all publish and yank handlers
- Inbound webhook receiver — external systems (CI pipelines, security scanners) can push events into BatleHub

### Observability

- Prometheus metrics endpoint (`/metrics`) — request counts, cache hit/miss rates, latency percentiles, error rates per registry
- Health check endpoint (`/healthz`) — verifies connectivity to the database and all configured storage backends
- Stats dashboard on the admin home screen — hits/misses, bandwidth saved, per-registry and aggregate

### CLI (`batlehub-cli`)

- Full command tree: `registry list|info`, `package list|versions`, `version yank|unyank|delete`, `owners list|add|remove`, `publish`, `auth whoami|login|refresh`, `token list|create|revoke`, `admin`, `config init|show|set`, `completion`, `hash-token`
- `batlehub-cli publish <file>` — auto-detects registry type, package name, and version from the artifact (`detect_meta`); supports all local/hybrid registry types
- `batlehub-cli auth login` — OIDC Authorization Code browser flow with token caching; Kubernetes token path support; auto-refresh on startup
- Shell completion for bash, zsh, fish, and others via `batlehub-cli completion`
- Named profile config at `~/.config/batlehub/config.toml`; global flags `--profile`, `--server`, `--token`, `--registry`, `--json` and `BATLEHUB_*` env-var equivalents
- **TUI mode** (`batlehub-cli tui`) — ratatui / crossterm terminal UI with: registry list, package explorer with live search/filter, package detail (yank / unyank keybindings), publish form, setup wizard (scans local manifests and shows per-type config snippets + publish commands), login screen (OIDC / Kubernetes / static token)

### UI (Vue 3 SPA)

- **Package Explorer** (`/explore`) — collapsible registry catalog sidebar; search and sort across cached and upstream packages; per-package detail page with version history and gate/firewall status per version; independent search permissions via `[registries.rbac.explore]`
- **Setup Guide** — API-driven; tabs appear only for registry types configured on the server; per-type config snippets and client commands defined in `ui/src/config/registryTypes.ts`
- **Monofolio design system** — OKLCH colour tokens, 2 px sharp corners, crimson + copper palette, JetBrains Mono font, cyber-grid background, `text-copper` utility class
- **Admin pages** — config reload (pending/apply flow, audit log), global banner editor, SBOM org export, webhook / notification subscription management

### Infrastructure

- Helm chart for Kubernetes deployment (`helm/`)
- Hardened OCI container image (`Containerfile.hardened`) with minimal attack surface
- Forgejo CI/CD workflows — lint (`cargo clippy -D warnings`), format check, tests, ≥ 80% line coverage, container build
- `sqlx-macros` and `sqlx-mysql` patched to empty stubs in `[patch.crates-io]` to remove the `rsa` crate (RUSTSEC-2023-0071)
- `aws-sdk-s3` and `aws-config` with `default-features = false` to avoid `legacy-rustls-ring` (RUSTSEC-2026-0098 / 0099 / 0104)
- Fuzz targets for RBAC evaluation, cache key generation, deny-latest rule, and release age gate (`task fuzz`)

---

[Unreleased]: https://git.batleforc.fr/batleforc/batlehub/compare/v0.5.0...HEAD
[0.5.0]: https://git.batleforc.fr/batleforc/batlehub/compare/v0.2.0...v0.5.0
[0.2.0]: https://git.batleforc.fr/batleforc/batlehub/releases/tag/v0.2.0
