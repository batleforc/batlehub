# Security survey — batlehub

**Date:** 2026-06-12
**Scope:** Source-code review of the high-risk surfaces for a registry proxy/cache:
authentication & authorization, path handling / storage keys, SQL injection, XML parsing
(XXE), and SSRF.
**Method:** Manual read of the auth middleware, storage backends, core publish flow, registry
adapters, and XML parsers. This is a point-in-time survey, not an exhaustive audit.

---

## Summary

| # | Finding | Severity |
| --- | --- | --- |
| 1 | Path traversal in the filesystem storage backend (`key_to_path`) | **High** |
| 2 | Inconsistent input validation across registry adapters | **High (systemic)** |
| 3 | `validate_version` is opt-in | Low |

Overall the codebase is in good shape: SQL is parameterized, the auth middleware fails closed,
the XML parsers are not XXE-exploitable, and several authorization edge cases are handled
deliberately. The one material issue is **path traversal in the filesystem storage backend**,
made reachable by **inconsistent per-adapter input validation**. Both are addressed by the same
defense-in-depth fix.

---

## Finding 1 — Path traversal in the filesystem storage backend (High)

**Location:** `crates/adapters/src/storage/filesystem.rs:31`

```rust
fn key_to_path(&self, key: &str) -> PathBuf {
    let rel = key.replace(':', "__");
    self.root.join(format!("{rel}.dat"))   // join() does NOT collapse ".."
}
```

`PathBuf::join` appends path components without normalizing them; the OS resolves `..` at access
time. A storage key containing a `../` segment therefore escapes the configured storage `root`.

This function is the shared sink for every filesystem operation:

- `store`   (`filesystem.rs:39`) — arbitrary file **write** (content attacker-controlled, suffix forced to `.dat`)
- `retrieve` (`filesystem.rs:56`) — arbitrary file **read** (limited to `*.dat`)
- `exists`  (`filesystem.rs:80`)
- `delete`  (`filesystem.rs:83`) — arbitrary file **delete**
- `delete_by_prefix` (`filesystem.rs:166`)

### Reachability

Storage keys are built directly from package `name` / `version` / `filename`, none of which are
guaranteed to be free of path separators:

- `artifact_storage_key(registry, name, version)` → `local:{registry}/{name}/{version}`
  (`crates/core/src/services/local_registry/mod.rs:133`)
- `maven_artifact_storage_key(registry, name, version, filename)`
  (`mod.rs:139`)
- `PackageId::cache_key()` → `{registry}/{name}/{version}[/{artifact}]`
  (`crates/core/src/entities/package.rs:40`)

**Primary vector — local publish (authenticated):** `PublishRequest.name` / `version` flow into
`artifact_storage_key` with no charset validation (`crates/core/src/services/local_registry/publish.rs:163`).
A publisher sending `name = "../../../../tmp/evil"` writes a `.dat` file outside `root`. This is an
authenticated arbitrary-file-write (constrained to the `.dat` extension and to paths the server
process can write).

**Secondary vector — Maven proxy (lower impact):** `parse_maven_path` splits the request path on
`/` and only filters empty segments, so a lone `..` survives as the `version` segment
(`crates/web/src/handlers/proxy/maven/routing.rs:64`). The dotted group-id join collapses multiple
`..` into `.` separators, so this vector yields only a single level of traversal — but it is on a
cache/read path.

### S3 backend

`crates/adapters/src/storage/s3/backend.rs` uses the same key shape. `..` in an S3 object key is
not a filesystem traversal, but it still permits cross-namespace object collisions / cache
poisoning, so the same key guard should apply.

---

## Finding 2 — Inconsistent input validation across registry adapters (High, systemic)

Path-traversal protection exists for *some* registries but not others, and there is no central
validation routine. This is why Finding 1 is reachable.

**Guarded today:**
- **Composer** — `parse_p2_package_name` restricts names to `[a-z0-9A-Z_.-]` and rejects `../`
  (`crates/web/src/handlers/proxy/composer/metadata.rs:247`), with tests
  `parse_p2_path_traversal_rejected` (`metadata.rs:334`) and
  `validate_version_param("../../etc/passwd")` (`composer/upload.rs:181`).
- **PyPI** — names normalized via `normalize_name`
  (`crates/web/src/handlers/proxy/pypi/simple.rs:80`).

**Not guarded:**
- **Maven, npm, NuGet, RubyGems, OpenVSX, GoProxy** — no equivalent `..` / separator rejection
  before the storage key is built.

There is **no shared `validate_package_name`**, and the *"Adding a new registry adapter"* checklist
in `CLAUDE.md` does not mention input validation — so new adapters inherit the gap by default.

---

## Finding 3 — `validate_version` is opt-in (Low)

**Location:** `crates/core/src/services/local_registry/mod.rs:28`

`validate_version` only constrains the version string when `policy.enforce_semver` or
`policy.version_pattern` is configured. With both unset, the version is unconstrained and flows
straight into the storage key. This is acceptable *if* the storage layer is hardened (Finding 1),
but today the version path is effectively unvalidated for traversal in the default configuration.

---

## What is solid

- **SQL injection** — queries use parameterized `.bind()` throughout
  (e.g. `crates/adapters/src/local_registry/postgres.rs:31`); no string-built SQL was found. The
  deliberate avoidance of `sqlx` macros does not weaken this.
- **Authentication middleware** — `crates/web/src/middleware/auth.rs` iterates providers; a provider
  **error** is logged and falls through to the next, ultimately to `Identity::anonymous()`. This is
  fail-closed for authorization (least privilege), not a bypass.
- **XXE** — the XML parsers (`quick-xml`) in `crates/adapters/src/sbom/extractor/{maven,nuget}.rs`
  and `crates/web/src/handlers/proxy/maven/routing.rs` do not resolve external entities or fetch
  DTDs, so they are not XXE-exploitable.
- **Authorization edge cases** — team-visibility checks **deny** on a missing namespace claim rather
  than failing open (`crates/core/src/services/local_registry/mod.rs:74`), and new-version publishes
  inherit existing visibility rather than resetting to public, with DB errors propagated rather than
  defaulted (`publish.rs:138`).
- **Password hashing** — Argon2 (`argon2` crate, `std` feature). The auth path logs only
  `user_id` / `role` at debug level, not credentials.

---

## Recommended remediation (defense in depth)

1. **Harden the chokepoint.** In `key_to_path` (and the S3 `obj_key` builder), reject any key that
   contains a `..` path segment, a leading `/` or `\`, or a NUL byte — or, equivalently, normalize
   the key and reject anything whose result escapes `root`. A single guard covers every registry,
   present and future, regardless of per-adapter validation.

2. **Add a shared `validate_package_name`** in `crates/core`, called from `PublishRequest`
   validation and reused by the proxy handlers, so malformed input is rejected at the edge with a
   clean `400` rather than failing deep in the storage layer.

3. **Make version validation always reject separators** even when semver enforcement is off (cheap,
   independent of the configurable policy).

4. **Update the adapter checklist** in `CLAUDE.md` ("Adding a new registry adapter") to require
   name/version validation, so the gap is not reintroduced.

5. **Add a regression test** reproducing the publish traversal (`name = "../../etc/x"`) against the
   filesystem backend, asserting the write is rejected and stays within `root`.

### Suggested priority
Finding 1 + Finding 2 share remediation steps 1–2 and should be fixed together; they are the only
material issues. Findings 3–5 are hardening/process improvements.
