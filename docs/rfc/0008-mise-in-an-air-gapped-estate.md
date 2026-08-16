# RFC 0008 — mise in an air-gapped estate

| Field       | Value                                                        |
| ----------- | ------------------------------------------------------------ |
| Status      | Draft                                                         |
| Author      | Max Batleforc <maxleriche.60@gmail.com>                       |
| Co-author   | —                                                             |
| Created     | 2026-08-15                                                    |
| Supersedes  | —                                                             |
| Complements | RFC 0004-bis §13.2 (content-addressable dedup), RFC 0002 (what BatleHub knows about a CVE) |
| Touches     | `crates/config`, `crates/core`, `crates/adapters`, `crates/web`, `server`, `cli`, `ui`, docs |

---

## 1. Summary

BatleHub already tells people how to point [mise](https://mise.jdx.dev) at it. The Setup Guide has a
`mise` tab, `batlehub-cli registry suggest --mise` reads a project's `mise.lock` and prints a
`[settings.url_replacements]` block, and `docs/registries/generic.md` documents the toolchain
mirrors that block refers to. All of that works, and none of it is an air gap. It is a *restricted
network* story: it assumes the workstation can still reach whatever the rewrite table failed to
mention.

This RFC makes `mise install` work on a host with **no route off the site at all**. It does three
things. It turns `mise.lock` — which already records the exact URL and checksum of every tool, per
platform — into a **plan** that BatleHub can be seeded from and audited against, instead of into
advice. It adds an **`[air_gap]` mode** in which a proxy-mode registry never dials upstream: a miss
fails fast, names itself, and is recorded, so the list of things the next bundle needs is produced
by the estate rather than guessed by an operator. And it moves mise's supply-chain verification —
cosign, SLSA, GitHub attestations, all on by default and all unreachable offline — to the connected
side of the gap, performed once at seed time and recorded as a verdict BatleHub serves, rather than
switched off on every workstation and forgotten.

### Before / after

```text
# today, on a disconnected workstation

$ mise install
mise aqua:EmbarkStudios/cargo-deny@latest ⠋
  … hangs on api.github.com, then fails with a connect timeout.
# The url_replacements block covered api.github.com. It did not cover
# fulcio.sigstore.dev, which aqua.cosign = true reaches before installing.
# Nothing says so. The operator sets MISE_PARANOID=0 and moves on.

# with this RFC, connected side (once):
$ batlehub-cli mise plan --lock mise.lock --platform linux-x64 -o mise-plan.json
28 tools · 41 downloads · 6 registries · 3 hosts with no mirror configured
$ batlehub-cli mise seed --plan mise-plan.json --verify
41/41 fetched · 41/41 checksums match the lock · 38/41 cosign-verified, 3 unsigned
$ batlehub-cli admin bundle export --plan mise-plan.json -o estate.bhub

# disconnected side:
$ batlehub-cli admin bundle import estate.bhub
signature ok (ed25519 f3a9…) · 41 blobs · 0 rejected
$ mise install
mise all tools installed          # no egress, checksums verified from mise.lock

# and when the plan was wrong:
$ mise install some-new-tool
mise ERROR download failed: 503 from batlehub.corp
      not in this instance: github/jdx/mise-tool@v1.2.0
$ batlehub-cli admin air-gap missing
github  jdx/mise-tool@v1.2.0   4 requests   first 2026-08-14  last 2026-08-15
```

---

## 2. Motivation

1. **The rewrite table is hand-assembled, and its failure mode is a hang.**
   `ui/src/config/registryTypes.ts`'s `mise` entry rewrites eight patterns: the GitHub API,
   release-asset downloads, `github.com` archives, `codeload.github.com`, `raw.githubusercontent.com`,
   `registry.npmjs.org` and `static.crates.io`. `mise settings --all` on 2026.8.0 lists 160 settings,
   of which at least nine name a *different* host in their default value —
   `go.download_mirror` (`dl.google.com/go`), `go.repo` (`github.com/golang/go`),
   `dotnet.registry_url` (`api.nuget.org`), `pipx.registry_url` (`pypi.org/pypi/{}/json`),
   `python.pyenv_repo`, `ruby.ruby_build_repo`, `ruby.ruby_install_repo`, `ruby.precompiled_url`,
   and the two `github.oauth_*_url`s. A URL the table does not match is not an error; it is a direct
   request, and on a disconnected network that is a connect timeout with no attribution. There is no
   way to ask BatleHub whether a table is complete, and no way to make the gap fail loudly.

2. **mise's verification is on by default and has no proxy path.**
   `github_attestations = true`, `github.slsa = true`, `aqua.cosign = true`, `aqua.slsa = true`,
   `aqua.minisign = true`. Those reach Sigstore's Fulcio and Rekor, and GitHub's attestation API.
   BatleHub proxies neither, and a transparency log is not a thing you cache — an offline Rekor
   answer proves nothing about inclusion. So the operator disables verification on every workstation,
   in the one environment whose entire justification is that it verifies things. The verification is
   *possible*, just not there: the connected side can do it once.

3. **Several backends fetch over git, which BatleHub does not speak.**
   asdf plugins, `python.pyenv_repo`, `ruby.ruby_build_repo` and `ruby.ruby_install_repo` are git
   clones. `crates/web` has no smart-HTTP surface — no `info/refs`, no `git-upload-pack` route
   exists. `url_replacements` cannot help: it rewrites the URL, and the rewritten URL is still a git
   endpoint. Today an estate discovers this per-tool, at install time, at the point of use.

4. **Nothing puts content into a disconnected instance.**
   Proxy mode needs an upstream by construction, and `[registries.cache] warm_paths` /
   `warm_packages` warm *from* that upstream — they are a connected-network feature. `ROADMAP.md`
   already records the gap for the estate as a whole ("Instance-to-instance transfer for air-gapped
   estates", still unchecked) and correctly says the only path in today is restoring a full backup,
   which moves the database, the config, and every credential in it. mise is what makes the problem
   tractable rather than open-ended: a mise estate's content is exactly the tools in `mise.lock` —
   finite, enumerable, version-pinned, and checksummed by a file the project already commits.

5. **`mise.lock` is a bill of materials and nothing consumes it as one.**
   `cli/src/api/suggest.rs` reads it and is explicit that it is "the best source there is: it records
   the *exact* URL of every tool the project installs, per platform". It then throws the URLs away
   and emits `[[registries]]` blocks. The same parse, kept, is the manifest of everything the mirror
   must hold — and the checksum against which a seeded copy can be proven byte-identical to what the
   connected side saw.

6. **A miss on a disconnected instance is indistinguishable from a bug.**
   With `serve_stale = true` (the default) and an unreachable upstream, a metadata request degrades
   to stale-or-error and an artifact request to an error whose text is about a connection. The
   operator cannot tell "this was never in the bundle" from "the network is broken", and no record
   accumulates that would make the next bundle better. The feedback loop that would converge an
   air-gapped mirror on completeness does not exist.

---

## 3. Goals / non-goals

**Goals**

- `mise install` completes on a host with no egress, against a BatleHub that was seeded before the
  gap, using only BatleHub.
- The set of content to mirror is *derived* from `mise.lock` and its platforms, not hand-listed.
- A URL the estate did not plan for fails immediately, locally, naming the host and the coordinate —
  and is recorded, so the next bundle can be complete by construction rather than by iteration.
- Supply-chain verification survives the gap: performed on the connected side where Sigstore and the
  attestation API are reachable, recorded per artifact, and visible on the disconnected instance.
- An operator can answer "is this bundle complete for this lock?" **before** it is carried across,
  and "what did the estate ask for that we did not have?" after.
- Nothing above changes behaviour for a connected instance that does not opt in.

**Non-goals**

- **Proxying Sigstore.** Fulcio and Rekor are not caches; a stale inclusion proof is not a proof. The
  design moves the verification, it does not relay the service.
- **Speaking git.** Three backends (asdf plugins, `pyenv`, `ruby-build`/`ruby-install`) fetch over
  git. Adding a smart-HTTP surface is a large new protocol area for backends that all have an
  HTTP-fetching alternative (`aqua:`, `ubi:`, `core:`). The RFC's answer is to detect them in `mise
  plan` and name them as unsupported, loudly, at planning time rather than at install time.
- **Mirroring GitHub.** The unit is "the artifacts this lock names", not "the hosts they came from".
- **Making mise itself offline-aware.** mise has `offline` and `prefer_offline` settings already;
  this RFC does not propose changes to mise, only configuration of it.
- **A general-purpose instance-to-instance replication protocol.** The ROADMAP item is broader than
  mise. This RFC defines a bundle format sufficient for a planned artifact set and constrains what
  the general case must stay compatible with (§9); it does not attempt live or incremental
  replication.
- **Solving the bootstrap of `mise` itself.** The first mise binary on a disconnected host arrives in
  the base image, not through a mirror that mise is required to reach. §11 keeps this open only for
  the *upgrade* path.

---

## 4. User-facing design

### 4.1 `[air_gap]` — a server that will not dial out

```toml
[air_gap]
enabled              = true    # default false; false is exactly today's behaviour
bundle_trusted_keys  = ["3b1f…"]  # hex ed25519 public keys accepted on import
record_misses        = true    # default true when enabled
miss_retention_days  = 90      # default 90; 0 keeps them until purged by hand
```

When `enabled = true`:

- **No proxy-mode registry attempts an upstream connection.** A cache hit is served exactly as
  today. A miss returns `503` with a JSON body naming the registry and coordinate, rather than a
  connect error some seconds later.
- **`serve_stale` becomes the normal path, not the degraded one.** Cached metadata is served without
  a revalidation attempt; there is nothing to revalidate against. The `serve_stale = false` case
  becomes a hard miss and is reported as one.
- **Cache warming is refused at validation time**, not at run time — `warm_paths` and
  `warm_packages` describe a fetch from an upstream that this mode says will never happen (§4.3).
- **`[proxy]` (the egress proxy section) is refused in the same way.** Both being set is a
  contradiction the operator should see at boot, not discover from a log.

`enabled = false` — the default, and the value in every existing config — leaves every code path as
it is today. This is additive; `CURRENT_CONFIG_VERSION` stays at `1`.

### 4.2 The plan — `mise.lock` as a bill of materials

```console
$ batlehub-cli mise plan --lock mise.lock --platform linux-x64,darwin-arm64 -o mise-plan.json
```

Produces a plan describing every download the locked tools imply, resolved per platform:

```json
{
  "plan_version": 1,
  "generated_from": { "file": "mise.lock", "sha256": "9c1e…" },
  "platforms": ["linux-x64", "darwin-arm64"],
  "entries": [
    {
      "tool": "aqua:EmbarkStudios/cargo-deny",
      "version": "0.18.2",
      "platform": "linux-x64",
      "url": "https://github.com/EmbarkStudios/cargo-deny/releases/download/0.18.2/cargo-deny-0.18.2-x86_64-unknown-linux-musl.tar.gz",
      "registry": { "name": "github", "type": "github" },
      "key": "github/EmbarkStudios/cargo-deny/0.18.2/cargo-deny-0.18.2-x86_64-unknown-linux-musl.tar.gz",
      "sha256": "4f0c…",
      "size": 5439201
    }
  ],
  "unsupported": [
    { "tool": "asdf:mise-plugins/mise-postgres", "reason": "git-fetched backend; no HTTP path through BatleHub" }
  ],
  "unmirrored_hosts": ["binaries.sonarsource.com"]
}
```

Three fields carry the argument of this RFC:

- **`key`** is the storage key the artifact will occupy, derived by the same
  `artifact_storage_key(registry, name, version)` the proxy and local-registry paths already share.
  A plan is therefore a statement about BatleHub's storage, not about a URL.
- **`unsupported`** is the git-backend list from §3, produced at planning time. The operator learns
  that `asdf:` tools will not work *before* the bundle is built, and the message names the backend
  rather than the symptom.
- **`unmirrored_hosts`** is every host in the lock for which no registry is configured. It is the
  answer to "is my `url_replacements` table complete?", which today has no answer.

`mise plan` is offline itself: it reads the lock and the server's registry list, and resolves nothing
over the network.

### 4.3 Seeding, exporting, importing

```console
# connected side — fetch every planned entry through BatleHub, prove it matches the lock
$ batlehub-cli mise seed --plan mise-plan.json --verify

# export the planned set as a signed, content-addressed bundle
$ batlehub-cli admin bundle export --plan mise-plan.json --sign-key ./estate.key -o estate.bhub

# disconnected side
$ batlehub-cli admin bundle import estate.bhub
```

`mise seed` drives the **existing** admin warm API (`client.cache_warm(&registry, packages, paths)`)
one entry at a time, then re-reads each artifact and compares its digest to the lock's. `--verify`
additionally runs the upstream verification mise would have run — cosign, SLSA, GitHub attestation —
and records the verdict (§5.2). Its exit status is non-zero if any entry is missing or any digest
disagrees, so it is usable as a CI gate on the connected side.

A bundle is a tar of:

```text
manifest.json          # the plan, plus per-entry verification verdicts and metadata rows
blobs/<sha256>         # content-addressed, so two registries holding the same bytes ship once
manifest.sig           # ed25519 detached signature over manifest.json
```

Blobs are content-addressed rather than key-addressed, which is what makes the format compatible with
the content-addressable dedup of RFC 0004-bis §13.2: `manifest.json` maps keys onto digests, and the
digest is the identity. `import` verifies `manifest.sig` against `air_gap.bundle_trusted_keys` before
reading a single blob, then writes each blob and its metadata rows transactionally.

### 4.4 Behaviour rules

- **A miss is a `503`, not a `404`.** `404` asserts the artifact does not exist, which is false — it
  exists, it is simply not in this instance. `503` also keeps the semantics a client already
  understands as "try later", where "later" is after the next bundle. The body is JSON with
  `registry`, `coordinate` and `bundle_hint`, and the same information goes to the miss log.
- **A miss is recorded once per unique `(registry, key)`,** with a first-seen, last-seen and a
  counter. mise retries; the log must not grow with the retries.
- **The catch-all rewrite.** `mise plan --emit-mise-toml` appends a final rule mapping anything not
  already rewritten onto `{proxy}/_air-gap/unmirrored/…`, which returns `501` naming the host and
  records it alongside the misses. This turns "a host nobody predicted" from a connect timeout into a
  line in the console. Whether mise applies `url_replacements` in declaration order with first-match
  wins is the one behaviour this depends on, and it is open (§11).
- **RBAC is unchanged.** An air-gapped instance still evaluates `RbacRule` and the rest of the chain;
  offline is not a synonym for anonymous. The `releases:read` grants in the generic-mirror examples
  remain what a workstation needs.
- **Verification verdicts are read-only after import.** A disconnected instance cannot re-run cosign,
  so it serves the recorded verdict and says where it came from — bundle id, key, and the date the
  connected side verified. It never presents a recorded verdict as a live one.

### 4.5 Validation

`AppConfig::validate()` rejects:

| Condition | Rationale |
| --- | --- |
| `air_gap.enabled = true` together with any `[proxy]` or `[registries.proxy]` table | An egress proxy is a route off the site. Both set is a contradiction, and the safe reading (which one wins?) is not obvious enough to pick silently. |
| `air_gap.enabled = true` with any `cache.warm_paths` / `cache.warm_packages` entry | Warming fetches from an upstream this mode guarantees will never be dialled. Failing at boot is better than a startup task that logs a connect error per path, forever. |
| `air_gap.bundle_trusted_keys` entry that is not 32 hex-encoded bytes | Same rule as `signing.trusted_keys`; an unusable key must not read as "signing is configured". |
| `air_gap.enabled = true` with `bundle_trusted_keys = []` | Import would accept any bundle. An air-gapped instance whose only content path is unauthenticated is worse than one with no content path. |

Warnings (logged and surfaced to the admin):

| Condition | Behaviour |
| --- | --- |
| `air_gap.enabled = true` and a registry is in `local` or `hybrid` mode | Kept and allowed — publishing to a disconnected instance is legitimate. The warning exists because the *hybrid fall-through* can no longer reach upstream, so a hybrid registry behaves as local, and an operator should know that is what they configured. |
| `air_gap.enabled = false` and `bundle_trusted_keys` is non-empty | Kept. Importing a bundle into a connected instance is how the connected side stages one; the warning notes the keys are unused for serving. |
| A registry has no cached content at boot in air-gap mode | Logged once, with the count, and shown on the admin page. An empty registry in this mode answers `503` to everything, and that is worth saying at boot rather than at first request. |

---

## 5. Architecture

### 5.1 Three sides, one plan

```mermaid
flowchart LR
    subgraph connected["connected side"]
        L["mise.lock"] --> P["batlehub-cli mise plan"]
        P --> PL["mise-plan.json"]
        PL --> S["batlehub-cli mise seed --verify"]
        S --> B1["BatleHub #40;staging#41;"]
        B1 --> E["admin bundle export"]
    end
    E --> BU["estate.bhub<br/>manifest + blobs + ed25519 sig"]
    subgraph gap["the gap"]
        BU
    end
    subgraph disconnected["disconnected side"]
        BU --> I["admin bundle import"]
        I --> B2["BatleHub #40;air_gap.enabled#41;"]
        B2 --> M["mise install"]
        B2 --> MISS["missing-content log"]
    end
    MISS -.->|"next plan"| P
```

The invariant the shape protects: **the plan is the only thing that crosses in both directions.** It
goes across as a manifest inside the bundle, and comes back as a miss list that feeds the next
`mise plan`. Nothing else needs to be carried, and in particular the database, the config and its
credentials do not — which is the specific objection `ROADMAP.md` raises against the
restore-a-backup workaround.

### 5.2 Verification, moved rather than removed

```mermaid
sequenceDiagram
    participant CLI as mise seed --verify
    participant BH as BatleHub (connected)
    participant UP as upstream
    participant SIG as Sigstore / attestations
    CLI->>BH: warm(registry, key)
    BH->>UP: GET artifact
    UP-->>BH: bytes
    BH-->>CLI: cached, digest D
    CLI->>CLI: D == mise.lock sha256 ?
    CLI->>SIG: cosign / SLSA / attestation for D
    SIG-->>CLI: verdict + issuer + timestamp
    CLI->>BH: record verdict(key, D, verdict, issuer, verified_at)
    Note over BH: verdict travels in the bundle manifest
```

Two properties this preserves across the gap:

- **The checksum is verified on both sides, independently.** `mise.lock` carries the sha256, and mise
  checks it at install time with no network. The seed step checks the same digest against what
  BatleHub actually stored. A bundle that was corrupted, truncated or tampered with in transit fails
  on the disconnected side without reference to anything the bundle itself claims.
- **The signature verdict is verified once, where it can be.** It is then *evidence about a past
  check*, and the design labels it that way everywhere it is shown. That is a real reduction in
  assurance compared to live verification, and pretending otherwise would be the failure mode worth
  avoiding: the alternative in the field today is `MISE_PARANOID=0` and no record at all.

Ed25519 is the only signature BatleHub verifies in-process (`signing.trusted_keys`; the `rsa` crate
is banned by `deny.toml` for RUSTSEC-2023-0071, which rules out PGP and x509). Cosign signatures are
ECDSA over an x509 identity, so BatleHub **records the verdict, not the signature** — it cannot
re-derive one and must not imply it can. Bundle signing itself is ed25519, reusing the existing
verification code and key format.

### 5.3 Where a miss is decided

```mermaid
flowchart TD
    A["request → handler"] --> B{"cached?"}
    B -->|yes| C["serve from storage"]
    B -->|no| D{"air_gap.enabled?"}
    D -->|no| E["ProxyService::handle → upstream"]
    D -->|yes| F["503 + record miss"]
    F --> G["missing_content: first_seen, last_seen, count"]
```

The decision sits in `ProxyService::handle`, at the point where it would otherwise stream from the
upstream client — after the rule chain, not before. That ordering matters and is deliberate: a
coordinate that RBAC or a `block_list` rule denies must still be denied in air-gap mode, and must not
be recorded as "missing content the next bundle should carry". A blocked package is not a gap in the
mirror.

---

## 6. Detailed design

### 6.1 `crates/config`

- New `AirGapConfig` in `crates/config/src/schema/` (its own `air_gap.rs`, following
  `registry.rs`'s shape): `enabled`, `bundle_trusted_keys`, `record_misses`, `miss_retention_days`,
  all `#[serde(default)]`.
- `AppConfig::validate()` gains the four rejections and three warnings of §4.5. Warnings go through
  the existing warning channel (`crates/config/src/schema/warnings.rs`) so they reach the admin
  surface rather than only the log.
- `CURRENT_CONFIG_VERSION` does not move: the section is additive and absent means today's behaviour.

### 6.2 `crates/core`

- `AirGapPolicy` alongside `RegistryPolicy`, snapshotted from `HotConfig` by the same
  clone-the-`Arc`-before-any-`await` discipline the rest of the proxy path uses.
- `ProxyService::handle` — at the upstream-fetch branch, when the policy is on: return
  `CoreError::ContentUnavailable { registry, key }` instead of dialling, and hand the coordinate to
  the miss recorder. New error variant rather than reusing `NotFound`, because the web layer's
  hybrid fall-through treats `NotFound` as "ask upstream" — the exact confusion RFC 0006's
  `AccessDenied`/`NotFound` split exists to prevent, and reusing it here would reintroduce it.
- New port `MissRecorder` in `crates/core/src/ports/`, with the in-memory and Postgres implementations
  in `crates/adapters`. Recording is fire-and-forget: a failure to record a miss must never turn a
  `503` into a `500`.
- `services/bundle.rs` — the manifest model, the digest/key mapping, and `verify_manifest_signature`,
  which reuses the ed25519 verification already written for `signing.verify_on_download`.

### 6.3 `crates/adapters`

- `missing_content` table + a `mig!` entry in `crates/adapters/src/migrations.rs`, keyed
  `(registry, storage_key)` with `first_seen`, `last_seen`, `count`, `kind`
  (`artifact` | `metadata` | `unmirrored_host`). Upsert increments the counter and moves `last_seen`;
  this is the "recorded once per unique key" rule of §4.4 expressed as a primary key.
- `artifact_attestations` table: `(storage_key, digest, verdict, issuer, verified_at, bundle_id)`.
  One row per digest, not per key — two registries holding identical bytes share the verdict, which
  is the same identity rule the blob store uses.
- Bundle reader/writer: streaming tar, digest-checked on the way in. A blob whose content does not
  hash to its filename is rejected without being written.

### 6.4 `crates/web`

- `503` renderer for `CoreError::ContentUnavailable`, with the JSON body of §4.4.
- `GET /_air-gap/unmirrored/{tail:.*}` — records and returns `501`. **It never fetches anything**;
  it exists to convert an unmatched rewrite into a diagnosable event, and its handler has no HTTP
  client in scope so it cannot become an SSRF surface (§7).
- Admin endpoints, each with a `body = T` in its `utoipa::path` per the OpenAPI contract test:
  `GET /api/v1/admin/air-gap/missing`, `DELETE /api/v1/admin/air-gap/missing`,
  `POST /api/v1/admin/bundle/import`, `GET /api/v1/admin/bundle`.
- Import applies `validate_coordinate` and `ensure_safe_key` to **every** manifest key before writing,
  and re-applies each target registry's `path_allow`. A bundle is untrusted input that names storage
  keys; without this it would be a way to plant content at an arbitrary key, which is precisely what
  the two existing funnels exist to stop.

### 6.5 `cli`

- `batlehub-cli mise plan` — extends `cli/src/api/suggest.rs` rather than duplicating it. The lock
  parser (`collect_from_mise_lock`) already extracts per-tool URLs; the plan keeps them and adds the
  registry/key resolution, the `unsupported` classification from `BACKEND_REGISTRIES` (a backend not
  in that table and not HTTP-fetching lands in `unsupported`), and the `unmirrored_hosts` diff
  against `GET /api/v1/registries`.
- `batlehub-cli mise seed [--verify]`, `admin bundle export|import`, `admin air-gap missing`.
- `registry suggest --mise` gains the catch-all rule in its emitted block, behind
  `--catch-all` so an existing user's output does not change shape without asking.

### 6.6 `ui`

- One new admin page, **Air gap**: bundle history (id, signer, imported-at, blob count, rejections),
  the missing-content table (sortable by count, exportable as the input to the next plan), and per
  artifact the recorded verdict with its `verified_at` and bundle id — labelled as a past check.
- `ui/src/config/registryTypes.ts`'s `mise` entry gains the catch-all rule and a note explaining what
  the `501` means, so the Setup Guide and the CLI emit the same block.

### 6.7 docs

- `docs/use/mise.md` — the one home for "point mise at BatleHub", connected and air-gapped. It is
  `use/` and not `registries/`: mise is a client, not a registry protocol. `docs/registries/generic.md`
  and `docs/registries/github.md` keep their one-line pointers and lose nothing else, per the
  one-instruction-one-home rule of RFC 0005-bis.
- `docs/guide/configuration.md` gains the `[air_gap]` section; `docs/operations/` gains the
  bundle runbook (build, carry, import, reconcile misses).

**Deliberately untouched**, so reviewers do not go looking:

- `crates/adapters/src/registry/http_client.rs` — air-gap mode is enforced above the HTTP client, in
  the service. Making the client itself refuse to dial would also break the connected side's seed
  path, which runs through the same code.
- The `[proxy]` egress section — unchanged and still the right answer for a *restricted* network.
  §4.5 only refuses the combination.
- `RegistryKind` — no new variant. mise is a client that speaks GitHub, npm, cargo and plain HTTP,
  all of which already have homes; a `mise` kind would be a protocol that does not exist.

---

## 7. Security considerations

- **The gap does not make content trusted; it makes it unverifiable later.** The design's answer is
  to verify on the connected side and carry the verdict, and to label it as a past check everywhere
  it is displayed. A recorded verdict shown as though it were live would be a claim about the product
  that is not true.
- **Bundles are attacker-relevant input.** They arrive on removable media, by definition from outside
  the instance's trust boundary. Import verifies an ed25519 signature against configured keys
  *before* reading blobs, rejects a bundle with an empty trusted-key list at config validation, and
  digest-checks every blob against its own name. A blob that does not hash to its filename is never
  written.
- **A manifest names storage keys, so it is a path-traversal surface.** `validate_coordinate`,
  `ensure_safe_key` and each registry's `path_allow` all apply to import, exactly as they do to a
  publish. This is the third funnel through the same guards, and deliberately so.
- **`/_air-gap/unmirrored/` must not become SSRF.** It is a recorder and a `501`. Its handler holds no
  HTTP client, and the URL tail is treated as an opaque string for logging — never parsed into a
  request. A test asserts no outbound request results from calling it.
- **Miss records are attacker-writable in one narrow sense.** Any client that can reach a registry can
  cause rows to appear. They are bounded by the `(registry, storage_key)` primary key, subject to
  `miss_retention_days`, and recorded only *after* the rule chain has allowed the request — so a
  denied coordinate never enters the log, and the log cannot be used to enumerate what the instance
  blocks.
- **Air-gap mode does not relax authorization.** RBAC, the block list and the gates run unchanged.
  The mode changes where bytes come from, not who may have them.
- **What an attacker gains from the new surface, if a check is bypassed:** for `/_air-gap/unmirrored/`,
  nothing — it returns a fixed status and writes a bounded row. For import, everything, which is why
  the signature check precedes blob reading rather than accompanying it.

---

## 8. Alternatives considered

| Alternative | Why rejected |
| --- | --- |
| A pull-through proxy in a DMZ, reachable from the enclave | That is a route off the site. It is the `[proxy]` feature that already exists, and it answers a different question — a restricted network, which BatleHub already serves. |
| Ship `~/.local/share/mise` as a tarball per workstation | No provenance, no RBAC, no CVE view, no shared cache; drifts per machine, and does not survive `mise upgrade` or a second project with different pins. It also moves binaries with no record of what verified them. |
| Teach BatleHub git smart-HTTP so asdf/pyenv/ruby-build work | A large new protocol surface, with its own auth and traversal characteristics, for three backends that each have an HTTP-fetching alternative. `mise plan` naming them as unsupported costs one list and no attack surface. |
| A new `mise` `RegistryKind` | mise is not a protocol. The content is GitHub releases, npm tarballs, crates and plain files, all of which have adapters. A `mise` kind would have to delegate to all of them and would own nothing. |
| Vendor the aqua registry into BatleHub so aqua tools resolve offline | Unnecessary: `aqua.baked_registry = true` is mise's default and the registry is compiled into the mise binary. There is nothing to mirror. |
| Serve `404` on a miss instead of `503` | `404` asserts non-existence, which is false and which the hybrid fall-through and the local-registry read path both treat as "ask upstream" — the confusion RFC 0006 spent an RFC separating. |
| Record misses client-side (a mise plugin or wrapper) | Puts the feedback loop on the workstation, where it is per-user, unaggregated, and lost on reimage. The instance is the only place that sees the whole estate. |
| Make the bundle a full database backup | The ROADMAP's own objection: it moves the config and every credential in it, and it cannot express "these approved artifacts" as a unit. |

---

## 9. Rollout and compatibility

- **Default behaviour.** `[air_gap]` absent ⇒ `enabled = false` ⇒ every path behaves exactly as
  today. The CLI subcommands are additive; `registry suggest`'s output only changes under an explicit
  `--catch-all`.
- **Config migration.** None. `CURRENT_CONFIG_VERSION` stays `1`.
- **Operator prerequisites.** An ed25519 keypair for bundle signing, held on the connected side, with
  the public half in the disconnected instance's `air_gap.bundle_trusted_keys`. Storage sized for the
  planned set — toolchain tarballs are large, and `limits.max_artifact_size_bytes` (default 500 MiB)
  usually needs raising, as `docs/registries/generic.md` already warns.
- **Rollback.** Setting `enabled = false` restores upstream fetching immediately; nothing about the
  mode is persisted. Imported content is ordinary cached content and survives — an instance that
  regains a network keeps everything the bundle gave it and starts filling misses from upstream.
- **Compatibility with the ROADMAP item.** This RFC implements the mise-shaped slice of
  "instance-to-instance transfer" and constrains the general case: blobs are content-addressed by
  sha256 (so RFC 0004-bis §13.2's dedup is the same identity), the manifest maps keys onto digests,
  and the signature covers the manifest rather than the tar. A future general bundle should extend
  `manifest.json` with more row types, not replace the container.

---

## 10. Test plan

- **Unit** (`crates/config/src/schema/tests.rs`): each §4.5 rejection and warning, including
  `air_gap` + `[proxy]`, `air_gap` + `warm_paths`, and an empty trusted-key list.
- **Unit** (`crates/core/src/services/proxy/`): `handle` in air-gap mode returns
  `ContentUnavailable` and does not touch the registry client (a `FixedRegistry` double that panics
  on call proves it); a coordinate denied by the rule chain is denied *and not recorded*.
- **Unit** (`crates/core/src/services/bundle.rs`): manifest signature accept/reject, blob digest
  mismatch, and a manifest whose key fails `validate_coordinate`.
- **Integration** (`crates/web/tests/air_gap.rs`, new file per the one-file-per-area convention):
  cached hit still serves; miss returns `503` with the documented body; the miss appears once for
  four requests with `count = 4`; `/_air-gap/unmirrored/` returns `501` and records; import rejects
  an unsigned bundle, a bundle signed by an untrusted key, a blob with a bad digest, and a manifest
  key containing `..`; a hybrid registry in air-gap mode does not fall through.
- **Integration** (`cli/tests/integration.rs`): `mise plan` against a fixture `mise.lock` produces
  the expected entries, classifies an `asdf:` tool as unsupported, and lists a host with no registry
  under `unmirrored_hosts`; `bundle export` → `import` round-trips against the in-process server.
- **External** (`crates/adapters/tests/pg_air_gap.rs`, via `task test:pg-*`): the `missing_content`
  upsert counter and `miss_retention_days` purge against real Postgres.
- **Layer 4** (`crates/examples/tests/smoke.rs`): the existing suite already runs `mise install`
  against real upstreams. Add the air-gapped counterpart — seed, export, import into a second
  instance configured with `air_gap.enabled = true` and no network, then `mise install` — as the
  end-to-end signal that the claim in §1 is true.
- **Existing suites that must pass unchanged**: `crates/web/tests/blocked_versions_hidden.rs` and the
  local-registry suites (the `NotFound`/`AccessDenied` distinction must not shift under a third error
  variant), `crates/web/tests/openapi_contract.rs` (the new endpoints all declare bodies), and the
  full `cargo test --workspace` with `[air_gap]` absent, which is the regression signal that the
  default path is untouched.

---

## 11. Decisions and open questions

### Resolved

| # | Question | Decision |
| --- | --- | --- |
| 1 | New `RegistryKind` for mise? | **No.** mise is a client, not a protocol; its content already has adapters. |
| 2 | Relay Sigstore through BatleHub? | **No.** A transparency log is not a cache; verify on the connected side and carry the verdict. |
| 3 | `404` or `503` on an air-gapped miss? | **`503`.** `404` asserts non-existence and is what the hybrid fall-through acts on. |
| 4 | Blob identity in the bundle | **sha256 content address**, so dedup (RFC 0004-bis §13.2) is the same identity and duplicate bytes ship once. |
| 5 | Bundle signature algorithm | **ed25519.** The `rsa` ban (RUSTSEC-2023-0071, enforced by `deny.toml`) rules out PGP/x509, and the verification code already exists for `signing.trusted_keys`. |
| 6 | Store the cosign signature or the verdict? | **The verdict.** BatleHub cannot verify ECDSA/x509 in-process, so storing the signature would imply a capability it does not have. |
| 7 | Where the miss check sits relative to the rule chain | **After.** A blocked coordinate is not missing content, and must not be proposed for the next bundle. |

### Still open

1. **Does mise apply `[settings.url_replacements]` in declaration order, first match wins?** The
   catch-all rule of §4.4 depends on it. If matching is unordered or last-match-wins, the catch-all
   would swallow the specific rules and the mechanism needs another shape — most likely an explicit
   deny-list of known hosts rather than a wildcard. This needs empirical confirmation against a
   pinned mise version before the phase that ships it, and the answer should be recorded here with
   the version it was checked on.
2. **Per-platform plans.** `mise.lock` records URLs per platform. Does a bundle carry every platform
   in the lock, or only the ones the estate runs? Carrying all is simpler and larger; the recommended
   default is `--platform` explicit with no implicit "all", so the size is a choice the operator makes
   knowingly.
3. **How the mise binary itself is upgraded across the gap.** The first install comes from the base
   image (§3, non-goals). `mise self-update` reaches GitHub, which the `github` registry can serve —
   but the version that performs the upgrade is the one whose `url_replacements` we control, so this
   is probably fine and probably needs a documented procedure rather than a feature. Confirm.
4. **Miss-log granularity for metadata.** An artifact miss is one key. A metadata miss for a listing
   is a package, and a `503` there fails a resolve rather than a download. Should the two be separate
   `kind`s in the console (as designed) or separate tables? Current answer: one table, one `kind`
   column; revisit if the console wants different columns for each.
5. **Whether `mise seed --verify` should fail the run on an unsigned artifact.** Many GitHub releases
   carry no attestation at all. Failing would make the feature unusable; not failing risks the verdict
   column reading as "fine". Proposed: exit non-zero only on a *failed* verification, report
   `unsigned` as its own count in the summary, and make `--require-signed` the strict opt-in.

---

## 12. Implementation phases

| Phase | Content |
| --- | --- |
| 1 | `[air_gap]` config, validation, `ContentUnavailable` → `503`, miss recording, admin list/purge endpoints. **Useful alone**: it turns a disconnected instance from "hangs mysteriously" into "says what it lacks", with no bundle format at all. |
| 2 | `batlehub-cli mise plan` — plan file, `unsupported` and `unmirrored_hosts` classification, `registry suggest --catch-all`, `/_air-gap/unmirrored/`. **Useful alone**: answers "is my rewrite table complete?" for connected estates too. |
| 3 | `mise seed [--verify]` and the attestation store — connected side only, no bundle yet. |
| 4 | Bundle format, export, import, signature verification, and the traversal/`path_allow` guards on import. |
| 5 | The Air gap admin page and `docs/use/mise.md` + the operations runbook. |
| 6 | Layer-4 smoke test: seed → export → import → `mise install` with no network, as the standing proof of §1. |
