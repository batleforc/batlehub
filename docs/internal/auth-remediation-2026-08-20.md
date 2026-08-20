# Auth remediation — batlehub

**Date:** 2026-08-20
**Scope:** The full authentication surface — the five `AuthProvider` implementations, the
auth middleware and extractors, the OIDC SSO handlers, the PAT endpoints, the Postgres
token repository, the SPA callback handling, and the CLI login flow.
**Status:** Findings 1–16 are **landed**. Finding 17 (identity linking) is open, and is not
a coding task — it needs a product decision first; see the last section.

Companion to `security-survey-2026-06-12.md`, which covered path handling, SQL and XXE.
That survey concluded "the auth middleware fails closed"; that remains true of the
*middleware*, and this document is about the providers and flows on either side of it.

---

## Summary

| # | Finding | Severity | Status |
| --- | --- | --- | --- |
| 1 | The OIDC `state` is never validated server-side | **Critical** | **Landed** |
| 2 | No PKCE on the authorization code flow | **Critical** | **Landed** |
| 3 | `create_token` compared `auth_provider` to the literal `"oidc"` | **Critical** | **Landed** |
| 4 | No audience validation on either OIDC provider | **Major** | **Landed** |
| 5 | Non-JWT credentials forwarded to the Kubernetes API server | **Major** | **Landed** |
| 6 | TokenReview `status.audiences` not verified | **Major** | **Landed** |
| 7 | PAT `user_id` is not qualified by provider | **Major** | **Landed** |
| 8 | The access token, not the ID token, carries identity | **Major** | **Landed** |
| 9 | Tokens returned in the callback query string | Moderate | **Landed** |
| 10 | PATs have no scannable prefix, no `last_used_at`, no audit trail | Moderate | **Landed** |
| 11 | Discovery `issuer` not compared to the configured `issuer_url` | Moderate | **Landed** |
| 12 | `POST /auth/oidc/refresh` is unauthenticated and unthrottled | Moderate | **Landed** |
| 13 | A provider unreachable at startup is skipped silently | Moderate | **Landed** |
| 14 | PAT role is frozen for up to 90 days | Moderate | **Landed** |
| 15 | One TokenReview round trip per request, uncached | Low | **Landed** |
| 16 | The CLI generates a CSRF state it never checks | Low | **Landed** |
| 17 | No identity linking between OIDC and Kubernetes | Design gap | **Open** — needs a decision |

What was already right, and must not regress: JWKS caching with a bounded refresh, `iss`
validation, expired tokens returning `Ok(None)` rather than an error, Argon2id for static
tokens with a non-short-circuiting comparison, 32 bytes of CSPRNG per PAT stored only as a
SHA-256 digest, revocation and expiry filtered in SQL rather than in the caller, the
Kubernetes CA pinned explicitly, and the service account token re-read on every call to
survive rotation.

---

## Two design decisions that changed during implementation

Both were recommended one way in the original plan and implemented another. Recording why,
because the reasoning is not visible from the diff.

### The login state lives in Postgres, not in a signed cookie

The plan recommended an HMAC-signed `HttpOnly` cookie carrying `state` and the PKCE
verifier. **That cannot work for the CLI.** `cli/src/api/auth.rs` fetches the authorization
URL with a *non-redirecting* request and prints it for the user to open in a browser: a
cookie set on that response lands on the CLI's throwaway HTTP client and is discarded, and
the browser that completes the flow has none. Every CLI login would have failed at the
callback.

`LoginStateStore` (`crates/core/src/ports/auth/login_state.rs`) keys server-side state by
the `state` parameter instead, which both clients carry identically. Postgres backs it
rather than Redis: the server already requires a database, a login is one row written and
deleted, and `DELETE … RETURNING` gives one-time redemption across replicas for free.

The trade is that the store cannot bind a flow to a particular browser — it cannot tell two
browsers apart. That is what the `spa_state` round trip does, and it is why both halves
exist. Neither substitutes for the other, and the doc comments on `oidc_login` and
`handleOidcCallback` say so at both ends.

### Token management requires an interactive login, not merely authentication

Finding 7 was scoped to qualifying ownership by provider. Doing so surfaced that
`list_tokens` and `revoke_token` accepted *any* authenticated identity. A leaked PAT could
therefore enumerate its victim's other tokens and revoke them — a denial of service on top
of the compromise.

All three token endpoints now require a session from a configured OIDC provider, the same
rule `create_token` already had. **This is a behaviour change**: `batlehub-cli auth token
list|revoke` with a static token from `config.toml` now returns 403 instead of working. The
normal flow is unaffected — `auth login` stores an OIDC session, which is what the CLI uses.

---

## What landed, by finding

### 1, 2, 9, 16 — the browser flow

- `oidc_login` generates its own unguessable `state`, records provider + PKCE verifier +
  nonce + the caller's `spa_state` under it, and sends only the handle. The callback
  consumes the entry (`take` deletes as it reads), so a replayed callback finds nothing.
- PKCE S256 throughout: `PkceChallenge::generate`, `code_challenge` in the authorization
  URL, `code_verifier` on redemption — sent even with a `client_secret`, per RFC 9700.
- The provider is read from the stored entry, so a code can only be redeemed at the token
  endpoint that issued it. `split_combined_state` and its `or_else(|| flows.first())`
  fallback are gone; a comment in `oidc/mod.rs` records what they used to allow.
- Tokens come back in the URL **fragment**, so they never reach the server hosting the SPA.
- `handle_auth_login` compares the echoed `oidc_state` against the `csrf` it generated —
  that value was written and never read before.
- The comment at the old `sso.rs:166` claiming CSRF was "prevented by the `state` parameter
  validated above" is deleted. Nothing validated it; that claim is why the gap survived
  review.

