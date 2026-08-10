# TODO — security remediation + 1.1.0 release

Source: deep security review of `main` @ `c431f7b` (2026-08-10).
Working branch: `security/release-1.1.0`

**Legend:** `[ ]` todo · `[~]` in progress · `[x]` done · `[!]` blocked / needs decision

---

## Baseline (verified before starting)

| Gate | Result |
|---|---|
| `cargo audit` (647 deps) | clean |
| `cargo deny check` | advisories / bans / licenses / sources ok |
| `cargo clippy --workspace --all-targets -- -D warnings` | clean |
| `cargo fmt --all --check` | clean |
| `cargo test --workspace` | pass |
| `pnpm audit` (`ui/`) | **2 HIGH** — js-yaml, nanoid |
| `pnpm audit --prod` (`website/`) | pass (nanoid HIGH masked by `--prod`) |

Decision taken: CORS default gets **flip + release-note migration path**, making 1.1.0 carry
a breaking behaviour change (documented in a `### Breaking` CHANGELOG section).

---

## PR 1 — Deploy blockers ✅ done

- [x] **1.1 Health probes** — `helm/.../deployment.yaml:131,137` probed `/api/v1/admin/health`,
      which is `require_admin`-gated → `403` → pod never became Ready.
  - [x] Added `GET /livez` (unconditional `200`, no I/O) in `crates/web/src/handlers/healthz.rs`
        + 2 unit tests
  - [x] Exported from `crates/web/src/lib.rs`, registered in `server/src/server_factory.rs`
  - [x] Readiness → `/healthz`, liveness → `/livez` in the chart, with a comment explaining why
        they differ
  - [x] Rewrote the probe section of `docs/high-availability.md`
  - [x] New `.github/workflows/helm-lint.yaml` — `helm lint` + rendered-manifest assertions
        (probe paths, non-root defaults, a non-default render). Put in a *new* workflow because
        `.forgejo/workflows/helm.yaml` only runs on push-to-main and cannot gate a PR.
  - [x] Verified: `helm template` renders `/livez` + `/healthz`
- [x] **1.2 Frontend audit** — `ui` CI leg was red
  - [x] `ui/pnpm-workspace.yaml`: `js-yaml: ^4.3.1`, added `nanoid: ^3.3.17`; rewrote the
        rationale comment
  - [x] `website/pnpm-workspace.yaml`: added `nanoid: ^3.3.17`
  - [x] Relocked both → js-yaml 4.3.1, nanoid 3.3.18
  - [x] Verified: `pnpm audit` clean in **both** workspaces (website clean without `--prod` too);
        `pnpm run build` ok; **311 vitest tests pass**; `pnpm run generate` produces no drift
- [x] **1.3 Version + docs**
  - [x] `Cargo.toml` `[workspace.package] version = "1.1.0"` + `Cargo.lock` (single source for
        all 7 crates)
  - [x] `helm/batlehub/Chart.yaml` `version` + `appVersion`
  - [x] `ui/package.json`, `website/package.json`
  - [x] `SECURITY.md`: dropped stale "pre-1.0 / 0.x" wording
  - [x] `CHANGELOG.md`: added the PR-1 entries under `[Unreleased]`
  - [x] Regenerated `ui/openapi.json` (`version` line only — `/livez` is correctly absent, same
        as `/healthz`, neither carries a `#[utoipa::path]`)
  - [x] ~~Regenerate `sbom-rust.cdx.json`~~ — **not needed**: `*.cdx.json` is gitignored
        (`.gitignore:45`), it is a CI build artifact, not a tracked file. Plan item was wrong.
  - [x] Promoted `[Unreleased]` → `[1.1.0] - 2026-08-10` once PRs 3–5 had added their entries
        (incl. the `### Breaking` CORS section)

## PR 2 — Container and pod hardening ✅ done

