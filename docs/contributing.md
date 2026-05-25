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
| `AuthProvider` | Token / OIDC / Kubernetes validation | `crates/adapters/src/auth/` |

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

```bash
# All unit tests (no database required)
cargo test

# Single crate
cargo test -p batlehub-core

# Integration tests (require PostgreSQL)
DATABASE_URL=postgresql://user:pass@localhost/batlehub_test cargo test -p batlehub-web

# Coverage (requires cargo-llvm-cov)
cargo llvm-cov --html
```

Tests for a module live at the bottom of the same file under `#[cfg(test)]`.
Integration tests for the web layer are in `crates/web/tests/integration.rs`.

---

## 8. Code conventions

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
