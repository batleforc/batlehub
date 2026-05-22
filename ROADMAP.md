# BatleHub — Roadmap

Planned features and improvements, grouped by theme. Within each group the order reflects rough implementation priority.

For discussion or to propose a feature, open an issue on the [project repository](https://git.batleforc.fr/batleforc/batlehub).

---

## New registry types

Current adapters: npm, Cargo, GitHub, OpenVSX, VS Code Marketplace, Go modules.

- [ ] **PyPI** — Python package index (simple API + wheel / sdist downloads)
- [ ] **Maven / Gradle** — Maven Central-compatible metadata XML + JAR / POM downloads
- [ ] **RubyGems** — gem downloads and version listing
- [ ] **NuGet** — .NET package protocol
- [ ] **Deb / RPM** — Debian APT and Red Hat YUM repository proxying
- [ ] **Terraform registry** — provider and module proxy protocol
- [ ] **GitLab releases and packages** — similar to GitHub but with different auth and pagination
- [ ] **Forgejo releases and packages** — Gitea fork with minor API differences

> **Not planned:** Docker / OCI artifacts. [Harbor](https://goharbor.io) solves this better than we could, unless concrete demand arises.

---

## Cache policy

- [ ] Honour `Cache-Control` headers from upstream responses (`no-cache`, `max-age`, `no-store`) to decide whether and how long to cache
- [ ] Eviction policies: TTL-based expiry, "not accessed for N days", garbage-collect all versions except the latest N, storage size cap with LRU eviction
- [ ] Deduplication for storage backends that support it (S3 object versioning, RustFS content-addressable storage)
- [ ] Proactive cache warming: pre-fetch all known versions of a package to eliminate cold-start latency on first request
- [ ] Cache index coherence: compare what is actually in the storage backend against what the registry metadata expects, and recover from corruption or manual deletions

---

## Metrics & observability

- [ ] Prometheus metrics endpoint (`/metrics`): request counts, cache hit/miss rates, latency percentiles, error rates per registry
- [ ] Health check endpoint (`/healthz`) that verifies connectivity to the database and all configured storage backends
- [ ] Stats dashboard on the admin home screen: hits/misses, bandwidth saved, per-registry and aggregate

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

## Rate limiting & DoS protection

- [ ] Per-user, per-group, and per-registry rate limits on API requests and artifact downloads, with configurable thresholds and time windows
- [ ] Configurable enforcement policies: hard block vs. soft warn when a limit is reached
- [ ] Explicit rate-limit warnings in API responses (`Retry-After`, `X-RateLimit-*` headers) and in the UI
- [ ] IP-based blocking for abusive clients, with configurable block duration and thresholds
- [ ] Integration with external IP reputation services to automatically block known malicious IPs

---

## Quota management

- [ ] Per-user, per-group, and per-registry quotas on storage usage and number of published packages
- [ ] Enforcement policies: block publish requests that exceed the quota, or allow with an explicit warning
- [ ] Quota warnings in API responses and admin UI when a limit is being approached
- [ ] Admin API for resetting quotas for specific users, groups, or registries

---

## Hot reloading & dynamic config

- [ ] Watch the config file for changes and prompt an admin for confirmation before applying
- [ ] Validate the new config before applying it (schema check + connectivity probes) to avoid breaking a running server
- [ ] Partial reloads: update RBAC rules or add/remove a registry without restarting the process
- [ ] API endpoint for triggering a config reload (`POST /api/admin/config/reload`) for automation when file-watching is unavailable
- [ ] Audit trail for all config changes (who triggered, what changed, when)
- [ ] Dynamic blocking rules fetched from an external trusted source (e.g. a signed Git repository); verify signatures before applying
- [ ] Dynamic allowlists of trusted publishers or approved versions, fetched from an external source and merged into RBAC / block rules automatically

---

## Webhooks & notifications

- [ ] Subscribe to notifications for specific packages, versions, or registries (new version published, version deprecated, package removed)
- [ ] Multiple notification channels: email, Slack, Microsoft Teams, outbound webhooks
- [ ] User-configurable notification preferences and channel configuration in the UI
- [ ] Inbound webhook API so external systems (CI pipelines, security scanners) can push events into BatleHub and trigger notifications or policy updates

---

## Private registry features

Applies to registries running in `local` or `hybrid` mode.

### Per-registry additions

- **npm** — versioning policies (enforce semantic versioning, allowlist version patterns)
- **Cargo** — versioning policies; verify full compatibility with the yank protocol from crates.io
- **VS Code extensions** — deprecation and unlisting; upload via the UI (form for VSIX + metadata), in addition to the existing `PUT` API

### For all private registry types

- [ ] Artifact signing and verification (OpenPGP or similar) for published packages
- [ ] Ownership and team management: multiple users / groups can publish and manage the same package with different roles
- [ ] Versioning policies: enforce semantic versioning or restrict accepted version patterns
- [ ] Beta / pre-release channel: allow specific users or groups to access unpublished versions before general release
- [ ] Bulk operations: bulk publish, bulk deprecation, bulk deletion
- [ ] Content-addressable deduplication and integrity verification for stored artifacts

### CLI tool

- [ ] `batlehub-cli` — a standalone CLI for common private registry tasks (`publish`, `deprecate`, `yank`, `list`), suitable for use in CI pipelines

---

## SBOM support

- [ ] Proxy existing SBOMs from upstreams that provide them (GitHub dependency graph API, npm `bom.json`)
- [ ] Generate a minimal per-artifact SBOM (SPDX or CycloneDX) at cache time, from the registry metadata and checksum already available
- [ ] Org-level SBOM export from the audit log: all artifacts served in a time range as a single document (`GET /api/sbom/export?from=…&format=spdx`)
- [ ] Generate SBOMs at upload time for private registries, extracting dependency manifests from the package (e.g. `go.mod`, `Cargo.toml`, `package.json`)
- [ ] Policy option: deny publishing a private package if no SBOM is provided or if the SBOM fails validation
- [ ] Periodically re-check cached SBOMs against vulnerability databases (see [Artifact integrity](#artifact-integrity--security)) and update block / warn metadata automatically

---

## UI improvements

- [ ] Package detail pages with full metadata, version history, and download links
- [ ] Search across all registries, including packages not yet cached (based on upstream registry metadata)
- [ ] User listing and block management in the admin panel (OIDC and Kubernetes-sourced identities, not just static tokens)
- [ ] Config editor with validation, diff preview, and apply button (integrates with hot reload)
  - [ ] Read-only warning when the config file is mounted from a Kubernetes ConfigMap, with instructions for applying changes externally

---

## Testing

- [ ] Unit tests for all registry adapters and policy evaluation logic
- [ ] Integration tests against real upstream registries (gated, opt-in)
- [ ] Broader fuzzing targets beyond the current four (RBAC, cache key, deny-latest, release age)
