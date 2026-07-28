# Add `jetbrains-marketplace` registry kind (JetBrains plugin marketplace proxy, local/hybrid/proxy)

## Context

BatleHub already has a `jetbrains` registry kind, but it is only a path-proxy for IDE installer archives from `download.jetbrains.com` (via `PathProxyRegistryClient`, proxy-only). The user wants a proxy-cache for the **JetBrains Marketplace** (`plugins.jetbrains.com`) — the plugin ecosystem — with **local, hybrid, and proxy** modes. This is a metadata-API adapter like `openvsx`/`vscode-marketplace`, not a path proxy, so it gets a **new** kind: `jetbrains-marketplace`. The existing `jetbrains` kind stays untouched.

Decisions confirmed with the user:
1. New `RegistryKind::JetbrainsMarketplace`, wire string `jetbrains-marketplace`, default upstream `https://plugins.jetbrains.com`.
2. **Full marketplace emulation** — the IDE-facing surface must be complete enough that `idea.plugins.host=<proxy base>` fully replaces plugins.jetbrains.com (search, compatible-updates, meta.json blobs, downloads), plus `updatePlugins.xml` custom-repo XML for the additive `idea.plugin.hosts` flow.
3. **Marketplace-compatible publish**: `POST .../api/updates/upload` multipart (`file`, `xmlId`|`pluginId`, `channel`, `isHidden`) with `Authorization: Bearer`, so JetBrains' plugin-repository-rest-client / Gradle tooling can publish to local/hybrid registries.

Verified facts: `actix-multipart`, `zip`, `quick-xml` are already deps of `crates/web` (pypi/nuget publish + maven XML precedents); `quick-xml` is an optional workspace dep of `crates/adapters` already activated by default features; `new_http_client` follows redirects (`Policy::limited(10)`) so the `/plugin/download` → CDN redirect works without SSRF-guard changes; `config` `validate()` is predicate-driven (no change needed); `LocalRegistryBackend::list_package_names` exists; coverage gate (80%) does NOT exclude the new adapter, so mockito tests are load-bearing.

## Design summary

**Adapter** (`crates/adapters/src/registry/jetbrains_marketplace/` — `mod.rs`, `client.rs`, `models.rs`, `tests.rs`; layout ref `composer/`/`conda/` (verified: `nuget/` has **no** separate `tests.rs` — its tests are inline in `client.rs`, and its `models.rs` only holds `normalize_id`), code template `openvsx.rs`):
- `resolve_metadata` / `list_versions`: `GET {base}/plugins/list?pluginId={xmlId}` (XML; exact xmlId lookup in one hop; carries versions, since/until builds, name, vendor, date→`published_at`). Parse with a quick-xml event state machine (pattern: `parse_nuspec` in `crates/web/src/handlers/proxy/nuget/nuspec.rs`). `PackageMetadata.extra` carries the **full version list with per-version details** (`{resolved_version, name, vendor, description, versions: [{version, since_build, until_build, channel, date_ms}, …]}`) so the web layer can render every per-plugin endpoint shape from one cached entry (see offline-resilience design below).
- `fetch_artifact`: artifact `None`/`"plugin"`/`"plugin@{channel}"` → `GET {base}/plugin/download?pluginId={name}&version={version}[&channel=]` (reqwest follows redirect to CDN); artifact `"file/{fileName}"` → `GET {base}/files/{name}/{version}/{fileName}` (name/version carry upstream numeric ids verbatim for IDE `/files/` passthrough).
- `search_packages`: `GET {base}/api/searchPlugins?search={q}&max={limit}` (JSON) → `UpstreamPackage`.
- Constructor/auth/errors: openvsx template (`new_http_client(Some(10), opts)`, `basic_auth_get`, `to_registry_error`, `cache_control`).
- Feature: `registry-jetbrains-marketplace = ["dep:quick-xml"]`, added to `default`.

