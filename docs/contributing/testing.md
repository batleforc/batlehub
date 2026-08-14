# Testing

This document describes how BatleHub is tested: the categories of automated
tests, what each layer covers, how to run them, and — most importantly — **what
is currently exercised by the integration suite**. It is a map, not a tutorial;
for how to add a test when you add a registry, see
[`adding-a-registry.md`](adding-a-registry.md) § Testing.

> Test-function counts in this document are grep-derived snapshots
> (`#[test]` / `#[tokio::test]` / `#[actix_web::test]`) and drift as the suite
> grows. Treat them as orders of magnitude, not contract. The file lists and the
> *shape* of coverage are the stable part.

---

## Table of Contents

1. [Test taxonomy](#1-test-taxonomy)
2. [Running the tests](#2-running-the-tests)
3. [Unit tests](#3-unit-tests)
4. [In-process integration tests](#4-in-process-integration-tests)
5. [Per-registry local-registry tests](#5-per-registry-local-registry-tests)
6. [Path-traversal guard tests](#6-path-traversal-guard-tests)
7. [External integration tests (real infra)](#7-external-integration-tests-real-infra)
8. [CLI integration tests](#8-cli-integration-tests)
9. [Example-project tests](#9-example-project-tests)
10. [Fuzz targets](#10-fuzz-targets)
11. [Coverage](#11-coverage)
12. [CI wiring](#12-ci-wiring)

---

## 1. Test taxonomy

BatleHub's tests fall into five layers, in increasing order of infrastructure cost:

| Layer | Where | Infra | Runner |
|-------|-------|-------|--------|
| **Unit** | `#[cfg(test)] mod tests` inline in each source file | none (HTTP upstreams mocked with `mockito`) | `cargo test --workspace --lib --bins` |
| **In-process integration** | `crates/web/tests/*.rs`, `crates/examples/tests/*.rs` | none — full actix app on in-memory backends | `cargo test -p batlehub-web --test '*'` |
| **CLI subprocess integration** | `cli/tests/integration.rs` | none — CLI binary vs. in-memory actix server | `cargo test -p batlehub-cli --test integration` |
| **External integration** | `crates/adapters/tests/*.rs` | real Postgres / MinIO(S3) / Redis via Podman | `task test:pg-*`, `task test:s3` |
| **Fuzz** | `fuzz/fuzz_targets/*.rs` | nightly toolchain | `task fuzz` |

The in-process layer is the workhorse: every test there spins up a real
actix-web application wired to `InMemoryPackageRepository`,
`InMemoryStorageBackend`, `InMemoryCacheStore`, and `FixedRegistry`, so it
exercises the true request lifecycle (auth middleware → handler → service →
rules → storage) without any external dependency.

---

## 2. Running the tests

```bash
# Everything that needs no external infra
cargo test --workspace

# One package / one filter
cargo test -p batlehub-web namespaces
cargo test -p batlehub-adapters --lib rbac

# In-process integration only
cargo test -p batlehub-web --test '*'
cargo test -p batlehub-cli --test integration

# External integration (each starts its own container via Podman)
task test:pg-cache            # Postgres — PgCacheStore
task test:pg-local-registry   # Postgres — PostgresLocalRegistry
task test:pg-storage-router   # Postgres — StorageRouter
task test:pg-artifact-meta    # Postgres — PgArtifactMetaRepository
task test:pg-vulnerability    # Postgres — PgVulnerabilityRepository
task test:s3                  # MinIO    — S3StorageBackend (feature storage-s3)

# Repo interop (real apt/dnf/pacman consume signed repos)
task test:repo-interop

# Coverage (starts Postgres + MinIO + Redis; HTML report in coverage/html/)
task coverage
task coverage-check           # fails if line coverage < 80%

# Fuzz (nightly)
task fuzz TARGET=fuzz_deny_latest MAX_TIME=30
```

The Redis adapter tests (`redis_cache`, `redis_rate_limit`,
`redis_warm_coordinator`) and `pg_rate_limit` / `actions_oidc` have no dedicated
`task test:*` wrapper but run under `task coverage` and in CI.

---

## 3. Unit tests

Unit tests live inline (`#[cfg(test)] mod tests`) in the file they cover. The
notable convention is **registry-client tests**: they mock the upstream HTTP API
with `mockito::Server` rather than hitting the real registry.

Registry adapters with test modules under `crates/adapters/src/registry/`:

- **Standalone `<name>/tests.rs`** (used when tests span more than one sibling
  file): `composer`, `conda`, `jetbrains_marketplace`, `pypi`, `rubygems`,
  `terraform`.
- **Inline `mod tests`**: `cargo`, `npm`, `openvsx`, `goproxy`, plus the shared
  infrastructure modules `fanout`, `http_client`, `path_proxy`, `ssrf`; and the
  directory clients `forgejo/client.rs`, `github/client.rs`, `gitlab/client.rs`,
  `maven/models.rs`, `nuget/client.rs`, `vscode_marketplace/client.rs`.

Every registry adapter has a test module. The `ssrf` and `http_client` modules
additionally cover SSRF protection and the shared upstream HTTP client
(auth forwarding, TLS, private-CA support).

---

## 4. In-process integration tests

`crates/web/tests/*.rs` — **~38 files, ~570 test functions** (point-in-time).
Shared app-factory infrastructure (`make_app`, `make_local_svc`,
`access_config*`, `LocalRegistryAppParts` / `build_local_registry_app`) lives in
`crates/web/tests/common/mod.rs`; every other file begins with
`mod common; use common::*;`.

Feature areas covered (file → area):

| File | Area |
|------|------|
| `proxy_basic.rs` | Core proxy: cache-first read, stale-on-error, streaming |
| `proxy_npm_edge_cases.rs`, `proxy_cargo_edge_cases.rs` | npm / cargo proxy edge cases |
| `proxy_openvsx_vscode_goproxy.rs` | OpenVSX / VS Code Marketplace / Go proxy |
| `cargo_and_downloads.rs` | Cargo proxy paths + download counting |
| `generic_proxy.rs` | Generic file-mirror `GET /proxy/{reg}/generic/{path}` |
| `repo_deb_rpm_pacman.rs` | Deb / RPM / Pacman path repositories |
| `terraform.rs` | Terraform modules + providers (v1 API) |
| `namespaces_and_visibility.rs` | Namespace claim/release + package visibility |
| `admin_packages.rs`, `admin_stats.rs`, `admin_health_and_bulk.rs`, `admin_access_check.rs` | Admin API: packages, stats, health, bulk ops, access-check |
| `bulk_and_quota_and_cache.rs` | Bulk operations, per-registry quotas, cache clear/warm |
| `tokens_and_pagination.rs` | API token CRUD + list pagination |
| `rate_limit.rs` | Rate-limiting middleware |
| `ip_blocks.rs`, `user_blocks.rs` | IP block enforcement, user block/unblock |
| `dynamic_groups.rs` | Dynamic group membership / RBAC groups |
| `beta_channel.rs` | Beta / pre-release channel gating |
| `banner_and_config_reload.rs` | Service banner + hot config-reload endpoint |
| `notifications.rs` | Notification channels / dispatch |
| `explore.rs` | Package Explorer discovery backend |
| `vuln_proxy_endpoints.rs`, `vuln_findings.rs` | Vulnerability proxy endpoints + findings store |
| `sbom_and_misc.rs` | SBOM read endpoints |
| `publish_traversal_guards.rs`, `upload_traversal_and_enforcement.rs` | Cross-registry publish/upload traversal guards + policy enforcement |

---

## 5. Per-registry local-registry tests

Each registry that supports **local/hybrid** mode has a dedicated
`local_<type>_registry.rs` file with a `make_local_<type>_app(mode)` factory and
a publish-payload helper. These assert the full private-registry lifecycle:
publish (with anon-403 / proxy-404 / duplicate-409 rejections), download, the
registry-specific metadata endpoints, yank/unyank/delete, and hybrid
local-vs-proxy precedence.

| File | Highlights |
|------|-----------|
| `local_cargo_registry.rs` | Sparse index, download, yank/unyank, unlist, admin-gated deprecate, owners, hybrid precedence |
| `local_npm_registry.rs` | Packument, version metadata, tarball download, name + version traversal guards |
| `local_maven_registry.rs` | PUT pom (version-mismatch-400 / dup-409), jar-before-pom, `maven-metadata.xml`, proxy-mode rejection |
| `local_nuget_registry.rs` | v3 service index (+ vulnerabilities resource), `X-NuGet-ApiKey` auth, flat-index, registration/catalog, search |
| `local_composer_registry.rs` | `packages.json`, p2 metadata (local/proxy/hybrid fallback), dist streaming, invalid-zip-422 |
| `local_go_registry.rs` | `@v/list`, `.info`, `.mod` extraction, `.zip` download, `@latest` |
| `local_vsx_registry.rs` | VSIX publish + download-after-publish (shared by OpenVSX & VS Code Marketplace) |
| `local_jetbrains_marketplace_registry.rs` | Plugin publish (jar / nested-zip / descriptor validation), `updatePlugins.xml` build filtering, search, compatible-updates, offline/stale serving |
| `local_rubygems_proxy.rs` | Proxy-mode gem download, info, versions, specs (full/latest/prerelease); publish/yank return 404 in proxy mode |

Additional local-registry coverage (Deb, RPM, Pacman, Conda, PyPI, Terraform)
lives in the feature-area files above (`repo_deb_rpm_pacman.rs`, `terraform.rs`,
and the proxy/upload files).

---

## 6. Path-traversal guard tests

Rejecting `..` and path separators in package coordinates before they reach a
storage key is a **hard requirement** for every registry (see the security note
in `CLAUDE.md` and `adding-a-registry.md`). The canonical regression is
`<type>_publish_traversal_version_returns_400`, which publishes with
`version = "../../etc/x"` and asserts `400`.

Traversal guards are currently tested for: **cargo, composer, conda, deb,
generic, jetbrains-marketplace, maven, npm (name + version), nuget (id +
version), openvsx, pacman, pypi, rpm, rubygems, terraform (provider artifact
path)** — plus a delete-cached-artifact traversal case. New registries **must**
add the matching test.

---

## 7. External integration tests (real infra)

`crates/adapters/tests/*.rs` — these need real infrastructure, opt in via
environment variables (`DATABASE_URL`, `S3_TEST_ENDPOINT`, `REDIS_URL`), and
**skip gracefully** when the variable is unset.

| File | Infra | Verifies | Task |
|------|-------|----------|------|
| `pg_cache.rs` | Postgres | `PgCacheStore` | `test:pg-cache` |
| `local_registry.rs` | Postgres | `PostgresLocalRegistry` (publish/yank/delete) | `test:pg-local-registry` |
| `artifact_meta.rs` | Postgres | `PgArtifactMetaRepository` | `test:pg-artifact-meta` |
| `storage_router.rs` | Postgres | `StorageRouter` | `test:pg-storage-router` |
| `pg_vulnerability.rs` | Postgres | `PgVulnerabilityRepository` | `test:pg-vulnerability` |
| `pg_rate_limit.rs` | Postgres | `PgRateLimitStore` | coverage / CI |
| `s3_storage.rs` | MinIO / S3 (`storage-s3`) | `S3StorageBackend` | `test:s3` |
| `redis_cache.rs` | Redis (`cache-redis`) | `RedisCacheStore` | coverage / CI |
| `redis_rate_limit.rs` | Redis (`cache-redis`) | `RedisRateLimitStore` | coverage / CI |
| `redis_warm_coordinator.rs` | Redis (`cache-redis`) | `RedisWarmCoordinator` | coverage / CI |
| `actions_oidc.rs` | none (mockito) | GitHub-Actions OIDC bootstrap / discovery / JWKS failures | CI |
| `selfhosted.rs` | none (mockito + in-test TLS) | private-CA HTTPS upstreams, `UpstreamHttpOptions` (bearer/basic auth, TLS) | CI |
| `repo_interop.rs` | none at test time (`#[ignore]`d generator) | writes a fully-signed Deb + RPM + Pacman repo with production signing code, consumed by `tests/interop/verify.sh` | `test:repo-interop` |

The **repo-interop** flow is notable: `repo_interop.rs` generates signed
repositories using the real signing code path, and `tests/interop/verify.sh`
then has genuine `apt`, `dnf`, and `pacman` clients consume and verify them —
end-to-end proof that the OS-package output is standards-compliant.

---

## 8. CLI integration tests

`cli/tests/integration.rs` — a single file with **~83 test functions**. It:

1. Starts a genuine actix-web `HttpServer` on in-memory backends
   (`TestServer::start`) and waits for the port.
2. Invokes the CLI **as a subprocess** via
   `Command::new(env!("CARGO_BIN_EXE_batlehub-cli"))`, so cargo builds the
   binary automatically before the test run.
3. Seeds admin-visible packages directly with `TestServer::seed_package()`.

**In-memory store separation caveat.** `InMemoryLocalRegistry` (used by
`LocalRegistryService` for publish/yank/delete) and `InMemoryPackageRepository`
(used by `AdminService` for package list/block) are **separate** stores in
tests, though they share tables in Postgres. Consequence: a package published
via the local-registry HTTP endpoint does **not** appear in
`GET /api/v1/packages`. Tests use `seed_package()` for `package list`, and query
registry-specific endpoints (e.g. the NuGet flat-index at
`/proxy/{reg}/nuget/v3/flat/{id}/index.json`) to verify yank/unyank/delete state.

Flows exercised include: `registry list/info/suggest` (from `mise.lock`,
`Cargo.toml`, …), `auth whoami/token/login (kubernetes,oidc)/refresh`,
`package list/versions`, `version yank/unyank/delete`, CLI publish for
nuget/npm/pypi/cargo (with a publish→proxy round-trip), `admin` (quota, ip-block,
banner, cache clear/warm, config reload/changes, audit-log list/purge/export,
stats, health, visibility, users, namespace, sbom, bulk ops, access-check,
notifications), `config show/set`, `completion`, `setup detect` (per-manifest
detection with depth / monorepo / hidden-dir handling), `setup ide`, and
`registry-types`.

---

## 9. Example-project tests

`crates/examples/tests/*.rs` — in-memory backends, no external DB, run in CI via
`batlehub-examples --test '*'`:

- `local_registry.rs` — local-mode upload/pull cycle through a real BatleHub app.
- `real_proxy.rs` — the real proxy against public upstreams; each test **skips
  gracefully** (early `return;`) when the toolchain or network is unavailable.
- `smoke.rs` — end-to-end smoke tests for every example project, with a recording
  HTTP server standing in for the proxy.
- `structure.rs` — project-structure assertions.

---

## 10. Fuzz targets

`fuzz/fuzz_targets/` (libfuzzer, nightly, `task fuzz`) — all fuzz
`batlehub-core` domain logic:

- `fuzz_rbac_evaluate.rs` — RBAC rule evaluation.
- `fuzz_deny_latest.rs` — `DenyLatestRule`; asserts only the exact string
  `"latest"` denies (unicode homoglyphs / whitespace must neither bypass nor
  over-block).
- `fuzz_release_age.rs` — `ReleaseAgeGateRule` (durations capped at one year).
- `fuzz_package_id_cache_key.rs` — `PackageId` cache-key generation is
  deterministic and always contains the registry component.

---

## 11. Coverage

- **Gate: 80% lines.** `task coverage-check` runs
  `cargo llvm-cov report --fail-under-lines 80`.
- `task coverage` aggregates the workspace tests **plus** each adapter
  integration test explicitly (pg_cache, pg_vulnerability, local_registry,
  artifact_meta, pg_rate_limit, storage_router, s3_storage [+`storage-s3`],
  actions_oidc, and the Redis suite [+`cache-redis`]).
- Excluded paths live in the `COVERAGE_EXCLUDE` variable in `Taskfile.yml`
  (kept identical between `coverage` and `coverage-check`). They are code that is
  either environment-glue or exercised only against real infra: `server/src`
  wiring (`main`, `server_factory`, `setup`, `stores`, `watcher`), the OIDC auth
  handlers, `crates/adapters/src/db/`, the Postgres/Redis adapter
  implementations, `cli/src/tui/` (terminal UI), and a few others. When you add
  code that genuinely can't be unit-tested, add it to `COVERAGE_EXCLUDE` with the
  same reasoning — don't lower the gate.

---

## 12. CI wiring

`.github/workflows/`:

- **`test.yaml`** — the main Rust matrix:
  - `lint`: `cargo clippy --workspace -- -D warnings` + `cargo fmt --all --check`.
  - `unit`: `cargo llvm-cov --workspace --lib --bins` + `cargo test --workspace --doc`.
  - `integration`: web (`-p batlehub-web --test '*'`), CLI
    (`-p batlehub-cli --test integration`), examples
    (`-p batlehub-examples --test '*'`), adapters default + Postgres, S3
    (`--features storage-s3 --test s3_storage`), Redis (`--features cache-redis`).
  - `heavy-marketplace`: `bash tests/heavy/marketplace.sh` (headless VS Code +
    IntelliJ).
- **`front-test.yaml`** — frontend (`ui/`): install, regenerate the OpenAPI spec
  + TS client, `pnpm run coverage`.
- **`repo-interop.yaml`** — `bash tests/interop/verify.sh` (apt + dnf + pacman
  accept signed repos).
- **`sonar.yaml`** — SonarCloud: rebuilds full Rust + frontend coverage and
  uploads `lcov.info`.

The `.github` workflows start their own Postgres/MinIO/Redis service containers
in YAML; the `task test:*` targets are the local-dev equivalents (Podman).
`.forgejo/workflows/` handles image builds, Helm, the website, and updatecli —
the Rust test matrix runs on the GitHub side.
