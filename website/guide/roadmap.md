# Roadmap

This page tracks planned features and improvements for BatleHub, grouped by theme. Within each group the order reflects rough implementation priority.

To propose a feature or discuss an item, open an issue on the [project repository](https://git.batleforc.fr/batleforc/batlehub).

[[toc]]

---

## New registry types {#new-registries}

BatleHub currently supports npm, Cargo, GitHub, OpenVSX, VS Code Marketplace, Go modules, Maven / Gradle, RubyGems, Terraform, Composer, PyPI, and Conda. The following adapters are planned or in progress:

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
| **PyPI** | ✅ Shipped | Simple API proxy with URL rewriting (pip, uv, Poetry); wheel and sdist downloads; twine-compatible private publishing in local/hybrid mode |
| **Conda** | ✅ Shipped | repodata.json proxy (all platforms); `.conda` and `.tar.bz2` downloads; private channel publishing; hybrid repodata merge |
| **NuGet** | Planned | .NET package protocol |
| **Deb / RPM** | Planned | Debian APT and Red Hat YUM repository proxying |
| **GitLab** | Planned | Releases and packages — similar to GitHub, different auth / pagination |
| **Forgejo** | Planned | Gitea fork with minor API differences |

::: info Docker / OCI not planned
[Harbor](https://goharbor.io) covers this use case better than BatleHub could. If you have a concrete need, open an issue.
:::

---

## Cache policy {#cache-policy}

| Feature | Status | Notes |
|---------|--------|-------|
| **Cache-Control headers** | ✅ Shipped | Honour `no-cache`, `max-age`, and `no-store` from upstream responses |
| **Eviction policies** | ✅ Shipped | TTL-based expiry, "not accessed for N days", keep only the latest N versions, storage size cap with LRU eviction |
| **Cache index coherence** | ✅ Shipped | Detect and recover from mismatches between storage contents and registry metadata (corruption, manual deletions) |
| **Content-addressable deduplication** | ✅ Shipped | Identical artifact bytes stored once, ref-counted across logical keys; transparent and backwards-compatible |
| **Proactive cache warming** | ✅ Shipped | Pre-fetch known versions at startup and on demand via `POST /api/v1/admin/registries/{registry}/warm` |

---

## Metrics & observability {#metrics}

| Feature | Status | Notes |
|---------|--------|-------|
| **Prometheus endpoint** | ✅ Shipped | `/metrics` — request counts, cache hit/miss rates, latency percentiles, error rates per registry |
| **Health check** | ✅ Shipped | `/healthz` — verifies connectivity to the database and all configured storage backends |
| **Admin dashboard** | ✅ Shipped | Hits/misses and bandwidth saved, per-registry and aggregate, on the admin home screen |

---

## Artifact integrity & security {#integrity}

BatleHub aims to be a trust boundary, not just a cache.

| Feature | Status | Notes |
|---------|--------|-------|
| Checksum verification | Planned | Verify artifact hashes when the upstream provides them (Cargo sparse index SHA-256, npm `integrity`, etc.) |
| Block on failed integrity | Planned | Block serving an artifact if its checksum fails, or optionally if no integrity metadata is available |
| Sigstore / npm provenance | Planned | Verify npm provenance attestations and Sigstore signatures |
| Cargo crate verification | Planned | `cargo verify-project`-style verification for Cargo crates |
| Signed release enforcement | Planned | Detect and optionally require signed releases (GitHub, OpenVSX, VS Code Marketplace) |
| Trusted publisher allowlist | Planned | Allowlist of trusted GitHub users / orgs, npm scopes, Cargo owners |
| Version allowlist / blocklist | Planned | Allowlist of approved versions; blocklist of specific versions with known issues |
| OSV vulnerability scanning | Planned | Block or warn on CVEs via the [OSV API](https://osv.dev) |
| YARA rule evaluation | Planned | Custom malware or policy pattern matching on artifact bytes |
| Antivirus scanning | Planned | Binary artifact scanning (VSIX, Go module zips) via a configurable external REST API |
| Upstream health warnings | Planned | Warn when cached data may be stale due to upstream errors |

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

See [Configuration § Actions OIDC auth](https://git.batleforc.fr/batleforc/batlehub/src/branch/main/docs/configuration#334-actions-oidc-auth-type--actions-oidc) for the full reference.

---

## Rate limiting & DoS protection {#rate-limiting}

| Feature | Status | Notes |
|---------|--------|-------|
| **Per-user and per-group rate limits** | ✅ Shipped | Fixed-window counters; configurable thresholds and time windows; InMemory / PostgreSQL / Redis backends |
| **Configurable enforcement** | ✅ Shipped | Hard block (429) or soft warn; standard `Retry-After` and `X-RateLimit-*` headers |
| **IP-based blocking** | ✅ Shipped | Auto-block IPs exceeding a violation threshold; manual block/unblock via admin API; `X-Block-Expires` header. See [Access Control guide](/guide/access-control#ip-blocking). |
| External IP reputation | Planned | Integrate with external threat-intel feeds to automatically block known malicious IPs |

---

## Quota management {#quotas}

| Feature | Status | Notes |
|---------|--------|-------|
| **Per-user, per-group, per-registry quotas** | ✅ Shipped | Max storage bytes and max package count; configurable per scope |
| **Enforcement policies** | ✅ Shipped | Block or warn on quota exceeded; `X-Quota-*` headers on every publish response |
| **Quota warnings** | ✅ Shipped | API responses and admin UI indicate when a limit is being approached |
| **Admin quota reset** | ✅ Shipped | Reset quotas for specific users, groups, or registries via admin API |

---

## Hot reloading & dynamic config {#hot-reload}

| Feature | Status | Notes |
|---------|--------|-------|
| File-watching with admin confirmation | ✅ Shipped | File watcher loads a pending reload; admin confirms via UI or `POST /api/v1/admin/config/pending/apply` |
| Config validation before applying | ✅ Shipped | Schema check + `HEAD` connectivity probes to each upstream (5 s timeout) |
| Partial reloads without restart | ✅ Shipped | Registries, policies, RBAC, versioning, signing, and beta-channel maps are all hot-swappable |
| Immediate reload API | ✅ Shipped | `POST /api/v1/admin/config/reload` — load, validate, and apply atomically for CI/CD |
| Disable hot reload | ✅ Shipped | `BATLEHUB_DISABLE_HOT_RELOAD=1` returns 503 from all reload endpoints (use with read-only Kubernetes ConfigMaps) |
| Config change audit trail | ✅ Shipped | Every reload written to `config_changes` table; `GET /api/v1/admin/config/changes` |
| Global admin banner | ✅ Shipped | Broadcast info / warning / error messages to all visitors; HA-safe via Redis or PostgreSQL; auto-set during reload |
| Dynamic blocking rules from external source | Planned | Fetch and apply block rules from a signed external repository (e.g. signed Git repo) |
| Dynamic allowlists from external source | Planned | Fetch trusted publisher / approved version lists and merge into RBAC / block rules automatically |

---

## Webhooks & notifications {#webhooks}

| Feature | Status | Notes |
|---------|--------|-------|
| Event subscriptions | Planned | Subscribe to new publish, deprecation, or removal events for specific packages, versions, or registries |
| Notification channels | Planned | Email, Slack, Microsoft Teams, outbound webhooks |
| User preferences UI | Planned | User-configurable notification preferences in the web UI |
| Inbound webhook API | Planned | Allow external systems (CI pipelines, security scanners) to push events into BatleHub |

---

## Private registry features {#private-registry}

Applies to registries running in `local` or `hybrid` mode. See the [User Guide](/guide/user) for current publish flows.

### Per-registry additions

| Registry | Status | Notes |
|----------|--------|-------|
| **Maven** | ✅ Shipped | `mvn deploy` support; POM-triggered three-phase publish; JAR/checksum pre-upload; dynamic `maven-metadata.xml`; `local` and `hybrid` modes |
| **Terraform** | ✅ Shipped | Private module publishing (tar.gz + `X-Terraform-Get` redirect); private provider publishing (manifest + per-platform binary); `local` and `hybrid` modes |
| **Composer** | ✅ Shipped | Private PHP package publishing via ZIP upload; `composer.json` extracted automatically; `local` and `hybrid` modes |
| **PyPI** | ✅ Shipped | Private Python distribution publishing via twine-compatible multipart upload (`POST /legacy/`); wheel and sdist formats; `local` and `hybrid` modes |
| **Conda** | ✅ Shipped | Private conda package publishing (`.tar.bz2` and `.conda`); metadata extracted from `info/index.json`; `repodata.json` generated and merged automatically; `local` and `hybrid` modes |
| **npm** | Planned | Versioning policies: enforce semantic versioning, restrict accepted version patterns |
| **Cargo** | Planned | Versioning policies; full yank protocol compatibility with crates.io |
| **VS Code extensions** | Planned | Deprecation and unlisting; VSIX upload form in the UI |

### For all private registry types

| Feature | Status | Notes |
|---------|--------|-------|
| **Artifact signing** | ✅ Shipped | Publish-time `X-Artifact-Signature` / `X-Signature-Type` headers; stored alongside artifacts and returned on download; configurable required enforcement and allowed-type allowlist |
| **Ownership management** | ✅ Shipped | Per-package owner list with admin/maintainer roles; admin API for listing, adding, and removing owners |
| **Versioning policies** | ✅ Shipped | Enforce semver and/or restrict accepted version patterns per registry; violations return HTTP 422 at publish time |
| **Beta/pre-release channel** | ✅ Shipped | Gate pre-release versions (semver `-pre` suffix) to specific users or groups; non-members see only stable versions. See [Access Control guide](/guide/access-control#beta-channel). |
| **Bulk operations** | ✅ Shipped | Bulk yank, unyank, and delete via admin API |
| **Content-addressable deduplication** | ✅ Shipped | Identical artifact bytes stored once, ref-counted across logical keys; transparent and backwards-compatible |
| Bulk publish / deprecation | Planned | Batch publish or deprecate multiple versions in a single API call |
| Integrity verification on re-serve | Planned | Re-verify checksums when serving artifacts, not only at publish time |

### CLI tool — `batlehub-cli`

A standalone CLI for common private registry tasks, suitable for CI pipelines:

```sh
batlehub-cli publish --registry internal-npm ./dist
batlehub-cli deprecate --registry internal-cargo my-crate@1.2.0
batlehub-cli yank --registry internal-cargo my-crate@1.2.0
batlehub-cli list --registry internal-go example.com/mymod
```

| Feature | Status |
|---------|--------|
| `publish`, `yank`, `unyank`, `list`, `deprecate` commands | Planned |

---

## SBOM support {#sbom}

Software Bill of Materials support, driven by compliance requirements (EU Cyber Resilience Act, US Executive Order 14028):

| Feature | Status | Notes |
|---------|--------|-------|
| Upstream passthrough | Planned | Proxy SBOMs provided by upstreams (GitHub dependency graph API, npm `bom.json`) |
| Per-artifact generation | Planned | Generate a minimal SPDX / CycloneDX document at cache time from metadata and checksum |
| Org-level export | Planned | `GET /api/sbom/export?from=…&format=spdx` — all artifacts served in a time range |
| Upload-time generation | Planned | For private registries: extract `go.mod`, `Cargo.toml`, `package.json` at publish time |
| Publish policy | Planned | Optionally deny packages with no SBOM or a failing SBOM |
| Continuous re-evaluation | Planned | Periodically re-check cached SBOMs against OSV and update block / warn metadata |

---

## UI improvements {#ui}

| Feature | Status | Notes |
|---------|--------|-------|
| **Package Explorer** | ✅ Shipped | `/explore` — collapsed catalog, registry sidebar, search/sort, upstream search, per-package version detail with firewall + gate status |
| Package detail deep links | Planned | Full metadata, version history, and download links beyond the Explorer summary |
| Global search | Planned | Search across all registries including packages not yet cached |
| User listing & block management | Planned | Manage OIDC and Kubernetes-sourced identities in the admin panel |
| Config editor | Planned | Inline config editing with validation, diff preview, and apply button (integrates with hot reload) |
| Read-only ConfigMap warning | Planned | Show a banner when the config is mounted from a Kubernetes ConfigMap with instructions for external updates |

---

## Testing {#testing}

| Feature | Status | Notes |
|---------|--------|-------|
| **Unit test coverage** | ✅ Shipped | Entities, services, auth providers, storage router, registry adapters, web middleware, and handler guards covered; ≥ 80% line coverage enforced via `task coverage-check` (llvm-cov) |
| Integration tests (real upstreams) | Planned | Gated, opt-in tests against real upstream registries |
| Fuzzing expansion | Planned | Broader fuzzing targets beyond the current four (`fuzz_rbac_evaluate`, `fuzz_package_id_cache_key`, `fuzz_deny_latest`, `fuzz_release_age`) |