**Offline resilience — metadata is cached, not blindly forwarded.** Requirement: the proxy must keep working for already-seen plugins even if plugins.jetbrains.com disappears. Three layers:
1. **Per-plugin metadata** (`/plugins/list?pluginId=`, `/api/plugins/{id}`, `/api/plugins/{id}/updates`, both `meta.json` shapes): the adapter's `resolve_metadata` fetches `/plugins/list?pluginId={xmlId}` once and stores the **complete version list with per-version details** (version, since/until build, channel, name, vendor, description, date) in `PackageMetadata.extra`. Handlers resolve through the ProxyService metadata cache — `resolve_metadata_cached` (`crates/core/src/services/proxy/resolve.rs:15`) is already cache-first with TTL (registry `cache` config) and serves stale on **transient** upstream error (`CoreError::Registry(_)` only, not `NotFound`) when the runtime policy's `serve_stale_metadata` is set — the config field is named `serve_stale` (`crates/config/src/schema/registry.rs:391`, default **true**) and maps to `RegistryPolicy.serve_stale_metadata` (`crates/core/src/services/hot_config.rs:21`); beware `resolve.rs:45` does `unwrap_or(false)` when no policy exists, so tests must set an explicit policy — and render each response shape (XML plugin-list, plugin JSON, updates array, meta.json) from the cached `extra`. One upstream fetch feeds every per-plugin endpoint, and all of them keep working offline once a plugin has been seen. Requires a core addition — **verified: no existing method suffices**. Only `handle` (`handle.rs:19`) and `authorize_read` (`handle.rs:164`) are public; `resolve_metadata_cached` is `pub(super)` (`resolve.rs:15`) and takes a pre-resolved client/cache_key/ttl. Add a public `ProxyService::resolve_metadata_for(req: &ProxyRequest)` by factoring the prelude out of `handle` (`handle.rs:30-63`: `validate_coordinate` → hot-lock clone of `(client, policy, …)` → `cache_key = "meta:{…}"` → `ttl = policy.metadata_ttl`) — see step 3b.
2. **Artifacts** (`/plugin/download`, `/files/{p}/{u}/{fileName}`, `pluginManager` downloads): through `ProxyService` storage cache as before — already offline-capable after first fetch. `pluginManager?action=download&id&build` is implemented as: resolve latest compatible version from the (cached, stale-capable) metadata via `build_in_range` → `proxy_stream` with the concrete version — so it is cached by construction, no forward-stream.
3. **Query/list endpoints** (`/api/search/plugins`, `/api/searchPlugins`, `POST /api/search/updates/compatible`, `/api/search/aggregation/{field}`, `/feature/getImplementations`, comments, and the fixed blobs `pluginsXMLIds.json`, `jbPluginsXMLIds.json`, `brokenPlugins.json`, `IDE/extensions.json`): a new **cached-forward helper** (replaces the plain npm-audit-style forward): forward to upstream via `UpstreamMap` + shared `reqwest::Client`, but persist the response body in the `CacheStore` (`crates/core/src/ports/storage/cache_store.rs` — API is `get` / `set(key, entry, ttl)` / `invalidate` / `get_stale`; the write method is **`set`**, TTL is a parameter of `set`) as a synthetic `CacheEntry` (`PackageMetadata::minimal` with `extra = {body, content_type}` — `minimal` exists, `package.rs:78`), keyed by **`fwd:` + registry + path + sorted query params** (dedicated `fwd:` prefix so keys never collide with the `meta:` / `artifact:` namespaces) (volatile params like `uuid` dropped; the compatible-updates POST keys on build + sorted xmlIds hash), metadata TTL from registry cache config. On upstream error, serve the stale entry. The fixed-URL blobs are the high-value offline case (the IDE fetches them at startup); param-heavy searches benefit on repeat queries. **Wiring prerequisite (verified)**: `Arc<dyn CacheStore>` is not reachable from web handlers today (zero references in `crates/web/`; it lives only inside `ProxyService.cache`) — step 5-bis injects it via `configure_app`.

**Hybrid semantics**: per-plugin endpoints local-first, forward on `CoreError::NotFound`; search/compatible-updates merge local + best-effort upstream, deduped by xmlId (local wins); `pluginsXMLIds.json` union; `updatePlugins.xml` is local-content-only (404 in pure proxy mode via `require_local_mode`).

