# Roadmap

This page tracks planned features and improvements for BatleHub, grouped by theme. Within each group the order reflects rough implementation priority.

To propose a feature or discuss an item, open an issue on the [project repository](https://git.batleforc.fr/batleforc/batlehub).

[[toc]]

---

## New registry types {#new-registries}

BatleHub currently supports npm, Cargo, GitHub, OpenVSX, VS Code Marketplace, Go modules, Maven / Gradle, RubyGems, Terraform, and Composer. The following adapters are planned:

| Registry | Status | Notes |
|----------|--------|-------|
| **npm** | ✅ Shipped | Package proxy + local/hybrid publishing |
| **Cargo** | ✅ Shipped | Sparse index + crate downloads |
| **GitHub** | ✅ Shipped | Release artifact proxy |
| **OpenVSX** | ✅ Shipped | Extension proxy |
| **VS Code Marketplace** | ✅ Shipped | Extension proxy |
| **Go modules** | ✅ Shipped | GOPROXY protocol |
| **Maven / Gradle** | ✅ Shipped | Maven Central–compatible metadata XML + JAR / POM; `mvn deploy` support |
| **RubyGems** | ✅ Shipped | Gem downloads and version listing; publish/yank/unyank |
| **Terraform registry** | ✅ Shipped | Provider and module proxy; private module + provider publishing |
| **Composer** | ✅ Shipped | Packagist v2 protocol; packages.json + p2 metadata + dist downloads; private ZIP publishing in local/hybrid mode |
| **PyPI** | Planned | Python simple API + wheel / sdist downloads |
| **NuGet** | Planned | .NET package protocol |
| **Deb / RPM** | Planned | Debian APT and Red Hat YUM repository proxying |
| **GitLab** | Planned | Releases and packages — similar to GitHub, different auth / pagination |
| **Forgejo** | Planned | Gitea fork with minor API differences |

::: info Docker / OCI not planned
[Harbor](https://goharbor.io) covers this use case better than BatleHub could. If you have a concrete need, open an issue.
:::

---

## Cache policy {#cache-policy}

All planned cache policy features have shipped:

- ✅ **Cache-Control headers** — honour `no-cache`, `max-age`, and `no-store` from upstream responses to decide whether and how long to cache
- ✅ **Eviction policies** — TTL-based expiry, "not accessed for N days", keep only the latest N versions, storage size cap with LRU eviction
- ✅ **Cache index coherence** — detect and recover from mismatches between storage contents and registry metadata (corruption, manual deletions)
- ✅ **Content-addressable deduplication** — identical artifact bytes stored once, ref-counted across logical keys; transparent and backwards-compatible
- ✅ **Proactive cache warming** — pre-fetch known versions of configured packages at startup and on demand via `POST /api/v1/admin/registries/{registry}/warm`

---

## Metrics & observability {#metrics}

- ✅ **Prometheus endpoint** (`/metrics`) — request counts, cache hit/miss rates, latency percentiles, error rates per registry
- ✅ **Health check** (`/healthz`) — verifies connectivity to the database and all configured storage backends
- ✅ **Admin dashboard** — hits/misses and bandwidth saved, per-registry and aggregate, on the admin home screen

---

## Artifact integrity & security {#integrity}

BatleHub aims to be a trust boundary, not just a cache. Planned integrity features:

- Checksum verification for downloaded artifacts when the upstream provides them (Cargo sparse index SHA-256, npm `integrity`, etc.)
- Block serving an artifact if its integrity check fails, or optionally if no integrity metadata is available
- Sigstore / npm provenance verification for npm packages
- `cargo verify-project`-style verification for Cargo crates
- Detect and optionally require signed releases (GitHub, OpenVSX, VS Code Marketplace)
- Allowlist of trusted publishers (GitHub users / orgs, npm scopes, Cargo owners)
- Allowlist of approved versions; blocklist of specific versions with known issues
- Vulnerability scanning via the [OSV API](https://osv.dev) to block or warn on known CVEs
- YARA rule evaluation for custom malware or policy patterns
- Antivirus scanning for binary artifacts (VSIX, Go module zips) via an external REST API
- Upstream health warnings when cached data may be stale

---

## Authentication providers {#auth-providers}

| Provider | Status | Notes |
|----------|--------|-------|
| **Static tokens** | ✅ Shipped | Plain-text and Argon2id-hashed; `batlehub hash-token` CLI |
| **OIDC** | ✅ Shipped | JWT validation, browser SSO (Authorization Code), role + group claim mapping |
| **Kubernetes service accounts** | ✅ Shipped | TokenReview API; in-cluster defaults; role + group mapping |
| **GitHub / Forgejo Actions OIDC** | ✅ Shipped | Validate short-lived workflow JWTs; rule-based group mapping from any claim; dynamic group name templates; glob + regex conditions |

::: info Saml / Github PAT / Gitlab PAT
Saml and specific GitHub/GitLab PAT providers are not planned, but may be possible to implement via the generic OIDC provider with some custom configuration. Open an issue if you have a concrete use case or want to contribute an adapter.
:::

### Actions OIDC highlights

The `actions-oidc` provider lets CI jobs authenticate without long-lived secrets. Workflow JWTs carry rich context claims (`repository`, `ref`, `environment`, `actor`, …) that can be matched by glob or regex rules to grant specific groups and roles:

```toml
[[auth]]
type = "actions-oidc"
name = "github-actions"
issuer_url = "https://token.actions.githubusercontent.com"

  [[auth.rules]]
  group_template = "{name}/{repository}/{ref_name}"
  role = "user"
  match = "all"
  [[auth.rules.conditions]]
  claim = "repository_owner"
  pattern = "myorg"
```

A token from `myorg/my-repo` on `main` resolves to group `github-actions/myorg-my-repo/main`, which you can grant registry permissions to with a wildcard: `"github-actions/*" = ["releases:write"]`.

See [Configuration § Actions OIDC auth](/guide/../docs/configuration#334-actions-oidc-auth-type--actions-oidc) for the full reference.

---

## Rate limiting & DoS protection {#rate-limiting}

- ✅ **Per-user and per-group rate limits** — fixed-window counters with configurable thresholds and time windows, backed by InMemory / PostgreSQL / Redis
- ✅ **Configurable enforcement** — hard block (429) or soft warn; standard `Retry-After` and `X-RateLimit-*` headers
- ✅ **IP-based blocking** — fail2ban-style: auto-block IPs that exceed a violation threshold; manual block/unblock via admin API; `X-Block-Expires` header; fail-open on store errors. See [Access Control guide](/guide/access-control#ip-blocking).
- Integration with external IP reputation services

---

## Quota management {#quotas}

- ✅ **Per-user, per-group, and per-registry quotas** — on storage usage and number of published packages
- ✅ **Enforcement policies** — block or warn on quota exceeded
- ✅ **Quota warnings** — in API responses and admin UI when a limit is being approached
- ✅ **Admin API for resetting quotas** — for specific users, groups, or registries

---

## Hot reloading & dynamic config {#hot-reload}

- File-watching with admin confirmation before applying changes
- Config validation (schema + connectivity probes) before any change takes effect
- Partial reloads: update RBAC rules or add/remove a registry without a full restart
- `POST /api/admin/config/reload` endpoint for automation
- Audit trail for all config changes
- Dynamic blocking rules and allowlists from external signed sources (e.g. a signed Git repository)

---

## Webhooks & notifications {#webhooks}

- Subscribe to events for specific packages, versions, or registries (new publish, deprecation, removal)
- Channels: email, Slack, Microsoft Teams, outbound webhooks
- User-configurable preferences in the UI
- Inbound webhook API for external systems (CI pipelines, security scanners) to push events into BatleHub

---

## Private registry features {#private-registry}

Applies to registries running in `local` or `hybrid` mode. See the [User Guide](/guide/user) for current publish flows.

### Per-registry additions

- ✅ **Maven** — private artifact publishing via `mvn deploy`; POM-triggered three-phase publish; JAR/checksum pre-upload; dynamically generated `maven-metadata.xml`; `local` and `hybrid` modes
- ✅ **Terraform** — private module publishing (tar.gz upload, `X-Terraform-Get` redirect); private provider publishing (version manifest + per-platform binary); `local` and `hybrid` modes
- ✅ **Composer** — private PHP package publishing via ZIP upload; `composer.json` extracted automatically; `local` and `hybrid` modes
- **npm** — versioning policies (enforce semantic versioning, restrict accepted patterns)
- **Cargo** — versioning policies; full yank protocol compatibility
- **VS Code extensions** — deprecation and unlisting; VSIX upload form in the UI

### For all private registry types

- ✅ **Artifact signing** — publish-time `X-Artifact-Signature` / `X-Signature-Type` headers; stored and returned on download; configurable required enforcement and allowed-type allowlist
- ✅ **Ownership management** — per-package owner list with roles; admin API for listing, adding, and removing owners
- ✅ **Versioning policies** — enforce semver and/or restrict accepted version patterns per registry
- ✅ **Beta/pre-release channel** — gate pre-release versions (semver `-pre` suffix) to specific users or groups; non-members see only stable versions. See [Access Control guide](/guide/access-control#beta-channel).
- ✅ **Bulk operations** — bulk yank, unyank, and delete via admin API
- Bulk publish, bulk deprecation
- ✅ **Content-addressable deduplication** — identical artifact bytes stored once, ref-counted across logical keys; transparent and backwards-compatible
- Integrity verification: verify checksums on re-serve, not only at publish time

### CLI tool — `batlehub-cli`

A standalone CLI for common private registry tasks, suitable for CI pipelines:

```sh
batlehub-cli publish --registry internal-npm ./dist
batlehub-cli deprecate --registry internal-cargo my-crate@1.2.0
batlehub-cli yank --registry internal-cargo my-crate@1.2.0
batlehub-cli list --registry internal-go example.com/mymod
```

---

## SBOM support {#sbom}

Software Bill of Materials support, driven by compliance requirements (EU Cyber Resilience Act, US Executive Order 14028):

| Feature | Description |
|---------|-------------|
| Upstream passthrough | Proxy SBOMs provided by upstreams (GitHub dependency graph API, npm `bom.json`) |
| Per-artifact generation | Generate a minimal SPDX / CycloneDX document at cache time from metadata and checksum |
| Org-level export | `GET /api/sbom/export?from=…&format=spdx` — all artifacts served in a time range |
| Upload-time generation | For private registries: extract `go.mod`, `Cargo.toml`, `package.json` at publish time |
| Publish policy | Optionally deny packages with no SBOM or a failing SBOM |
| Continuous re-evaluation | Periodically re-check cached SBOMs against OSV and update block / warn metadata |

---

## UI improvements {#ui}

- Package detail pages: full metadata, version history, download links
- Global search across all registries, including packages not yet cached
- User listing and block management for OIDC and Kubernetes-sourced identities
- Config editor with validation, diff preview, and apply button (tied to hot reload)
  - Read-only warning when the config is mounted from a Kubernetes ConfigMap

---

## Testing {#testing}

- ✅ **Unit tests — significant progress** — entities, services, auth providers, storage router, registry adapters (cargo, npm, github, …), web middleware, and handler guards are all covered. Line coverage enforced at ≥ 80% via `task coverage-check` (llvm-cov).
- Integration tests against real upstream registries (gated, opt-in)
- Broader fuzzing targets beyond the current four (`fuzz_rbac_evaluate`, `fuzz_package_id_cache_key`, `fuzz_deny_latest`, `fuzz_release_age`)
