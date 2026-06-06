# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

All task runner commands use `task` (Taskfile). The key ones:

```bash
# Build / check
cargo build --workspace
cargo check --workspace

# Tests
cargo test --workspace                              # all unit + in-process integration tests
cargo test --package batlehub-web nuget            # single package, filter by name
cargo test --package batlehub-adapters --lib rbac  # lib-only tests (no integration)
cargo test -p batlehub-cli --test integration      # CLI integration tests (subprocess binary)

# Linting (CI fails on any warning)
cargo clippy --workspace -- -D warnings
cargo fmt --all --check

# Format
cargo fmt --all

# Coverage (requires Podman — starts Postgres + MinIO)
task coverage        # HTML report in coverage/html/
task coverage-check  # fails if line coverage < 80%

# Integration tests that need real Postgres (via Podman)
task test:pg-cache
task test:pg-local-registry

# Integration test that needs real S3/MinIO
task test:s3

# Run server (requires Postgres)
cargo run -p batlehub-server -- --config config.example.toml

# Frontend
cd ui && npm run dev          # Vite dev server (proxies /api → localhost:8080)
cd ui && npm run generate     # regenerate TypeScript client from ui/openapi.json
task dump-spec                # refresh ui/openapi.json from running server

# Fuzz (nightly)
task fuzz TARGET=fuzz_rbac_evaluate MAX_TIME=30
```

## Architecture

### Crate layout

```
crates/core      — domain: entities, ports (traits), rules, services (no I/O)
crates/config    — TOML schema (AppConfig, RegistryConfig, …) + loader
crates/adapters  — infrastructure: HTTP registry clients, Postgres, Redis, S3, in-memory impls
crates/web       — actix-web handlers, middleware, extractors
crates/examples  — integration test helpers and smoke/real-proxy test binaries
server/          — binary: wires everything together, no domain logic
cli/             — batlehub-cli binary: clap commands + reqwest API client + ratatui TUI
ui/              — Vue 3 + Vite SPA (TypeScript, Tailwind, shadcn-inspired components)
```

The dependency direction is strict: `core` ← `adapters` ← `web` ← `server`. The `config` crate is read only by `server` and `web`.

### Request lifecycle

1. **Auth middleware** (`crates/web/src/middleware/auth.rs`) — iterates `Vec<Arc<dyn AuthProvider>>`, resolves to an `Identity`, stores it in request extensions. `X-NuGet-ApiKey` is normalised to `Authorization: Bearer` in `extractors::raw_auth_from_request` before providers see it.

2. **Handler** (`crates/web/src/handlers/proxy/<registry>.rs`) — calls `require_registry_type` + `require_local_mode` guards, then either:
   - **Local/hybrid mode**: reads from `LocalRegistryService` (database-backed)
   - **Proxy/hybrid fallthrough**: delegates to `ProxyService::handle`

3. **ProxyService** (`crates/core/src/services/proxy.rs`) — acquires a short-lived read lock on `HotConfig` to clone the `Arc<RegistryClient>` and `Arc<RegistryPolicy>`, then:
   - Resolves metadata (cache-first, stale-on-error optional)
   - Evaluates rules (`RbacRule`, `DenyLatestRule`, `BlockListRule`, `ReleaseAgeGateRule`)
   - Streams artifact from upstream (or serves from storage cache)

### Hot reload

`HotConfig` is wrapped in `Arc<RwLock<HotConfig>>` (`HotConfigLock`). Handlers snapshot the parts they need by cloning `Arc<>` before any `await`. Config reload replaces the entire inner value atomically. In-flight requests finish with the old snapshot.

### Adding a new registry adapter

1. **`crates/adapters/src/registry/<name>.rs`** — implement `RegistryClient` (`registry_type`, `resolve_metadata`, `fetch_artifact`; optionally `list_versions`, `search_packages`). Use `NugetRegistryClient` as a reference.
2. **`crates/adapters/src/registry/mod.rs`** — add `#[cfg(feature = "registry-<name>")] pub mod <name>` entry.
3. **`crates/web/src/handlers/proxy/<name>.rs`** — actix-web handlers; use `proxy_stream`, `require_registry_type`, `require_local_mode`, `content_type_for` helpers from `common.rs`.
4. **`crates/web/src/lib.rs`** — register routes with `cfg.service(...)` and add the `utoipa` tag.
5. **`crates/config/src/schema.rs`** — allow `"<name>"` in the `RegistryConfig` type field.
6. **`server/src/main.rs`** — instantiate the client and wire it into `HotConfig`.
7. **`ui/src/config/registryTypes.ts`** — add a `RegistryTypeDef` entry with setup snippets.

For **local/hybrid mode**, additionally implement `get_<name>_versions` (and related helpers) in `crates/core/src/services/local_registry.rs`, following the existing `get_nuget_versions` / `get_maven_versions` patterns.