- [x] **2.1 Non-root images** (both previously ran as root)
  - [x] `Containerfile`: `COPY --chown=65532:65532` on the cache dir + `USER 65532:65532`
  - [x] `Containerfile.hardened`: same UID (deliberately — the chart pins one `runAsUser` and
        it must be right for either image), added the missing `mkdir /var/cache/batlehub` to
        its builder stage so the `--chown` copy has something to copy
- [x] **2.2 Chart security context** (both keys were `{}`)
  - [x] `podSecurityContext`: runAsNonRoot / runAsUser / runAsGroup / fsGroup 65532,
        seccompProfile RuntimeDefault
  - [x] `securityContext`: allowPrivilegeEscalation false, readOnlyRootFilesystem true,
        capabilities drop ALL
  - [x] `readOnlyRootFilesystem` — grep confirmed no runtime temp-file writes in `crates/` or
        `server/` (`tempfile` appears only in dev/test paths and the CLI). Still to be
        **confirmed on a real pod** in the scratch-namespace step of the verification gate;
        if it trips, mount an `emptyDir` at `/tmp` rather than relaxing the flag.
- [x] **2.3 Availability templates**
  - [x] `pdb.yaml`, rendered only when `replicaCount > 1`. (The "both fields set" guard was
        removed during review — see finding 7 below; `maxUnavailable` now takes precedence.)
  - [x] `networkpolicy.yaml`, opt-in, always emits a DNS egress rule first (a default-deny
        Egress policy without one breaks every upstream fetch)
  - [x] Documented the RWO-vs-multi-replica trap on `persistence`
  - [x] Verified: `helm lint` clean; PDB absent at replicaCount=1 and present at 3;
        NetworkPolicy renders when enabled; non-default render (s3 + ingress + extraHosts)
        succeeds; all three CI-guard assertions pass



## PR 3 — HTTP response hardening ✅ done (landed in 6ae42db)

- [x] **3.1 Global security headers** — `nosniff`, `X-Frame-Options: DENY`, `Referrer-Policy`
      via a `DefaultHeaders` wrap near-outermost; HSTS documented at the ingress instead
- [x] **3.2 Default `Content-Type`** on `proxy_stream` (`common.rs:249`) — one change covers
      `github.rs:46`, `gitlab.rs:44,288`, `forgejo.rs:59`, `npm/read.rs:60,130`, `openvsx.rs:101`,
      `jetbrains_marketplace/files.rs:562`
- [x] **3.3 CSP for the SPA** only (not `/proxy/*`) — **implemented differently than planned.**
      It cannot be a response header: `/scalar` loads its bundle from a CDN and dies under
      `script-src 'self'`, and `actix_files::Files` takes no per-service middleware. Shipped as
      a `<meta http-equiv>` built at build time by `ui/build/csp.ts`. Enforced, not report-only —
      the bundle is clean under it.
- [x] **3.4 CORS flip (breaking)**
  - [x] `build_cors`: empty → same-origin; `["*"]` → explicit any-origin
  - [x] `AppConfig::warnings()` entry when `["*"]` is set
  - [x] `values.yaml` documented key + `CHANGELOG` Breaking + `docs/configuration.md` upgrade note

## PR 4 — Explore visibility filter ✅ done (landed in 6ae42db)

- [x] `ExploreFilter` gains viewer descriptor (`is_admin`, `is_authenticated`, `groups`)
- [x] `db/packages/explore.rs`: filter `local_pkgs` CTE, mirroring `check_team_visibility`
      exactly (longest-prefix claim wins; `team` with no claim = deny)
- [x] Same predicate in `count_explore_packages` (so the total matches the page)
- [!] `registry_explore_stats` — **deliberately not filtered.** It takes `accessible_registries`,
      not an `ExploreFilter`, so filtering needs the viewer plumbed through a second signature
      for a count that names no package. Residual: a non-public package still contributes +1 to
      its registry's sidebar total.
