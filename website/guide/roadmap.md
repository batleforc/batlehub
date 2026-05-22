# Roadmap

This page tracks planned features and improvements for BatleHub, grouped by theme. Within each group the order reflects rough implementation priority.

To propose a feature or discuss an item, open an issue on the [project repository](https://git.batleforc.fr/batleforc/batlehub).

[[toc]]

---

## New registry types {#new-registries}

BatleHub currently supports npm, Cargo, GitHub, OpenVSX, VS Code Marketplace, and Go modules. The following adapters are planned:

| Registry | Notes |
|----------|-------|
| **PyPI** | Python simple API + wheel / sdist downloads |
| **Maven / Gradle** | Maven Central–compatible metadata XML + JAR / POM |
| **RubyGems** | Gem downloads and version listing |
| **NuGet** | .NET package protocol |
| **Deb / RPM** | Debian APT and Red Hat YUM repository proxying |
| **Terraform registry** | Provider and module proxy protocol |
| **GitLab** | Releases and packages — similar to GitHub, different auth / pagination |
| **Forgejo** | Gitea fork with minor API differences |

::: info Docker / OCI not planned
[Harbor](https://goharbor.io) covers this use case better than BatleHub could. If you have a concrete need, open an issue.
:::

---

## Cache policy {#cache-policy}

- **Cache-Control headers** — honour `no-cache`, `max-age`, and `no-store` from upstream responses to decide whether and how long to cache
- **Eviction policies** — TTL-based expiry, "not accessed for N days", keep only the latest N versions, storage size cap with LRU eviction
- **Deduplication** — content-addressable storage for backends that support it (S3 object versioning, RustFS)
- **Proactive cache warming** — pre-fetch all known versions of a package to eliminate cold-start latency
- **Cache index coherence** — detect and recover from mismatches between storage contents and registry metadata (corruption, manual deletions)

---

## Metrics & observability {#metrics}

- **Prometheus endpoint** (`/metrics`) — request counts, cache hit/miss rates, latency percentiles, error rates per registry
- **Health check** (`/healthz`) — verifies connectivity to the database and all configured storage backends
- **Admin dashboard** — hits/misses and bandwidth saved, per-registry and aggregate, on the admin home screen

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

## Rate limiting & DoS protection {#rate-limiting}

- Per-user, per-group, and per-registry rate limits with configurable thresholds and time windows
- Configurable enforcement: hard block vs. soft warn on limit exceeded
- Standard rate-limit headers (`Retry-After`, `X-RateLimit-*`) and UI warnings when approaching a limit
- IP-based blocking for abusive clients
- Integration with external IP reputation services

---

## Quota management {#quotas}

- Per-user, per-group, and per-registry quotas on storage usage and number of published packages
- Enforcement policies: block or warn on quota exceeded
- Quota warning in API responses and admin UI
- Admin API for resetting quotas

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

- **npm** — versioning policies (enforce semantic versioning, restrict accepted patterns)
- **Cargo** — versioning policies; full yank protocol compatibility
- **VS Code extensions** — deprecation and unlisting; VSIX upload form in the UI

### For all private registry types

- Artifact signing and verification (OpenPGP or similar)
- Ownership and team management: multiple users / groups per package with distinct roles
- Versioning policies: enforce semantic versioning or restrict accepted version patterns
- Beta / pre-release channel: gate unpublished versions to specific users or groups
- Bulk operations: bulk publish, bulk deprecation, bulk deletion
- Content-addressable deduplication and integrity verification for stored artifacts

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

- Unit tests for all registry adapters and policy evaluation logic
- Integration tests against real upstream registries (gated, opt-in)
- Broader fuzzing targets beyond the current four (`fuzz_rbac_evaluate`, `fuzz_package_id_cache_key`, `fuzz_deny_latest`, `fuzz_release_age`)