### Test patterns

- **Unit tests**: `#[cfg(test)] mod tests` inside the same file. Registry adapter tests use `mockito::Server` to mock HTTP upstreams.
- **Integration tests** (in-process): `crates/web/tests/integration.rs` — spins up a full actix-web app with `InMemoryPackageRepository`, `InMemoryStorageBackend`, `InMemoryCacheStore`, and `FixedRegistry`. Each registry type has a `make_local_<type>_app(mode: RegistryMode)` factory and a helper to build publish payloads.
- **CLI integration tests**: `cli/tests/integration.rs` — builds the CLI binary then invokes it as a subprocess against an in-memory actix-web server (same pattern as the web tests). Uses `env!("CARGO_BIN_EXE_batlehub-cli")` so cargo builds the binary automatically before running. See architecture note below about in-memory store separation.
- **External integration tests**: `crates/adapters/tests/pg_*.rs`, `s3_storage.rs` — require real Postgres/MinIO (run via `task test:pg-*` / `task test:s3`).
- **Fuzz targets**: `fuzz/fuzz_targets/` — run with nightly via `task fuzz`.

#### CLI test architecture — in-memory store separation

`InMemoryLocalRegistry` (used by `LocalRegistryService` — publish/yank/delete) and `InMemoryPackageRepository` (used by `AdminService` — package list/block) are **separate** in-memory stores. In Postgres they share the same tables, so this separation only matters in tests.

Consequence: packages published via the local-registry HTTP endpoint do **not** appear in `GET /api/v1/packages` (which queries `AdminService`). Use `TestServer::seed_package()` to inject entries directly into `InMemoryPackageRepository` when testing commands like `package list`. To verify yank/unyank/delete state, query the registry-specific endpoints (e.g. the NuGet flat-index at `/proxy/{reg}/nuget/v3/flat/{id}/index.json`) rather than `package list`.

Coverage is enforced at 80% lines. Excluded paths (DB adapters, some registry clients, auth/OIDC handlers, server main) are listed in the `COVERAGE_EXCLUDE` variable in `Taskfile.yml`.

### Storage keys

Artifacts are stored with `artifact_storage_key(registry, name, version)` → `"{registry}/{name}/{version}"`. Maven uses `maven_artifact_storage_key` for multi-file artifacts. These keys are stable and shared between `ProxyService` (cache writes) and `LocalRegistryService` (local reads).

### Database migrations

SQL migrations live in `crates/adapters/migrations/`. They are embedded via `crates/adapters/src/migrations.rs` using a `mig!` macro (avoids the `sqlx::migrate!` macro which pulls in `sqlx-mysql` → `rsa` RUSTSEC advisory). When adding a migration, increment the sequence number and add a `mig!` entry to `embedded_migrator()`.

### Security constraints

`sqlx-macros` and `sqlx-mysql` are patched to empty stubs in `[patch.crates-io]` (Cargo.toml) to remove the `rsa` crate (RUSTSEC-2023-0071) from the dependency tree. Do not add `features = ["macros"]` to the `sqlx` dependency or re-enable `sqlx-macros`/`sqlx-mysql`.

`aws-sdk-s3` and `aws-config` use `default-features = false` to avoid `legacy-rustls-ring` (RUSTSEC-2026-0098/0099/0104). Do not enable default features on these crates.

### Frontend

The Vue SPA lives in `ui/`. The TypeScript API client (`ui/src/client/`) is auto-generated from `ui/openapi.json` via `npm run generate` — do not edit it manually. Setup snippets for the Setup Guide are defined in `ui/src/config/registryTypes.ts` as `REGISTRY_TYPE_DEFS`.

#### Sync SDK

When the backend api is updated, the openapi spec/sdk must be resynced:

1. Generate the swagger spec : `task dump-spec` (copies to `ui/openapi.json`)
2. Generate the Typescript client: `task ui:generate` (reads from `ui/openapi.json`, outputs to `ui/src/client/`)

The generated use fetch and include the full model spec, so any changes to the API will be reflected in the generated client. The client is used in the frontend and can also be imported by external users who want a typed API client for the server.

## Docs

The doc live in two places:

- **Docs site** (batleforc.git.batleforc.fr/batlehub) — user-facing documentation, setup guides, API reference (generated from `ui/openapi.json`), architecture overview. Markdown files in `docs/` are copied to the static site.
- **Base docs** (/docs/) - Internal documentation and helper for some case
- **Roadmap** (ROADMAP.md) — high-level plans and progress tracking for features and registry types.
- **Code comments** - doc directly in the codebase, especially for complex logic like the request lifecycle, hot reload, and registry client implementations. Use `///` for public items and `//` for internal comments.