- [x] `explore_package_detail`: `check_visibility` → **404** (not 403)
- [x] Populate viewer in `list.rs` / `detail.rs` / `stats.rs`
- [x] Fix stale doc comment on `check_visibility` (`read.rs` ~368)
- [x] Postgres-backed regression test written (in-memory repo returns `Ok(vec![])`, so it
      cannot catch this) — 8 tests + `task test:pg-explore`; CI needs no wiring, the
      integration job already runs `-p batlehub-adapters --test '*'` with `DATABASE_URL`
- [x] **Unplanned, found while implementing:** `packages_cache_key` ignored the viewer. With
      viewer-dependent results the first caller to populate an entry would have had their view
      served to everyone after them — the same leak by another route. Key now includes a
      viewer component.
- [ ] ⚠️ **The 8 tests have never actually run.** No container runtime here and a source build
      of Postgres failed (ICU, then bison, no root). They compile, link, and *skip* without
      `DATABASE_URL` — so they report "ok" without executing a line of the new SQL. **The
      predicate is unverified; CI's integration job is the gate.**

## PR 5 — Small correctness fixes ✅ done (landed in 6ae42db)

- [x] `inbound_webhook.rs:103` → ProxyTrust-aware `client_ip`
- [x] `clippy.toml` `disallowed-methods` for `realip_remote_addr` / `connection_info`,
      allowlisted only in `middleware/proxy_trust.rs`
- [x] `extractors.rs`: percent-decode query params + test

## PR 6 — Production hardening guide ✅ doc done · ⬜ cluster values are yours

Written up as `docs/production-hardening.md`: every default that is deliberately permissive or
off, why, and what to set — proxy trust, CORS, rate limiting, IP blocking, unauthenticated
`/metrics`, HSTS-at-the-ingress, secrets (incl. the `change-me-*` grep), storage-vs-replicas,
pod security. Ends with a two-command audit.

- [x] `docs/production-hardening.md`
- [x] `docs/high-availability.md` §3.4 CORS rewritten for the new default
- [x] `website/guide/high-availability.md` CORS section — a hand-maintained copy that had
      already drifted; a breaking change could not be left contradicting itself between them
- [ ] **Applying these to the real production values.** Needs your cluster's CIDRs, hostnames
      and secret references, so it is not something I can land blind. The checklist items
      below are the operator's to tick, not mine:
  - [ ] `[server].trusted_proxies` = ingress CIDR
  - [ ] `cors_allowed_origins` = real SPA origin(s), if the UI is served cross-origin
  - [ ] Rate limiting + IP blocking on, with the Redis/Postgres store
  - [ ] `/metrics` restricted at the ingress
  - [ ] All secrets via `secretKeyRef`; no `change-me-*` placeholders
  - [ ] TLS SANs cover `ingress.extraHosts` incl. wildcard
  - [ ] `replicaCount >= 2` + PDB + S3 storage (not the RWO PVC)


---

## CodeRabbit review on PR #95 — 10 findings, all addressed

Each verified against the code before changing anything. Three were real defects, the rest
accuracy fixes.

