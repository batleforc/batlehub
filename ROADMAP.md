# BatleHub — Roadmap

Planned features and improvements, grouped by theme. Within each group the order reflects rough implementation priority.

For discussion or to propose a feature, open an issue on the [project repository](https://git.batleforc.fr/batleforc/batlehub).

---

## New registry types

Current adapters: npm, Cargo, GitHub, Forgejo/Gitea, GitLab, OpenVSX, VS Code Marketplace, Go modules, Maven, RubyGems, Terraform, Composer, PyPI, Conda, NuGet, Deb (APT), RPM (YUM/DNF), JetBrains (IDE archives).

- [x] **PyPI** — Python package index; Simple API proxy with URL rewriting; wheel / sdist downloads; private publishing via `twine` in `local`/`hybrid` mode
- [x] **Maven / Gradle** — Maven Central-compatible metadata XML + JAR / POM downloads; private publishing via `mvn deploy` in `local`/`hybrid` mode
- [x] **RubyGems** — gem downloads and version listing (proxy + local/hybrid with publish/yank/unyank)
- [x] **NuGet** — .NET package protocol; NuGet v3 service index + flat container proxy; `.nupkg` and `.nuspec` downloads; private publishing via `dotnet nuget push` in `local`/`hybrid` mode
- [x] **Deb / RPM** — Debian APT (`type = "deb"`) and Red Hat YUM/DNF (`type = "rpm"`) repository proxying **and** private hosting in `local`/`hybrid` mode: `.deb`/`.rpm` publish, `Packages`/`Release` and `repodata/` regeneration, Ed25519 OpenPGP-signed metadata (`InRelease`/`Release.gpg`, `repomd.xml.asc`). Signing is hand-rolled (Ed25519 only) to avoid the banned `rsa` crate
- [x] **JetBrains IDE archives** — `type = "jetbrains"`: proxy-only path-based cache for JetBrains IDE installer archives (default upstream `download.jetbrains.com`); reuses the generic `PathProxyRegistryClient`. No private hosting. IDE archives are large (~1-1.7 GB), so `limits.max_artifact_size_bytes` must be raised
- [x] **Terraform registry** — provider and module proxy protocol; private module + provider publishing in `local`/`hybrid` mode
- [x] **GitLab releases and packages** — `type = "gitlab"`: paginated release list/tag, link assets, source archives + raw files via the `/-/` URL scheme, nested groups, `PRIVATE-TOKEN`/Bearer auth; package registry passthrough (`/api/v4/…`, ideal for generic packages). Ecosystem package registries (npm/Maven/PyPI/…) are proxied via the matching typed adapter pointed at the GitLab package endpoint
- [x] **Forgejo releases and packages** — `type = "forgejo"`: paginated Gitea/Forgejo `/api/v1` release list/tag, assets, source archives, raw files (reuses the GitHub URL scheme); package registry passthrough (`/api/packages/…`). Ecosystem registries via the matching typed adapter
- [x] **Composer** — PHP Composer registry (Packagist v2 protocol — `packages.json`, p2 metadata, dist downloads); private package publishing via ZIP upload in `local`/`hybrid` mode
- [x] **Anaconda / Conda** — Python data science package registry; `repodata.json` proxy and channel merging; `.tar.bz2` and `.conda` package parsing; private channel publishing in `local`/`hybrid` mode
- [x] **Arch Linux / Pacman** — `type = "pacman"`: proxy upstream Arch mirrors **and** private hosting in `local`/`hybrid` mode: `.pkg.tar.{zst,xz,gz}` publish (metadata read from `.PKGINFO`), per-arch `<repo>.db`/`<repo>.files` database regeneration, Ed25519 OpenPGP-signed database (`<repo>.db.sig`) and packages (`.sig` + embedded `%PGPSIG%`) so `SigLevel = Required` works. Signing reuses the hand-rolled Ed25519 signer (the `rsa` crate is banned)

> **Not planned:** Docker / OCI artifacts. [Harbor](https://goharbor.io) solves this better than we could, unless concrete demand arises.

---

## Cache policy

- [x] Honour `Cache-Control` headers from upstream responses (`no-cache`, `max-age`, `no-store`) to decide whether and how long to cache
- [x] Eviction policies: TTL-based expiry, "not accessed for N days", garbage-collect all versions except the latest N, storage size cap with LRU eviction
- [x] Cache index coherence: compare what is actually in the storage backend against what the registry metadata expects, and recover from corruption or manual deletions
- [x] Content-addressable deduplication: identical artifact bytes are stored once regardless of how many logical keys (registries, package names) reference them — ref-counted via `artifact_dedup_index` / `artifact_dedup_refs`, backwards-compatible with pre-dedup artifacts
- [x] Proactive cache warming: pre-fetch known versions of configured packages on startup and on demand via the admin API (`POST /api/v1/admin/registries/{registry}/warm`); configurable per registry with `warm_packages`, `warm_latest_n`, and `warm_concurrency`

---

## Metrics & observability

- [x] Prometheus metrics endpoint (`/metrics`): request counts, cache hit/miss rates, latency percentiles, error rates per registry
- [x] Health check endpoint (`/healthz`) that verifies connectivity to the database and all configured storage backends
- [x] Stats dashboard on the admin home screen: hits/misses, bandwidth saved, per-registry and aggregate

---

## Artifact integrity & security

- [x] Verify checksums for downloaded artifacts when the upstream provides them — per-registry `[registries.integrity]`: on the proxy fetch path the buffered bytes are hashed and compared against the metadata checksum (Cargo SHA-256, npm SRI/`shasum`, PyPI SHA-256). Supports SRI (`sha512-…`) and bare hex (algorithm inferred from length)
- [x] Block serving an artifact if its integrity check fails, or optionally if the upstream provides no integrity metadata at all — a mismatch fails the download with `502` and is never cached (not bypassable); `require_metadata = true` additionally blocks downloads with no advertised checksum (with `bypass_roles`)
- [ ] Sigstore / npm provenance verification for npm packages
- [ ] `cargo verify-project`-style verification for Cargo crates
- [ ] Detect and optionally require signed releases for GitHub, OpenVSX, and VS Code Marketplace
- [ ] Allowlist of trusted publishers (GitHub users / orgs, npm scopes, Cargo crate owners)
- [ ] Allowlist of approved versions; blocklist of specific versions with known issues
- [x] Vulnerability scanning via the [OSV API](https://osv.dev) to block or warn about packages with known CVEs — periodic SBOM re-scan plus a per-registry `cve_gate` rule (`min_severity`, `block`/warn-only, `bypass_roles`)
- [ ] YARA rule evaluation for custom malware or policy patterns
- [ ] Antivirus scanning for binary artifacts (VSIX, Go module zips) via a configurable external REST API
- [ ] Warn when an upstream registry is returning high error rates or slow responses and cached data may be stale
- [x] Warn when a registry does not provide integrity metadata for its artifacts — the proxy logs a warning and increments `batlehub_integrity_checks_total{outcome="missing"}` when an artifact is fetched with no advertised checksum (and blocks instead when `require_metadata = true`)

---

## Authentication providers

- [x] **Static tokens** — plain-text and Argon2id-hashed Bearer tokens in `config.toml`; `batlehub hash-token` CLI
- [x] **OIDC** — JWT validation via OIDC discovery + JWKS; browser SSO (Authorization Code flow); role and group mapping from claims; namespaced group prefixes for multi-provider setups
- [x] **Kubernetes service accounts** — TokenReview API validation; role and group mapping; in-cluster defaults
- [x] **GitHub / Forgejo Actions OIDC** (`type = "actions-oidc"`) — validate short-lived JWTs issued to workflow jobs; map claims (`repository`, `ref`, `environment`, `actor`, …) to groups and roles via configurable rules; supports static group names and dynamic group templates (e.g. `"{name}/{repository}/{ref_name}"` → `"forgejo-action/batleforc-batlehub/main"`); glob and regex pattern matching; AND / OR condition logic per rule

---

## Rate limiting & DoS protection

- [x] Per-user and per-registry rate limits on API requests and artifact downloads, with configurable thresholds and time windows (in-memory token bucket; state resets on restart)
- [x] Configurable enforcement policies: hard block vs. soft warn when a limit is reached
- [x] Explicit rate-limit warnings in API responses (`Retry-After`, `X-RateLimit-*` headers)
- [x] Per-group rate limits (shared token-bucket pools per OIDC/Kubernetes group; enforcement override per group)
- [x] IP-based blocking for abusive clients, with configurable block duration and thresholds
- [ ] Integration with external IP reputation services to automatically block known malicious IPs

---

## Quota management

- [x] Per-user, per-group, and per-registry quotas on storage usage and number of published packages
- [x] Enforcement policies: block publish requests that exceed the quota, or allow with an explicit warning
- [x] Quota warnings in API responses and admin UI when a limit is being approached
- [x] Admin API for resetting quotas for specific users, groups, or registries

---

## Hot reloading & dynamic config

- [x] Watch the config file for changes and prompt an admin for confirmation before applying — file watcher (using `notify` crate) loads a pending reload; admin confirms via `POST /api/v1/admin/config/pending/apply` or discards it
- [x] Validate the new config before applying it (schema check + connectivity probes) to avoid breaking a running server — schema validation runs immediately; connectivity probes (`HEAD` to each upstream with a 5s timeout) run before the pending reload is stored
- [x] Partial reloads: update RBAC rules or add/remove a registry without restarting the process — registries, policies, versioning, signing, and beta-channel maps are all behind `Arc<RwLock<HotConfig>>`; in-flight requests finish with the old data before the swap
- [x] API endpoint for triggering a config reload (`POST /api/v1/admin/config/reload`) for automation when file-watching is unavailable — also `GET /api/v1/admin/config/pending`, `POST /api/v1/admin/config/pending/apply`, `DELETE /api/v1/admin/config/pending`
- [x] Audit trail for all config changes (who triggered, what changed, when) — stored in `config_changes` table; retrievable via `GET /api/v1/admin/config/changes`
- [x] **Global admin banner** — broadcast a message (info / warning / error) to all website visitors; automatically set during a reload and cleared on completion; backed by in-memory, Redis, or PostgreSQL depending on the cache backend; `PUT/DELETE /api/v1/admin/banner` + `/admin/config-reload` UI page
- [x] `BATLEHUB_DISABLE_HOT_RELOAD=1` — disable the file watcher and all reload endpoints (e.g. when config is a read-only Kubernetes ConfigMap)
- [ ] Dynamic blocking rules fetched from an external trusted source (e.g. a signed Git repository); verify signatures before applying
- [ ] Dynamic allowlists of trusted publishers or approved versions, fetched from an external source and merged into RBAC / block rules automatically

---

## Webhooks & notifications

- [x] Subscribe to notifications for specific packages, versions, or registries (new version published, version deprecated, package removed)
- [x] Multiple notification channels: email, Slack, Microsoft Teams, outbound webhooks
- [x] User-configurable notification preferences and channel configuration in the UI
- [x] Inbound webhook API so external systems (CI pipelines, security scanners) can push events into BatleHub and trigger notifications or policy updates

---

## Private registry features

Applies to registries running in `local` or `hybrid` mode.

### Per-registry additions

- **npm** — versioning policies (enforce semantic versioning, allowlist version patterns)
- **Cargo** — versioning policies; verify full compatibility with the yank protocol from crates.io
- **VS Code extensions** — deprecation and unlisting; upload via the UI (form for VSIX + metadata), in addition to the existing `PUT` API
- [x] **Maven** — private artifact publishing via `mvn deploy`; POM-triggered three-phase publish; JAR/checksum pre-upload; dynamically generated `maven-metadata.xml` from DB; `local` and `hybrid` modes
- [x] **Terraform** — private module publishing (tar.gz upload, `X-Terraform-Get` redirect); private provider publishing (version manifest + per-platform binary upload); `local` and `hybrid` modes
- [x] **PyPI** — private wheel / sdist publishing via `twine`; Simple API served from DB; `local` and `hybrid` modes
- [x] **Conda** — private channel with `repodata.json` generation from DB; `.tar.bz2` and `.conda` package upload; `local` and `hybrid` modes

### For all private registry types

- [x] Artifact signing framework: publish with `X-Artifact-Signature` / `X-Signature-Type` headers; signature stored in DB and returned on download; optional `signing.required` enforcement. Optional download-time verification of stored `ed25519` signatures against configured `signing.trusted_keys` (`signing.verify_on_download`); Ed25519 only, since the `rsa` crate is banned
- [x] Ownership and team management: per-package owner table (user/group, admin/maintainer roles); `initialize_owner` on first publish; `can_publish` check on subsequent publishes; admin API to list/add/remove owners
- [x] Versioning policies: `enforce_semver`, `allow_prerelease`, `version_pattern` (regex) per registry; enforced at publish time with HTTP 422
- [x] Beta / pre-release channel: allow specific users or groups to access unpublished versions before general release
- [x] Bulk operations: `POST /api/v1/admin/registries/{registry}/bulk-yank|bulk-unyank|bulk-delete`
- [x] Content-addressable deduplication for stored artifacts (ref-counted via `artifact_dedup_index` / `artifact_dedup_refs`)
- [x] Integrity verification: verify checksums on re-serve, not only at publish time — `integrity.verify_on_serve` re-hashes stored bytes against a self-computed SHA-256 (recorded when first cached) on every serve (proxy cache hits and local reads); a mismatch fails with `502` and evicts the corrupt entry

### CLI tool - `batlehub-cli`

- [x] a CLI for common private registry tasks (`publish`, `yank`, `list`), suitable for use in CI pipelines — `batlehub-cli` binary in `cli/`; global flags `--profile`, `--server`, `--token`, `--registry`, `--json` and env-var equivalents (`BATLEHUB_*`)
  - [x] Publish command that wraps the upload API, with support for multiple registry types and automatic metadata extraction from the artifact (e.g. extension, archive contents, manifest files) — `batlehub-cli publish <file>`; `detect_meta` auto-detects registry type, name, and version
  - [x] Version management commands for yanking, unyanking, or deleting specific versions — `batlehub-cli version yank|unyank|delete`
  - [x] Package management commands for listing versions, viewing metadata, or managing owners — `batlehub-cli package list|versions` and `batlehub-cli owners list|add|remove`
  - [x] Authentication support for static tokens and token management — `batlehub-cli auth whoami` and `token list|create|revoke`; token passed via `--token` / `BATLEHUB_TOKEN`
  - [x] List of available registries and their types, with per-registry configuration details — `batlehub-cli registry list|info`
  - [x] List packages and versions in a registry, with filtering options — `batlehub-cli package list|versions`
  - [x] Autocompletion support for shell integration — `batlehub-cli completion bash|zsh|fish|...` generates and prints the completion script; pipe to shell RC file
  - [x] Config file support for storing credentials and default options, with CLI overrides — `~/.config/batlehub/config.toml` with named profiles; `batlehub-cli config init|show|set`
  - [x] Config file output for both CI automation and human use — `config init` interactive wizard; `--json` flag on all commands for machine-readable output
- [x] A TUI mode for interactive use — `batlehub-cli tui` launches a `ratatui` / `crossterm` terminal UI
  - [x] List of registries with search and filter capabilities — `registry_list` screen
  - [x] Per-registry package explorer with version details and management actions — `package_list` screen (live search/filter) + `package_detail` screen (yank / unyank keybindings)
  - [x] Interactive prompts for publishing new versions — `publish_form` screen with auto-detected name and version fields
  - [x] Help setup registry for a current project by scanning local files — TUI `SetupWizard` screen (`s` from registry list); detects Cargo.toml, go.mod, package.json, pyproject.toml, pom.xml, composer.json, *.gemspec, *.nuspec, *.csproj, *.tf, environment.yml; shows per-type config snippets and publish commands
  - [x] Auth workflow integration for OIDC and Kubernetes service accounts, including token caching and refresh — `batlehub-cli auth login` (OIDC browser flow + K8s token path); `auth refresh`; `oidc_refresh_token` / `oidc_expires_at` / `kubernetes_token_path` persisted in profile; auto-refresh on startup; TUI `Login` screen (`L` from registry list) with three-tab method selector

---

## SBOM support

- [x] Proxy existing SBOMs from upstreams that provide them (GitHub dependency graph API, npm `bom.json`) — enabled by `fetch_upstream = true` in `[registries.sbom]`
- [x] Generate a minimal per-artifact SBOM (SPDX 2.3 or CycloneDX 1.4) at proxy time, from registry metadata and the downloaded archive
- [x] Org-level SBOM export: all artifacts served in a time range as a single merged document (`GET /api/v1/sbom/export?from=…&to=…&format=spdx|cyclonedx`) — admin UI at `/admin/sbom`
- [x] Generate SBOMs at upload time for private registries, extracting dependency manifests from the archive (`go.mod`, `Cargo.toml`, `package.json`, `pom.xml`, `requirements.txt`)
- [x] Policy option: deny publishing a private package if no manifest is found in the archive (`required = true` in `[registries.sbom]`)
- [x] Per-artifact SBOM accessible from the Package Explorer version detail view (SPDX and CycloneDX download buttons per version)
- [x] Periodically re-check cached SBOMs against vulnerability databases (see [Artifact integrity](#artifact-integrity--security)) and update block / warn metadata automatically — `[vulnerability_scan]` background task queries OSV and records findings, surfaced per-version in the Package Explorer and admin views

---

## UI improvements

- [x] **Package explorer** (`/explore`) — collapsible catalog with registry sidebar; search and sort across all cached and upstream packages; per-package detail page showing version history with gate/firewall status per version; `[registries.rbac.explore]` config block for independent search permissions
- [ ] Package explorer caching and pagination for large registries (e.g. npm) to avoid fetching the entire index on every request; cache invalidation on new versions published or cache expiry
- [ ] Package detail pages with full metadata, version history, and download links (full deep-linking beyond the explorer summary)
- [ ] User listing and block management in the admin panel (OIDC and Kubernetes-sourced identities, not just static tokens)
- [ ] Config editor with validation, diff preview, and apply button (integrates with hot reload)
  - [ ] Read-only warning when the config file is mounted from a Kubernetes ConfigMap, with instructions for applying changes externally

---

## Testing

- [~] Unit tests for all registry adapters and policy evaluation logic — significant coverage added (entities, services, auth, storage router, registry adapters, web middleware, handler guards); ≥80% line coverage enforced by `task coverage-check`
- [x] CLI test suite — 23 unit tests (`parse_oidc_paste`, `is_token_expiring_soon`, `detect_project_types` for all 9 manifest types) + 16 integration tests (registry, package, version yank/unyank/delete, publish, auth, shell completion, Kubernetes login); fixed `InMemoryLocalRegistry` case-sensitivity bug so yank/delete tests pass end-to-end
- [ ] Integration tests against real upstream registries (gated, opt-in)
- [ ] Broader fuzzing targets beyond the current four (RBAC, cache key, deny-latest, release age)
- [ ] Cover code with [Sonarqube](https://sonarcloud.io/project/overview?id=batleforc_batlehub)
