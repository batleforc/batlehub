# BatleHub — Roadmap

Planned features and improvements, grouped by theme. Within each group the order reflects rough implementation priority.

For discussion or to propose a feature, open an issue on the [project repository](https://git.batleforc.fr/batleforc/batlehub).

---

## New registry types

Current adapters: npm, Cargo, GitHub, OpenVSX, VS Code Marketplace, Go modules, Maven, RubyGems, Terraform, Composer, PyPI, Conda, NuGet.

- [x] **PyPI** — Python package index; Simple API proxy with URL rewriting; wheel / sdist downloads; private publishing via `twine` in `local`/`hybrid` mode
- [x] **Maven / Gradle** — Maven Central-compatible metadata XML + JAR / POM downloads; private publishing via `mvn deploy` in `local`/`hybrid` mode
- [x] **RubyGems** — gem downloads and version listing (proxy + local/hybrid with publish/yank/unyank)
- [x] **NuGet** — .NET package protocol; NuGet v3 service index + flat container proxy; `.nupkg` and `.nuspec` downloads; private publishing via `dotnet nuget push` in `local`/`hybrid` mode
- [ ] **Deb / RPM** — Debian APT and Red Hat YUM repository proxying
- [x] **Terraform registry** — provider and module proxy protocol; private module + provider publishing in `local`/`hybrid` mode
- [ ] **GitLab releases and packages** — similar to GitHub but with different auth and pagination
- [ ] **Forgejo releases and packages** — Gitea fork with minor API differences
- [x] **Composer** — PHP Composer registry (Packagist v2 protocol — `packages.json`, p2 metadata, dist downloads); private package publishing via ZIP upload in `local`/`hybrid` mode
- [x] **Anaconda / Conda** — Python data science package registry; `repodata.json` proxy and channel merging; `.tar.bz2` and `.conda` package parsing; private channel publishing in `local`/`hybrid` mode

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

- [ ] Verify checksums for downloaded artifacts when the upstream provides them (e.g. Cargo's sparse index includes SHA-256 per version)
- [ ] Block serving an artifact if its integrity check fails, or optionally if the upstream provides no integrity metadata at all
- [ ] Sigstore / npm provenance verification for npm packages
- [ ] `cargo verify-project`-style verification for Cargo crates
- [ ] Detect and optionally require signed releases for GitHub, OpenVSX, and VS Code Marketplace
- [ ] Allowlist of trusted publishers (GitHub users / orgs, npm scopes, Cargo crate owners)
- [ ] Allowlist of approved versions; blocklist of specific versions with known issues
- [ ] Vulnerability scanning via the [OSV API](https://osv.dev) to block or warn about packages with known CVEs
- [ ] YARA rule evaluation for custom malware or policy patterns
- [ ] Antivirus scanning for binary artifacts (VSIX, Go module zips) via a configurable external REST API
- [ ] Warn when an upstream registry is returning high error rates or slow responses and cached data may be stale
- [ ] Warn when a registry does not provide integrity metadata for its artifacts

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

- [x] Artifact signing framework: publish with `X-Artifact-Signature` / `X-Signature-Type` headers; signature stored in DB and returned on download; optional `signing.required` enforcement
- [x] Ownership and team management: per-package owner table (user/group, admin/maintainer roles); `initialize_owner` on first publish; `can_publish` check on subsequent publishes; admin API to list/add/remove owners
- [x] Versioning policies: `enforce_semver`, `allow_prerelease`, `version_pattern` (regex) per registry; enforced at publish time with HTTP 422
- [x] Beta / pre-release channel: allow specific users or groups to access unpublished versions before general release
- [x] Bulk operations: `POST /api/v1/admin/registries/{registry}/bulk-yank|bulk-unyank|bulk-delete`
- [x] Content-addressable deduplication for stored artifacts (ref-counted via `artifact_dedup_index` / `artifact_dedup_refs`)
- [ ] Integrity verification: verify checksums on re-serve, not only at publish time

### CLI tool

- [ ] `batlehub-cli` — a standalone CLI for common private registry tasks (`publish`, `deprecate`, `yank`, `list`), suitable for use in CI pipelines
  - [ ] Publish command that wraps the upload API, with support for multiple registry types and automatic metadata extraction from the artifact (e.g. `go.mod` for Go modules, `pom.xml` for Maven)
  - [ ] Version management commands for deprecating, yanking, or deleting specific versions
  - [ ] Package management commands for listing versions, viewing metadata, or managing owners
  - [ ] Authentication support for all providers (static tokens, OIDC, Kubernetes service accounts)
  - [ ] List of available registries and their types, with per-registry configuration details (e.g. whether publishing is enabled, versioning policies)
  - [ ] List packages and versions in a registry, with filtering and sorting options

---

## SBOM support

- [x] Proxy existing SBOMs from upstreams that provide them (GitHub dependency graph API, npm `bom.json`) — enabled by `fetch_upstream = true` in `[registries.sbom]`
- [x] Generate a minimal per-artifact SBOM (SPDX 2.3 or CycloneDX 1.4) at proxy time, from registry metadata and the downloaded archive
- [x] Org-level SBOM export: all artifacts served in a time range as a single merged document (`GET /api/v1/sbom/export?from=…&to=…&format=spdx|cyclonedx`) — admin UI at `/admin/sbom`
- [x] Generate SBOMs at upload time for private registries, extracting dependency manifests from the archive (`go.mod`, `Cargo.toml`, `package.json`, `pom.xml`, `requirements.txt`)
- [x] Policy option: deny publishing a private package if no manifest is found in the archive (`required = true` in `[registries.sbom]`)
- [x] Per-artifact SBOM accessible from the Package Explorer version detail view (SPDX and CycloneDX download buttons per version)
- [ ] Periodically re-check cached SBOMs against vulnerability databases (see [Artifact integrity](#artifact-integrity--security)) and update block / warn metadata automatically

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
- [ ] Integration tests against real upstream registries (gated, opt-in)
- [ ] Broader fuzzing targets beyond the current four (RBAC, cache key, deny-latest, release age)