**Local ids**: `externalPluginId = xmlId`, `externalUpdateId = version` (string ids in our own generated URLs; IDE treats them as opaque — documented risk, integer-emulation fallback out of scope for v1).

**Publish** extracts the plugin descriptor from the archive (`.jar`: `META-INF/plugin.xml`; `.zip`: nested `*/lib/*.jar`), parses id/version/name/vendor/since/until/depends/change-notes with quick-xml; form `xmlId`/`pluginId` must match descriptor id (400 on mismatch, 422 if descriptor lacks id/version); validates xmlId with `validate_package_name` and version with `validate_path_safe` **at the edge**; responds 201 JSON `{id, pluginId, version, channel}`.

**`index_metadata` stored per version**:
```json
{"xmlId": "...", "version": "...", "name": "...", "description": "...", "vendor": "...",
 "sinceBuild": "233.0", "untilBuild": "241.*", "channel": "", "fileName": "...", "size": 0,
 "isHidden": false, "depends": ["..."], "changeNotes": "..."}
```
`isHidden` versions: publish maps `isHidden=true` → the existing `PublishRequest.unlisted` flag, whose semantics are exactly "excluded from all listings, downloadable by exact coordinate" and which is filtered for free by `filter_unlisted` inside `load_visible_versions` (`crates/core/src/services/local_registry/read.rs:409`). Keep `isHidden` in `index_metadata` only for meta.json rendering — no manual hidden-filtering in the eco module.

## Implementation steps (ordered for incremental compilation)

