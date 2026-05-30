# Contributing to BatleHub

This guide is the starting point for developers working on the BatleHub codebase. It covers the project layout, key architectural patterns, how to run the tests, and known design limitations you need to be aware of before touching specific areas.

---

## Table of contents

1. [Prerequisites](#1-prerequisites)
2. [Workspace layout](#2-workspace-layout)
3. [Architecture: ports and adapters](#3-architecture-ports-and-adapters)
4. [Request lifecycle](#4-request-lifecycle)
5. [Database and migrations](#5-database-and-migrations)
6. [Adding a new feature — checklist](#6-adding-a-new-feature--checklist)
7. [Running tests](#7-running-tests)
8. [Code conventions](#8-code-conventions)
9. [Known limitations and accepted trade-offs](#9-known-limitations-and-accepted-trade-offs)

---

## 1. Prerequisites

| Tool | Minimum version | Notes |
|------|----------------|-------|
| Rust toolchain | stable (see `rust-toolchain.toml`) | `rustup` is the recommended installer |
| PostgreSQL | 14 | Integration tests expect `DATABASE_URL` in the environment |
| Node.js + pnpm | Node 20 / pnpm 8 | UI only (`ui/`) — not needed for Rust-only work |

```bash
# Clone and build
git clone https://git.batleforc.fr/batleforc/batlehub
cd batlehub
cargo build
```

Le repo Git est disponible dans deux provider Git:

- https://git.batleforc.fr/batleforc/batlehub : Instance SelfHosted de Forgejo
- https://github.com/batleforc/batlehub : Miroir GitHub (Principalement en lecture seule, les contributions se font via des pull requests sur la Forgejo)

---

## 2. Workspace layout

```
batlehub/
├── crates/
│   ├── config/        Config schema (TOML → typed structs) and validation
│   ├── core/          Domain types, port traits, pure business logic — no I/O
│   ├── adapters/      Concrete I/O implementations: Postgres, S3, HTTP clients
│   └── web/           actix-web handlers, middleware, OpenAPI wiring
├── server/            Binary entry point: wires everything together
├── docs/              Guides (you are here)
├── ui/                Vue 3 front-end
└── patches/           sqlx-macros stub (see sqlx note in Cargo.toml)
```

### Dependency direction

```
config  ──►  core  ──►  adapters  ──►  web  ──►  server
```

`core` has no I/O dependencies. `adapters` implements `core`'s port traits.
`web` depends on `core` types but calls into adapters only through traits —
never by name. `server` is the only crate that knows both sides and wires them.

---

## 3. Architecture: ports and adapters

BatleHub uses the *hexagonal* (ports-and-adapters) pattern. Every external
dependency is hidden behind a trait defined in `crates/core/src/ports/`.

| Trait | Description | Primary implementation |
|-------|-------------|----------------------|
| `RegistryClient` | Upstream registry HTTP protocol | `crates/adapters/src/registry/*.rs` |
| `StorageBackend` | Artifact blob store (read/write/delete) | `crates/adapters/src/storage/` |
| `LocalRegistryBackend` | Index for privately published packages | `crates/adapters/src/local_registry/postgres.rs` |
| `PackageRepository` | Audit log and proxy metadata (Postgres) | `crates/adapters/src/db/postgres.rs` |
| `QuotaRepository` | Publish quota tracking | `crates/adapters/src/db/quota.rs` |
| `ArtifactMetaRepository` | Cache TTL / access-time tracking | `crates/adapters/src/db/artifact_meta.rs` |
| `CacheStore` | Metadata cache (memory / Postgres / Redis) | `crates/adapters/src/cache/` |
| `AuthProvider` | Token / OIDC / Kubernetes / Actions-OIDC validation | `crates/adapters/src/auth/` |

**Rule**: `crates/core` must never import from `crates/adapters` or `crates/web`.
Tests inside `core` use in-memory mocks, not the Postgres implementations.

---

## 4. Request lifecycle

```
HTTP request
  │
  ▼
actix-web middleware  (AuthMiddlewareFactory → AuthIdentity extractor)
  │
  ▼
Handler  (crates/web/src/handlers/proxy/<registry>.rs)
  │  extracts: AuthIdentity, RegistryMap, web::Data<Arc<ProxyService>>
  ▼
ProxyService::proxy(PackageId)
  │  checks: RegistryPolicy (RBAC rules, firewall_only, release-age-gate, …)
  │  checks: cache (CacheStore + StorageBackend)
  ▼
RegistryClient::fetch_artifact(PackageId)   ← upstream HTTP call
  │
  ▼
StorageBackend::store()                     ← persist to filesystem / S3
  │
  ▼
HTTP response streamed back to client
```

For **local/hybrid registries**, the publish path goes through
`LocalRegistryService::publish()` → `LocalRegistryBackend::publish()` →
`StorageBackend::store()`. Quota is checked and recorded between the two.

### PackageId conventions

`PackageId { registry, name, version, artifact }` is the cache key and data
carrier between the web layer and adapters. Conventions vary by ecosystem —
see `docs/adding-a-registry.md` for the full mapping table.

---

## 5. Database and migrations

Migrations live in `crates/adapters/migrations/` as numbered SQL files and are
registered in `crates/adapters/src/migrations.rs` using the `mig!()` macro.
They run automatically on startup via `sqlx::Migrate`.

**Important**: `sqlx::query!()` macros are disabled. The project patches
`sqlx-macros` with a no-op stub to avoid pulling in `sqlx-mysql` which carries
RUSTSEC-2023-0071 (an unfixed RSA vulnerability). All database queries use the
runtime API instead:

```rust
// Correct
sqlx::query("SELECT ... WHERE id = $1")
    .bind(id)
    .fetch_one(&pool)
    .await?;

// Will not compile — do not use
sqlx::query!("SELECT ...", id).fetch_one(&pool).await?;
```

When adding a new migration:
1. Create `crates/adapters/migrations/00N_description.sql`.
2. Add `mig!(N, "description", "../migrations/00N_description.sql")` to
   `crates/adapters/src/migrations.rs` (keep them in order).

---

## 6. Adding a new feature — checklist

### New registry type (proxy-only)

See `docs/adding-a-registry.md` for the full step-by-step walkthrough.
Short version:

- [ ] `crates/adapters/src/registry/<name>.rs` — implement `RegistryClient`
- [ ] `crates/adapters/Cargo.toml` — add `registry-<name> = []` feature, include in `default`
- [ ] `crates/adapters/src/registry/mod.rs` — export under `#[cfg(feature = "registry-<name>")]`
- [ ] `crates/config/src/schema.rs` — add `"<name>"` to the `validate()` match arm
- [ ] `server/src/main.rs` — add arm to `build_registry_client()`
- [ ] `crates/web/src/handlers/proxy/<name>.rs` — HTTP handler(s)
- [ ] `crates/web/src/lib.rs` — register routes in `collect_routes()`

### New DB-backed feature

- [ ] `crates/adapters/migrations/00N_<feature>.sql` — migration
- [ ] `crates/adapters/src/migrations.rs` — register it
- [ ] `crates/core/src/ports/<feature>.rs` — port trait
- [ ] `crates/core/src/ports/mod.rs` — re-export
- [ ] `crates/adapters/src/db/<feature>.rs` — Postgres implementation
- [ ] `crates/adapters/src/db/mod.rs` — export
- [ ] Wire the repository into `server/src/main.rs`

### New admin API endpoint

- [ ] `crates/web/src/handlers/back_office/<feature>.rs`
- [ ] Call `require_admin(&identity)?` at the start of every handler
- [ ] Register routes in `collect_routes()` — most-specific paths first
  (actix-web matches in registration order; `DELETE /quota/{reg}/{user}` must
  appear **before** `DELETE /quota/{reg}`)
- [ ] Add `pub mod <feature>;` to `crates/web/src/handlers/back_office/mod.rs`

---

## 7. Running tests

BatleHub has four layers of tests, each trading breadth for depth. Run them in order when you want full confidence; run just the first two for fast feedback.

### Layer 0 — unit tests (no external dependencies)

```bash
cargo test                      # all crates
cargo test -p batlehub-core     # domain logic only
cargo test -p batlehub-adapters # I/O adapter implementations
```

Every module keeps its unit tests at the bottom of the same source file under `#[cfg(test)]`. These tests use in-process mocks and stubs only — no database, no HTTP server, no network.

**What they validate:** Pure business logic — publish rules, quota checks, RBAC decisions, cache-key formatting, wire-format parsing, JWT claim evaluation. They run in milliseconds and must always pass on any developer machine.

#### Auth provider unit tests

Auth providers expose a `for_testing` constructor that injects a pre-loaded JWKS so tests never hit the network. Each module's `#[cfg(test)]` block covers:

- Token parsing and claim extraction
- Role elevation and group assignment via rule evaluation
- Condition matching (glob and regex patterns, auto-detection)
- Group template rendering
- Error paths: expired tokens, unknown signing keys, malformed headers

The `actions-oidc` provider additionally exposes `for_testing_stale`, which backdates the JWKS cache by more than `JWKS_MIN_REFRESH` so the cache-refresh path can be exercised without sleeping.

---

### Layer 0.5 — adapter integration tests (mockito HTTP, no external services)

```bash
cargo test -p batlehub-adapters --test actions_oidc
cargo test -p batlehub-adapters --test selfhosted
```

`crates/adapters/tests/` contains integration tests for adapters that need an HTTP server but no database or object storage. Tests use **mockito** (`mockito::Server::new_async()`) to spin up in-process HTTP servers.

**What they validate:** The full bootstrap and request cycle for network-facing adapters — OIDC discovery fetch, JWKS retrieval, provider construction failure paths, and end-to-end JWT authentication. Each test file covers one adapter family:

| File | What it covers |
|------|---------------|
| `actions_oidc.rs` | GitHub/Forgejo Actions OIDC: discovery → JWKS → JWT auth round-trip, error paths (5xx, malformed JSON), TOML config round-trip |
| `selfhosted.rs` | Self-hosted registry HTTP options: bearer forwarding, basic auth, TLS |

No external services are needed. These tests run offline and are included in `task coverage` automatically.

---

### Layer 1 — web integration tests (in-memory backends, no PostgreSQL)

```bash
cargo test -p batlehub-web
```

`crates/web/tests/integration.rs` (~6 000 lines) spins up a real actix-web application using `actix_web::test::init_service` and in-memory backends (no Postgres, no S3). It sends actual HTTP requests and asserts on status codes, headers, and JSON bodies.

**What they validate:**
- All proxy handlers across every registry type — correct URL routing, wire-format parsing, upstream passthrough, cache behaviour, and error mapping.
- Auth middleware — anonymous fallback, bearer token resolution, role mapping.
- Rate-limit middleware — per-user buckets, per-group buckets, warn/block modes, 429 responses.
- Back-office admin API — package listing, block/unblock, audit log, quota management, yank/unyank.
- Local registry publish/pull cycle — three-phase commit, quota enforcement, ownership checks.

These tests cover the largest surface area and run without any external services. They are the primary regression net for handler changes.

---

### Layer 2 — example structure tests (no network)

```bash
cargo test -p batlehub-examples --test structure
```

`crates/examples/tests/structure.rs` is a single static-analysis test (`all_examples_are_complete`) that iterates over all 12 example directories and asserts:
- Required files are present (`mise.toml`, `README.md`, a start script, a config file).
- TOML and JSON files parse without error.
- Config files reference the expected proxy URL placeholder.
- Shell scripts carry a proper shebang line.

**What they validate:** That every shipped example is complete and well-formed before anyone runs it. Catches copy-paste omissions and accidental deletions immediately, without touching the network or running any package manager.

---

### Layer 3 — local registry upload/pull cycle (no network, curl only)

```bash
cargo test -p batlehub-examples --test local_registry
```

`crates/examples/tests/local_registry.rs` starts a genuine actix-web batlehub proxy in `RegistryMode::Local` with fully in-memory backends (no PostgreSQL, no upstream registries) and runs an end-to-end publish → download cycle for every publish-capable registry type.

| Test | Publish endpoint | Download check |
|------|-----------------|---------------|
| `local_npm_publish_pull` | `PUT /proxy/{reg}/{name}` | packument + tarball |
| `local_cargo_publish_pull` | `PUT /proxy/{reg}/api/v1/crates/new` | `.crate` download |
| `local_go_publish_pull` | `PUT /proxy/{reg}/{module}@v/{ver}.zip` | list, `.mod`, `.zip` |
| `local_rubygems_publish_pull` | `POST /proxy/{reg}/api/v1/gems` | `.gem` download |
| `local_composer_publish_pull` | `POST /proxy/{reg}/api/upload` | p2 metadata + dist |
| `local_maven_publish_pull` | `PUT /proxy/{reg}/maven2/{path}` | artifact download |
| `local_openvsx_publish_pull` | `PUT /proxy/{reg}/{pub}.{name}/{ver}/vsix` | vsix download |
| `local_terraform_module_publish_pull` | `POST /proxy/{reg}/v1/modules/{ns}/{name}/{prov}/{ver}` | versions + artifact |

**What they validate:** That the full publish → store → serve pipeline works end-to-end for each ecosystem's wire format (binary framing for cargo, TAR+gzip for rubygems, ZIP for composer and goproxy, etc.) without any network dependency or package-manager tooling. These tests are the first line of defence when touching `LocalRegistryService`, storage backends, or registry-specific handlers.

---

### Layer 4 — smoke tests against example apps (requires mise + language runtimes)

```bash
cargo test -p batlehub-examples --test smoke
```

`crates/examples/tests/smoke.rs` copies each example into a temp directory, runs `mise install` to pull language runtimes, starts the example application, and curls it. Tests that hit the network skip gracefully when the required tool is not available.

**What they validate:**

| Group | Tests | What is verified |
|-------|-------|-----------------|
| MockProxy routing | `proxy_curl_endpoints` | curl hits a hand-rolled TCP proxy; `X-Served-By: mock-proxy` header is returned |
| Downstream tool routing | `vsix_downloads_via_proxy`, `github_asset_download_via_proxy` | curl downloads pass through the mock proxy log |
| Real app startup | `api_npm`, `api_go`, `api_python`, `api_ruby`, `api_composer_console`, `api_maven_spring`, `api_maven_quarkus` | example app starts, HTTP `/` returns "hello" |
| mise proxy routing | `mise_install_tasks_route_through_proxy` | package-manager requests are logged in the mock proxy |

These are the highest-cost tests and are intended for CI environments with full language tooling. They confirm that the shipped examples actually work.

---

### Layer 5 — real proxy against live upstreams (requires network + language runtimes)

```bash
cargo test -p batlehub-examples --test real_proxy
```

`crates/examples/tests/real_proxy.rs` starts a genuine batlehub actix-web proxy with in-memory backends and **real upstream registry HTTP clients**, then uses actual package-manager tools to fetch packages through it.

| Test | Tool / ecosystem | What is verified |
|------|-----------------|-----------------|
| `real_proxy_npm_api` | Node / npm | npm example installs deps via proxy, app starts, GET `/` → "hello" |
| `real_proxy_cargo_fetch` | cargo | `cargo fetch` resolves a crate through the proxy |
| `real_proxy_go_api` | Go | Go example builds + runs with `GOPROXY` pointing at the proxy |
| `real_proxy_pypi_api` | Python / pip | Python example installs via proxy, app starts |
| `real_proxy_rubygems_api` | Ruby / bundler | Ruby example installs via proxy, app starts |
| `real_proxy_composer_console` | PHP / composer | composer install routes through proxy |
| `real_proxy_maven_spring_api` | Java / Maven | Spring Boot example builds via proxy, starts, GET `/` → "hello" |
| `real_proxy_maven_quarkus_api` | Java / Maven | Quarkus example builds via proxy, starts, GET `/` → "hello" |
| `real_proxy_terraform_init` | Terraform | `terraform init` downloads provider through proxy |
| `real_proxy_github_releases` | GitHub Releases | asset download resolves through proxy |
| `real_proxy_openvsx_download` | Open VSX | extension download resolves through proxy |
| `real_proxy_vscode_marketplace_download` | VS Code Marketplace | extension download resolves through proxy |

**What they validate:** True end-to-end correctness of each `RegistryClient` implementation against the live upstream protocol — caching headers, redirect handling, tarball streaming, checksum verification. These are network-dependent and will skip or fail gracefully when the required toolchain or network is unavailable.

---

### Coverage

The project enforces a minimum of **80% line coverage** measured by `cargo-llvm-cov`. Both tasks require PostgreSQL and MinIO (started automatically from the `Taskfile`):

```bash
# Generate an HTML report (opens at target/llvm-cov/html/index.html)
task coverage

# Enforce the 80% threshold — fails the build if coverage drops below it
task coverage-check
```

To run coverage manually without the Task runner:

```bash
# Install the tool once
cargo install cargo-llvm-cov

# Base workspace coverage (unit tests)
cargo llvm-cov --no-report --workspace

# Add each integration test that needs separate invocation
cargo llvm-cov --no-report -p batlehub-adapters --test actions_oidc
cargo llvm-cov --no-report -p batlehub-adapters --test pg_cache     # needs DATABASE_URL
cargo llvm-cov --no-report -p batlehub-adapters --test local_registry  # needs DATABASE_URL
cargo llvm-cov --no-report -p batlehub-adapters --test storage_router  # needs DATABASE_URL
cargo llvm-cov --no-report -p batlehub-adapters --features storage-s3 --test s3_storage  # needs S3

# Generate the report
cargo llvm-cov report --html --output-dir coverage/html
```

The workspace-level `[workspace.metadata.llvm-cov]` config excludes `server/src/main.rs` (startup wiring only) from the report. Every other module is expected to have at least some exercised lines.

**Adding a new integration test to coverage**: integration tests that need no external service (mockito-only, like `actions_oidc`) must be listed explicitly in both the `coverage` and `coverage-check` tasks in `Taskfile.yml` — `cargo llvm-cov --workspace` does not pick up `crates/adapters/tests/*.rs` files automatically.

---

### Security audits

Run dependency vulnerability scans before shipping or merging security-sensitive changes:

```bash
# Rust — checks crates against the RustSec advisory database
task audit

# Frontend — checks npm packages against the npm advisory database
task ui:audit
```

`task audit` suppresses advisories that have no actionable fix via `.cargo/audit.toml`. Add a new entry there (with a justification comment) when an advisory is known and accepted rather than silencing the whole tool. `task ui:audit` exits non-zero when high-severity vulnerabilities are found; use `npm audit --audit-level=high` manually if you need to ignore lower-severity findings during development.

---

## 8. Code conventions

- **No wildcard imports** — write every imported name explicitly (`use foo::{Bar, Baz}`, not `use foo::*`). Wildcard imports hide where names come from, make unused-import warnings silent, and cause surprise breakage when an upstream crate adds a new symbol that clashes with a local one. The only accepted exception is `#[cfg(test)] use super::*` inside a same-file test module.
- **No `sqlx::query!()` macros** — use the runtime API (see §5).
- **No comments that describe what the code does** — only add one when the
  *why* is non-obvious (a hidden constraint, a workaround, a subtle invariant).
- **Error type per layer**: `CoreError` in `core`, `AppError` in `web`.
  Map at the boundary: `AppError::from(CoreError)` in `crates/web/src/error.rs`.
- **HTTP status for infrastructure errors**: storage and DB errors map to
  `503 Service Unavailable` so load-balancers can retry on another instance.
  Logic errors (not-found, conflict) map to the appropriate 4xx.
- **Quota rollback is best-effort** (`tokio::spawn`). Errors are logged via
  `tracing::error!` but do not propagate to the caller. See §9 for the
  accepted race condition.
- **Route registration order matters**. In `collect_routes()`, register more
  specific paths (longer or with more literal segments) before catch-alls.

---

## 9. Known limitations and accepted trade-offs

### Quota enforcement has a TOCTOU race (accepted)

`QuotaService::check_and_record_publish()` reads the current usage with
`repo.get_usage()` and writes the new total with `repo.record_publish()` in
two separate SQL statements. There is no `SELECT FOR UPDATE` or advisory lock.

**Consequence**: two concurrent publish requests from the same user can both
read the same stale counter, both pass the limit check, and both record —
ending up one package (or one upload's worth of bytes) over the configured
hard limit. The overshoot is bounded to the number of in-flight concurrent
publishes from the same user, which is typically one or two in practice.

**Why it is accepted**: adding database-level serialization (a `SELECT FOR
UPDATE` on the `quota_usage` row) would require restructuring
`PgQuotaRepository` and introducing explicit transactions across the check and
record steps, adding latency to every publish. The quota feature is intended as
a safeguard against accidental runaway usage, not a strict financial billing
boundary — a transient overshoot of one version is acceptable.

**If you need strict enforcement**: wrap `get_usage` and `record_publish` in a
single `BEGIN … SELECT … FOR UPDATE … UPDATE … COMMIT` transaction inside
`PgQuotaRepository`, and update `QuotaRepository::check_and_record_publish`
(or introduce a new method) to execute both steps atomically.

---

### `LocalRegistryBackend` uses a two-phase publish

`LocalRegistryService::publish()` uses a three-step protocol:

1. `backend.publish()` — inserts the row with `status = 'pending'` (invisible to readers).
2. `storage.store()` — persists the artifact bytes.
3. `backend.commit_publish()` — promotes the row to `status = 'published'`.

In-process errors at any step trigger a best-effort cleanup (`remove_version`,
`storage.delete`, quota rollback). A hard crash between steps 1 and 2 leaves an
orphaned *pending* row; a crash between steps 2 and 3 leaves a pending row plus
the artifact in storage.

Pending rows are safe: they are invisible to `get_versions` and `exists`, so
they do not cause 404s. They are cleaned up automatically by calling
`LocalRegistryBackend::cleanup_pending(older_than)`, which deletes pending rows
older than the given duration. Wire this up to a startup sweep or a
periodic maintenance task; a threshold of one hour is a safe default.

To recover manually: call `cleanup_pending` or run:

```sql
DELETE FROM local_packages WHERE status = 'pending';
```