### 4, 11 — token acceptance

- `audiences` on `[[auth]] type = "oidc"`, defaulting to `[client_id]` — never unchecked.
- `audience` is **required** on `type = "actions-oidc"` and startup fails without it. That
  issuer signs for every repository on GitHub, so `iss` alone means "some Actions job
  somewhere".
- `required_spec_claims` makes `aud` and `iss` mandatory, not merely checked-if-present.
  jsonwebtoken validates an audience **only when the claim exists**, so a token omitting
  `aud` passed a check that a token with the wrong `aud` failed. A test caught this.
- Wrong or missing `aud`/`iss` returns `Ok(None)`, not `Err`: not-for-us is not a provider
  fault, and the next provider still gets its turn.
- `OidcAuthProvider::new` fails when the discovery document declares a different issuer than
  `issuer_url` (OIDC Discovery §4.3), and config validation rejects a non-HTTPS `issuer_url`
  outside loopback.

### 7 — PAT ownership

Migration 037 adds `user_tokens.provider`; `TokenOwner` is `(provider, user_id)` and every
query matches on both. Name uniqueness moved to `(provider, user_id, name)`.

### 8 — identity from the ID token

`OidcTokens` gains `session_token` — the ID token when the provider issues one, the access
token otherwise. The nonce is checked on the callback (OIDC Core §3.1.3.7 step 11). A
provider returning neither an `id_token` nor a JWT access token now fails the *login*, with
an `error!` naming the cause; before, every subsequent request quietly resolved to anonymous
and the symptom read as a permissions problem.

### 3, 10, 12, 13, 14, 15 — the rest

- `OidcProviderNames` replaces the hardcoded `"oidc"` (see below).
- PATs are minted as `bh_pat_<64 hex>` so secret scanners can see them; `last_used_at` is
  recorded, throttled to one write a minute per token in-process *and* in SQL; creation and
  revocation log at `info` with the id and owner, never the token.
- `/auth/oidc/refresh` is throttled per client IP, 30/minute, process-local on purpose: a
  shared store would put a network dependency in front of the one endpoint a client hits
  when its session is already failing.
- `required` on `[[auth]]` (default `true` for `oidc`, `false` for `actions-oidc`) makes an
  unreachable IdP fatal; the degraded path raises `batlehub_auth_provider_down`.
- TokenReview verdicts are cached for 60 s, keyed by token hash. Successes only — caching a
  rejection would lock out a service account whose RoleBinding just landed.

### 3 — the hardcoded provider name

`identity.auth_provider != Some("oidc")` locked token creation out of every deployment that
renamed its provider, and the test suite pinned the same literal so it stayed green.
Replaced with an allow-list built from the configured providers; empty denies.

### 5, 6 — Kubernetes

`looks_like_a_jwt` keeps non-JWT credentials (i.e. PATs) out of TokenReview bodies, and
`audiences_are_confirmed` requires the API server to confirm the token is bound to a
requested audience. **Operator-visible**: workloads must present a *projected* token minted
for the configured audience, not the default mounted one. Documented in
`docs/guide/configuration.md` §3.3.3 with the volume snippet and the log line to grep for.

---

## Verification

`cargo test --workspace`: 3515 passed, 104 suites. `ui`: 1045 passed.
`cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --all --check`: clean.
`ui/openapi.json` and `ui/src/client/` regenerated.

Each of the three highest-severity fixes was verified by neutralising it and confirming the
new tests fail: 4 SSO tests for the state check, 3 Kubernetes tests for the JWT and audience
guards, 10 token tests for the provider allow-list.

---

## Open: finding 17 — identity linking

**Needs a product decision before any code.** There is no linking mechanism: `Identity`
carries `user_id` and `auth_provider`, the middleware stops at the first provider that
answers, and the same person arriving by OIDC and by a Kubernetes service account is two
unrelated identities with disjoint quotas, namespaces, tokens and history.

The only bridge that exists is the RBAC wildcard `"*:team-a"`
(`crates/core/src/rules/rbac.rs:68`), which matches a group name regardless of provider
prefix. That is string equality, not proof that two identities are the same principal — and
on the `actions-oidc` side group names are rendered from claims the caller controls
(`group_template`).

If linking is wanted, the shape is:

- `identity_links (provider, external_id, canonical_user_id)`, unique on the first two.
- Linking is initiated **from an authenticated OIDC session only**, never the reverse: the
  human proves who they are, then claims the machine identity. A service account must not be
  able to attach itself to a human.
- The middleware resolves `canonical_user_id` after provider selection, and everything
  downstream that keys on `user_id` (quotas, `me/*`, ownership, PATs) uses the canonical
  value.
- `user_tokens.provider` (migration 037, landed) is the prerequisite — without it there was
  nothing to migrate onto a canonical id.

Worth writing as a proper RFC (next free number: 0014) rather than a task, because it
changes the meaning of `user_id` across the codebase.

---

## Upgrade notes

Four changes are operator-visible and belong in release notes:

1. **`audience` is now required** on `[[auth]] type = "actions-oidc"`. Startup fails without
   it. Workflows must request it (`core.getIDToken('<audience>')`).
2. **Kubernetes workloads must use a projected token** bound to the configured audience.
   The default mounted service account token is now refused.
3. **`batlehub-cli auth token list|revoke` needs an OIDC session.** Static tokens get 403.
4. **An unreachable OIDC provider now fails startup** by default. Set `required = false` on
   the `[[auth]]` entry to keep the previous warn-and-continue behaviour.

Migrations 036–038 run automatically. 037 backfills `user_tokens.provider` to `'oidc'`,
which is correct: it is the only value the old check ever let through.