### 1. Core enum — `crates/core/src/entities/registry_kind.rs`
Add `JetbrainsMarketplace` variant + `ALL` + `as_str() => "jetbrains-marketplace"`. Do NOT add to `supports_local_mode` exclusion, `requires_explicit_upstream_in_proxy_mode`, or `is_path_addressed`. Extend the three predicate tests that enumerate kinds by hand and will break: `local_mode_support_matches_source_hosting_exclusion` (l.168), `only_deb_rpm_and_generic_require_explicit_upstream_in_proxy_mode` (l.179), `path_addressed_kinds_are_the_path_proxy_ones` (l.188). (`server` won't compile until step 4 — expected; keep `cargo check -p batlehub-core` green.)

### 2. Local-registry eco module — `crates/core/src/services/local_registry/eco_jetbrains.rs` (new)
Declare in `local_registry/mod.rs`. Following `eco_composer.rs`/`eco_nuget.rs` patterns (identity-aware via `load_visible_versions*`):
- `get_jetbrains_plugins(registry, build, channel, identity)` — via `backend.list_package_names`; **filter `yanked` explicitly in the eco module** (verified: `load_visible_versions` filters `unlisted` but NOT `yanked` — pattern `eco_composer.rs:18`; hidden comes free via `unlisted`, see design), filter channel (default Stable `""`), pick newest build-compatible version **by publish date (`published_at`/`date_ms`), not lexicographic `sort()`** — the existing ecos' string sort is wrong for arbitrary plugin version strings; returns typed entries the web layer renders.
- `get_jetbrains_versions(registry, xml_id, identity)` — wrapper over `load_visible_versions_or_not_found`.
- `get_jetbrains_compatible_updates(registry, xml_ids, build, identity)`.
- Pure `build_in_range(since, until, build) -> bool` — strip product prefix (`IU-241.x`→`241.x`), numeric dotted compare, `*` wildcard, open-ended until. Heavy inline unit tests (correctness-critical + coverage).

### 3. Adapter — `crates/adapters/src/registry/jetbrains_marketplace/` (new, per design above)
Plus `crates/adapters/src/registry/mod.rs` cfg-gated mod/pub-use pair and `crates/adapters/Cargo.toml` feature. Mockito tests: metadata latest+pinned, 404→NotFound, malformed XML→Registry error, fetch_artifact follows 302 and streams bytes, `file/…` URL shape, search happy path.

### 3b. Core — ProxyService metadata entry point (`crates/core/src/services/proxy/`)
**Mandatory (verified: no equivalent public path exists).** Expose `ProxyService::resolve_metadata_for(&self, req: &ProxyRequest) -> Result<PackageMetadata, CoreError>`, composing `authorize_read` (`handle.rs:164`) + `resolve_metadata_cached` (`resolve.rs:15`, `pub(super)`, requires pre-resolved client/cache_key/ttl) with the same policy/cache-key/TTL derivation `handle` uses — factor the prelude (`handle.rs:30-63`) out of `handle` rather than duplicating. Implementation note: `authorize_read` takes its own hot-lock and builds a synthetic `PackageMetadata` inline (`handle.rs:183-191`) — unify the two lock acquisitions while factoring. Unit-test: cache hit, miss→fetch→cached, upstream error + `serve_stale_metadata` → stale served.

### 4. Server wiring — `server/src/builders.rs` + `server/src/hot_config.rs`
- `builders.rs`: import; client arm `Arc::new(JetbrainsMarketplaceRegistryClient::new(url, opts)?)`; default-upstream arm `resolve_urls(&reg.upstreams, "https://plugins.jetbrains.com")`.
- `hot_config.rs::upstream_url_for` (lines 152-169): add arm returning `"https://plugins.jetbrains.com"` (feeds `UpstreamMap` for pass-through handlers) + a default test near line 534. **Blocking (verified)**: today only `Npm, Terraform, Pypi, Conda, Nuget, Composer` are listed and everything else hits `_ => None` — without this arm every cached-forward handler 404s as upstream-absent.
- `server/src/main.rs`: no change. Workspace `cargo check` clean after this step.

### 5. Web handlers — `crates/web/src/handlers/proxy/jetbrains_marketplace/` (new dir)
Files: `mod.rs` (type guard + a local base-url helper built from `req.connection_info()` → `format!("{scheme}://{host}")` — inline pattern at `pypi/simple.rs:90-93`; there is **no** shared `request_base_url` function; also define a local `content_type_for` — it is per-ecosystem, e.g. `nuget/nuspec.rs:7`, not in `common.rs`), `ide.rs` (search/compatible/aggregation/plugins JSON), `xml.rs` (`updatePlugins.xml` + `/plugins/list` via `quick_xml::Writer`, pattern `maven/routing.rs::build_metadata_xml`), `files.rs` (meta.json + `/files/{p}/{u}/{fileName}` artifact + static json blobs), `publish.rs` (multipart upload, pattern `pypi/publish.rs`), `plugin_archive.rs` (zip/jar descriptor extraction, patterns `nuspec.rs::extract_nuspec_from_nupkg:138` / `goproxy/read.rs:67-89`; **bound the decompressed size** of entries read — zip-bomb guard — since `collect_payload`'s 500 MiB cap (`common.rs:63`) is large for in-memory extraction), `cached_forward.rs` (upstream GET/POST forward helpers built on `web::Data<UpstreamMap>` + `web::Data<reqwest::Client>` — note the "npm-audit pattern" (`npm/read.rs:261`) is npm-specific with a hardcoded path and no caching, so this helper is **new code**, not an extension — with the CacheStore body cache + stale fallback per the offline-resilience design; include a key-normalization unit test: same request ± volatile param → same `fwd:` key), `render.rs` (shape per-plugin responses — plugin JSON, updates array, meta.json, plugin-list XML — from `PackageMetadata.extra`). Declare in `handlers/proxy/mod.rs`.

Routes (all `tag = "proxy/jetbrains-marketplace"`, all through `require_registry_type(&registry, "jetbrains-marketplace", &map)`; reuse `serve_local_or_proxy_artifact`, `proxy_stream`, `require_local_mode`, `collect_payload`, `publish_and_respond` from `common.rs`):

| Route | local/hybrid | proxy |
|---|---|---|
| `GET .../updatePlugins.xml` | local plugins → XML (filter by `build` if given) | 404 |
| `GET .../plugins/list?pluginId=` | eco → XML; hybrid miss → cached-metadata render | cached-metadata render (XML from `extra`) |
| `GET .../api/search/plugins`, `GET .../api/searchPlugins` | local search (+hybrid merge with cached forward) | cached forward |
| `POST .../api/search/updates/compatible` | eco (+hybrid merge) | cached forward (key: build + xmlIds hash) |
| `GET .../api/plugins/{id}`, `.../{id}/updates` | id = xmlId, from eco; hybrid miss → cached-metadata render | cached-metadata render (JSON from `extra`) |
| `GET .../files/pluginsXMLIds.json` | local ids (hybrid union with cached forward) | cached forward |
| `GET .../files/{jbPluginsXMLIds,brokenPlugins,IDE/extensions}.json` | `[]` (hybrid: cached forward) | cached forward |
| `GET .../files/{p}/meta.json`, `.../files/{p}/{u}/meta.json` | computed from index_metadata; hybrid miss → cached-metadata render | cached-metadata render |
| `GET .../files/{p}/{u}/{fileName}` | `serve_local_or_proxy_artifact` (artifact `"file/{fileName}"`) | ProxyService (cached) |
| `GET .../plugin/download?pluginId&version[&channel]` | local `get_artifact`; hybrid miss → `proxy_stream` artifact `"plugin[@channel]"` | `proxy_stream` (cached) |
| `GET .../pluginManager?action=download&id&build…` | eco latest-compatible → local artifact; hybrid miss → cached-metadata resolve + `proxy_stream` | cached-metadata resolve (`build_in_range` on `extra`) → `proxy_stream` (cached) |
| `GET .../api/search/aggregation/{field}`, `.../feature/getImplementations`, `.../api/products/intellij/plugins/{id}/comments` | empty JSON | cached forward |
| `POST .../api/updates/upload` | multipart publish, `require_local_mode` | 404 |

"Cached-metadata render" = ProxyService metadata cache (`resolve_metadata_cached`, TTL + stale-on-error) + response rendered from `PackageMetadata.extra`. "Cached forward" = the new cached-forward helper (CacheStore-backed body cache, stale fallback). Both survive upstream loss for anything previously requested.

Edge validation on every coordinate-building handler (`validate_package_name` / `validate_path_safe`) for clean 400s.

### 5-bis. CacheStore wiring — RESOLVED without new plumbing
Implementation note (simplification found during execution): `ProxyService.cache` is a **public** field (`pub cache: Arc<dyn CacheStore>`, `proxy/mod.rs:45`), and handlers already receive `web::Data<Arc<ProxyService>>` — so `cached_forward.rs` reaches the store through `svc.cache` and no `configure_app`/server change was needed. Verification: `rg CacheStore crates/web/src` matches only comments in `cached_forward.rs`.

### 6. Route registration — `crates/web/src/lib.rs`
Add openapi tag `proxy/jetbrains-marketplace` (note: the `tags(...)` block is curated, not exhaustive — e.g. `proxy/jetbrains` is referenced by its handler but absent from the list, and nothing breaks; add it anyway for consistency), imports, `paths(...)` entries, and `cfg.service(...)` calls in the literal-prefix section, most-specific-first: `api/updates/upload` → `api/search/updates/compatible` → `api/search/aggregation/{field}` → `api/search/plugins` → `api/searchPlugins` → comments → `api/plugins/{id}/updates` → `api/plugins/{id}` → `plugins/list` → `plugin/download` → `pluginManager` → `updatePlugins.xml` → `feature/getImplementations` → literal `files/*.json` → `files/{p}/{u}/meta.json` → `files/{p}/meta.json` → `files/{p}/{u}/{fileName}`. Extend the route-order comment (~line 448). No collisions with existing literal prefixes (checked: forgejo `api/packages`, gitlab `api/v4`, composer `api/upload`, rubygems `api/v1`, nuget `api/v2`).

### 7. Tests
- `crates/web/tests/common/mod.rs`: `FixedRegistry::new("jetbrains-marketplace")` under key `"jbm"` in `make_app_ext` + `("jbm", "jetbrains-marketplace")` type mapping.
- New `crates/web/tests/local_jetbrains_marketplace_registry.rs` with `make_local_jbm_app(mode)` via `local_registry_app_parts("local-jbm", "jetbrains-marketplace", mode, None)` and jar/zip fixture builders (`zip` crate writing `META-INF/plugin.xml`). Cases: publish→201; updatePlugins.xml content + self-referencing url; artifact roundtrip via `/files/{xmlId}/{version}/{file}`; search + pluginsXMLIds listing; compatible-updates in/out of build range + `updatePlugins.xml?build=` filtering; both meta.json shapes; `isHidden` excluded-but-downloadable; **`jetbrains_marketplace_publish_traversal_version_returns_400`** (descriptor version `../../etc/x`); xmlId mismatch→400, missing plugin.xml→422; mode guards (proxy-mode publish/updatePlugins → 404, wrong type → 404); hybrid fallthrough streams `FixedRegistry` body; nested-zip publish.
- **Offline-resilience tests** (the "JetBrains disappears" scenario): **`FixedRegistry` cannot fail** (verified: `resolve_metadata`/`fetch_artifact` always return `Ok`, `common/mod.rs:55-94`) — reuse the existing `UnavailableRegistry` + `make_unavailable_npm_app(repo, cache, serve_stale)` pattern (`crates/web/tests/proxy_openvsx_vscode_goproxy.rs:231`, policy at l.263; model tests `upstream_down_with_stale_metadata_returns_200` / `upstream_down_no_stale_returns_502`) adapted to jetbrains-marketplace, with an explicit policy `serve_stale_metadata: true` (the test-harness default is `false`, `common/mod.rs:185`): (a) per-plugin metadata endpoint succeeds, upstream dies, same endpoint still serves the cached/stale answer; (b) cached-forward blob (`brokenPlugins.json`) same pattern; (c) `plugin/download` of a previously fetched artifact still streams from storage with upstream dead. Adapter mockito tests already cover error mapping; the stale path itself is exercised at the ProxyService level (step 3b tests).
- `crates/examples/tests/smoke.rs`: registry config entry + assertions on `/updatePlugins.xml` and `/api/search/plugins`; optional network-gated `real_proxy.rs` download case.

### 8. UI / CLI / docs / spec
- `ui/src/config/registryTypes.ts`: `RegistryTypeDef` `id: "jetbrains-marketplace"`, `fileHint: "plugins.jetbrains.com"`; snippets: `idea.plugins.host` custom property, Manage Plugin Repositories URL (`.../updatePlugins.xml`), curl download, curl multipart publish, TOML config with the three modes.
- `cli/src/api/suggest.rs`: host row `("plugins.jetbrains.com", "jetbrains-marketplace", "jbm")` (~line 120).
- `ROADMAP.md` entry; `docs/publishing.md` (curl + Gradle/plugin-repository-rest-client host override); `docs/configuration.md` + `config.example.toml` example; README registry table.
- Resync spec/client: `task dump-spec` then `task ui:generate`.

### 9. Website (docs site, `website/` — separate pnpm/VitePress project, `task website:*`)
- `website/.vitepress/components/ConfigGenerator.vue`: add `"jetbrains-marketplace"` to the `RegistryType` union (~line 10-25); add `"jetbrains-marketplace": "https://plugins.jetbrains.com"` to the default-upstream map (~line 371); the array at l.969-974 is `PROXY_ONLY_TYPES` (**verified**: front-end mirror of `supports_local_mode()==false`, contains `github/forgejo/gitlab/jetbrains`; it locks the mode selector to `proxy`) — `jetbrains-marketplace` supports all modes so it must NOT be added there; optional one-line pre-existing fix: `generic` is missing from that Set even though the backend excludes it from local mode (`registry_kind.rs:98`); add `<option value="jetbrains-marketplace">JetBrains Marketplace</option>` to the registry `<select>` (~line 1630).
- `website/guide/user.md`: new section `## JetBrains Marketplace {#jetbrains-marketplace}` next to the existing "JetBrains IDE archives" section (~line 890) — covering: IDE setup via `idea.plugins.host` (full replacement) and Manage Plugin Repositories / `updatePlugins.xml` (additive), curl download via `/plugin/download?pluginId=&version=`, multipart publish to `/api/updates/upload` with Bearer token, the three modes, and a cross-reference distinguishing it from the `jetbrains` IDE-archive kind (whose note at ~line 921 currently suggests pointing `jetbrains` at `plugins.jetbrains.com` — update that note to point readers at the new kind instead).
- `website/guide/roadmap.md`: add JetBrains Marketplace to the supported-registries sentence (line 13) and a `✅ Shipped` table row (`jetbrains-marketplace`: full marketplace emulation, local/hybrid/proxy, marketplace-compatible publish).
- `website/index.md`: bump the registry-type count at line 63 ("eighteen" → nineteen) and keep the proxy-only exception wording accurate (the exception still applies to `jetbrains` IDE archives only, not the new marketplace kind).

### 10. Keep plan in sync
This file (`/projects/proxy-cache/.prompt/jetbrains-marketplace-plan.md`) is the implementation document — keep it updated as amendments land. (Amended 2026-07-28 after code-verification: firm step 3b, new step 5-bis, `serve_stale` naming, `fwd:` key prefix, `unlisted` for isHidden, `UnavailableRegistry` test pattern, `PROXY_ONLY_TYPES` resolved.)

**Status 2026-07-28: IMPLEMENTED.** All steps landed. Notable deviations from this plan, discovered during execution: (a) step 5-bis needed no new wiring (`ProxyService.cache` is public — see above); (b) `PublishRequest` gained an `unlisted: bool` field (it previously hard-coded `unlisted: false`) so the isHidden→unlisted mapping is atomic at publish time — all existing publish call sites updated with `unlisted: false`; (c) a `FlakyJbmRegistry` (AtomicBool switch) implements the offline tests, alongside a mockito upstream for the cached-forward blob test; (d) quick-xml 0.41 emits entities as separate `GeneralRef` events — both XML parsers (adapter + plugin.xml) accumulate `Text`/`CData`/`GeneralRef` and assign on `End`. Verification run: fmt/clippy clean, core 389 + adapters 587 + web 23 (new integration) + workspace suites green; `task coverage-check` not runnable in the implementation sandbox (no Podman) — rely on CI.

## Verification
```
cargo check --workspace
cargo clippy --workspace -- -D warnings
cargo fmt --all --check
cargo test -p batlehub-adapters --lib jetbrains_marketplace
cargo test -p batlehub-core --lib jetbrains
cargo test -p batlehub-web jetbrains_marketplace
cargo test --workspace
task coverage-check          # 80% gate — new adapter is NOT in COVERAGE_EXCLUDE
task dump-spec && task ui:generate
```

## Open risks
1. String external ids in `/files/` paths for local plugins — if an IDE path parses them as ints, fallback is a derived stable integer (v2).
2. Upload 201 response shape vs plugin-repository-rest-client expectations — flag for manual QA with a real `publishPlugin` run.
3. Build-range matching must approximate IntelliJ's `BuildNumber` semantics (`241.*`, open until, product prefix) — mitigated by `build_in_range` test matrix.
4. Nested-zip descriptor extraction buffers in memory — the `collect_payload` cap is 500 MiB (`common.rs:63`), large for in-memory work; additionally bound the **decompressed** size of archive entries read (zip-bomb guard), following the bounded reads in `goproxy/read.rs:67-89` / `nuspec.rs:138`.
5. Cached-forward key normalization (risk stays active — full caching scope confirmed): volatile query params (`uuid`, machine ids) must be stripped from cache keys or hit rates collapse and the cache bloats; keep an explicit documented drop-list, use the dedicated `fwd:` key prefix, and cover with a unit test (same request ± volatile param → same key). CacheStore entries wrap raw bodies in a synthetic `PackageMetadata` — slightly off-label use of the metadata cache; verify Redis/Postgres cache adapters tolerate large `extra` payloads (search responses can be tens of KB).
6. Offline coverage is "what was seen before": a plugin never requested while upstream was alive cannot be served after it disappears. For stronger guarantees, cache warming (`crates/core/src/services/warming/`) could pre-pull chosen xmlIds — note as follow-up, not v1.