| # | File | Verdict | Action |
|---|---|---|---|
| 7 | `pdb.yaml` | **valid, Major** | `maxUnavailable: 0` silently rendered `minAvailable: 1` — Helm reads `0` as false. Reproduced, then switched to a *was-it-configured* test. Also dropped my own "not both" guard: values.yaml ships `minAvailable: 1`, so the guard made `maxUnavailable` unusable at all. Precedence now; 4 cases verified. |
| 8 | `values.yaml` | **valid, Major** | `readOnlyRootFilesystem: true` + filesystem storage + `persistence.enabled=false` left no writable cache path — a regression **I** introduced in PR 2. Added an `emptyDir` fallback (`persistence.ephemeralSizeLimit`, 1Gi). PVC and S3 paths unchanged; all three verified. |
| 10 | `ui/index.html` | **valid, Major** | `connect-src 'self'` breaks a cross-origin API — and this repo's own `.env` points `VITE_API_BASE_URL` at a different origin, so it was not hypothetical. Extracted `ui/build/csp.ts`, derived `connect-src` from the same build-time var the SDK uses, injected by a Vite `transformIndexHtml` plugin. 9 unit tests; both build variants verified. |
| 9 | `Taskfile.yml` | valid | Bounded the readiness loop (60s, dumps `podman logs` on timeout) and probed TCP rather than the Unix socket, which initdb's temporary server also binds. |
| 1 | `helm-lint.yaml` | valid | Two global greps passed even with the probe paths **swapped**. Now scoped per probe block — proved it by rendering a swapped manifest and confirming the guard fails. |
| 2 | `CHANGELOG.md` | valid | There is no `private` visibility; the values are `public`/`internal`/`team`. CodeRabbit caught the CHANGELOG — I had made the same error in **4 code comments**, all fixed. |
| 3 | `docs/high-availability.md` | valid | "the only two routes exempt from auth" contradicted my own `production-hardening.md`, which documents `/metrics` as unauthenticated. Reworded + cross-linked. |
| 5 | `docs/production-hardening.md` | valid | Confirmed host routing without `trusted_proxies` is a `bail!` in `validate()` — the server never starts, so the warnings endpoint is unreachable. Split fatal vs. non-fatal. |
| 6 | `networkpolicy.yaml` | valid | An ingress rule with `ports` but no `from` allows **any** source, not "any source in the cluster" as my comment claimed. Corrected in template and values. |
| 4 | `docs/production-hardening.md` | valid (style) | "wants blocked" → "wants to have blocked". |

**Deliberately not done:** the other **7** `until podman exec … pg_isready` loops in
`Taskfile.yml` share the unbounded pattern fixed in mine. They are pre-existing and outside this
PR's diff; fixing them here turns a security release into a Taskfile refactor. Worth a follow-up.

### Second review pass on `1f1efab` — 3 findings, all addressed

| # | File | Verdict | Action |
|---|---|---|---|
| 3 | `ui/build/csp.test.ts` | **valid** | The best catch of the two rounds. `toContain("script-src 'self'")` is a substring match: it stays green if someone widens the directive to `script-src 'self' 'unsafe-inline'` — exactly the regression the test claimed to block. My second assertion checked a directive ordering that can never occur, so it guarded nothing. Now pins the whole directive with its `;` terminator, plus a new test asserting the API origin widens **only** `connect-src`. Mutation-tested: injecting `'unsafe-inline'` into `buildCsp` now fails the suite. |
| 1 | `todo.md` | valid | PR 3–6 headings said done while every child box was unticked. **First fix was wrong** — a blanket tick marked PR 6 (your cluster's values) and two items I deliberately skipped as complete. Rewritten to state what actually happened. |
| 2 | `todo.md` | valid | Added `sh` to the bare fence (markdownlint MD040). |

**Note on `todo.md`:** it is a working tracker, and it got committed in `1f1efab` — so CodeRabbit
now reviews it as a source file. Happy to `git rm` it and keep it local if you would rather the
repo not carry it.

---

## Pre-tag verification gate

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
task test:pg-cache && task test:pg-local-registry && task test:pg-explore && task test:s3
task coverage-check
task security
cd ui && pnpm run build && pnpm run test
helm template helm/batlehub | kubectl apply --dry-run=server -f -
```

Then build the image, confirm Trivy is green, deploy to a scratch namespace and verify the pod
reaches **Ready** as uid 65532 with a read-only root filesystem.

---

## Progress log

- 2026-08-10 — file created; baseline gates recorded; starting PR 1.
- 2026-08-10 — PRs 1-6 all implemented and committed as `6ae42db` on `security/release-1.1.0`,
  opened as PR #95. The per-item checkboxes above went stale when this (untracked) file was
  reverted; `git show 6ae42db --stat` is the authoritative record of what landed.
- 2026-08-10 — CodeRabbit reviewed PR #95: 10 findings, all addressed (table above).
