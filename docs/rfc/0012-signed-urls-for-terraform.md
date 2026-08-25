# RFC 0012 — Signed URLs for the request that carries no credential

| Field      | Value                                                                       |
| ---------- | --------------------------------------------------------------------------- |
| Status     | Draft                                                                       |
| Short      | Signed URLs for the credential-less request |
| Settles    | Letting a client that sends no credential — Terraform's provider archive — download from a registry that is closed to everyone else |
| Author     | batleforc                                                                   |
| Co-author  | —                                                                           |
| Created    | 2026-08-18                                                                  |
| Supersedes | —                                                                           |
| Touches    | `crates/config`, `crates/core` (new `signed_url` service), `crates/web` (terraform handlers), docs |

---

## 1. Summary

Terraform authenticates the two JSON documents of a provider install and then
fetches the provider archive **with no `Authorization` header**. Measured against
Terraform 1.8.5 in RFC 0009 §12.3, and it is not a configuration mistake: the
client has no mechanism to send one on that request.

The consequence today is written into `docs/registries/terraform.md`: a mirror
registry needs `anonymous = ["releases:read", "source:read"]`. That grant is not
shaped like the problem. It is per *registry*, so opening the last step of a
provider install opens every read on that registry to everybody — every other
provider, every version listing, and in hybrid mode everything published locally.

This RFC mints a **signed, expiring, single-coordinate URL inside the document
that was authenticated**, and teaches the archive route to accept that signature
as evidence of the authentication that already happened. The signature carries
the identity that fetched the document; verification reconstructs it and runs the
**same rule chain as before**. It authenticates a request; it authorises nothing.

### Before / after

```text
# today — the registry must be open to everyone for the last step to work
[registries.rbac]
anonymous = ["releases:read", "source:read"]

GET {mirror}/registry.terraform.io/hashicorp/random/5.40.0.json    [auth]
  → {"archives": {"linux_amd64": {"url": "../../../v1/providers/…/artifact/linux/amd64"}}}
GET {mirror}/v1/providers/hashicorp/random/5.40.0/artifact/linux/amd64
                                                                   ← no auth,
                                                                     allowed because
                                                                     anonymous is

# with this RFC — the registry can be closed
[registries.rbac]
anonymous = []

GET {mirror}/registry.terraform.io/hashicorp/random/5.40.0.json    [auth as alice]
  → {"archives": {"linux_amd64": {"url": "../../../v1/providers/…/artifact/linux/amd64?bh_sig=1.…"}}}
GET {mirror}/v1/providers/hashicorp/random/5.40.0/artifact/linux/amd64?bh_sig=1.…
                                                                   ← no auth header,
                                                                     signature says
                                                                     "alice, this
                                                                     coordinate, for
                                                                     the next 5 min"
```

The audit row for the download names `alice`, which it does not today.

---

## 2. Motivation

1. **The workaround is registry-wide and the problem is one request.**
   `[registries.rbac]` has no per-route granularity, so `anonymous =
   ["releases:read", "source:read"]` — the fix `terraform.md` documents — is the
   only lever, and it hands an unauthenticated caller every read the registry
   serves. On the instance this was written against, five of eleven registries
   carry that grant, and only two of them need it.

2. **The estate loses the actor, not just the gate.** With anonymous granted the
   rule chain still runs, so blocks and the licence gate still refuse — but the
   chain evaluates *anonymous*, so group grants never apply, quota is charged to
   nobody, and every provider download in the estate is audited with no actor.
   `terraform_provider_artifact` already calls `proxy_stream(…, RELEASES_READ)`;
   the machinery is there and it is being fed an empty identity.

3. **RFC 0009 named this and deferred it, for a reason that has expired.** §12.3
   says minting a signed URL "would be a new auth mechanism invented for one
   client". Since then the same shape has been recorded twice more: the VS Code
   gallery (`vscode-marketplace.md`, and now RFC 0011) and this. A mechanism for
   one client is a special case; a mechanism for a *class* of clients — those
   whose protocol fetches a URL a server chose, without credentials — is a
   primitive.

4. **The documented alternative cannot work here.** "An authenticating ingress in
   front of BatleHub" is offered on both pages. An ingress can only authenticate
   a request that carries something to authenticate; this one carries nothing.
   The advice is sound for the *documents* and empty for the archive.

---

## 3. Goals / non-goals

**Goals**

- `terraform init` completes against a registry whose `anonymous` grant is empty.
- The archive request is authorised as the identity that fetched the document
  that named it, with the same rules, quota and audit as any other download.
- No new client software, no patched Terraform, no wrapper: the mechanism lives
  entirely in a URL the client already follows.
- Off by default, and enabling it changes nothing for clients that do
  authenticate.
- Stateless verification, so it holds across replicas without a shared store.

**Non-goals**

- A general "download token" API for users to mint by hand. The only minter is a
  handler that has just authenticated a request for the same coordinate.
- Solving the VS Code gallery. RFC 0011 owns that and takes a different route
  (a credential the editor is patched to send). This RFC should leave a primitive
  0011 or its successor *could* reuse, and should not wait for it.
- Changing the mirror or registry protocol documents in any way Terraform can
  observe beyond the `url` value.
- Replacing `[registries.rbac]`. A signed URL never grants a permission the
  identity inside it does not have.

---

## 4. User-facing design

### 4.1 Configuration

One new block, global, because the key is an instance secret rather than a
registry property:

```toml
[server.signed_urls]
# 32 bytes minimum, from the environment — the config loader already interpolates
# ${VAR} (see `docs/guide/configuration.md`), and a signing key does not belong
# in a file that gets committed.
secret = "${BATLEHUB_URL_SIGNING_SECRET}"
# How long a minted URL stays valid. Terraform follows it immediately.
ttl_seconds = 300          # default; hard-capped at 3600
# Verified but never minted with — for rotation without a flag day.
previous_secrets = ["${BATLEHUB_URL_SIGNING_SECRET_OLD}"]
```

and one per-registry switch, defaulting to `false`:

```toml
[[registries]]
type = "terraform"
name = "tf"
signed_downloads = true

[registries.rbac]
anonymous = []             # now possible
user = ["releases:read", "source:read"]
```

`signed_downloads = true` with no `[server.signed_urls].secret` is a
**startup error**, not a warning: a registry that believes it is closed and is
not is the failure this RFC exists to prevent. It joins the existing
`config.validate()` checks (`crates/config/src/schema/mod.rs`).

### 4.2 What an operator sees

- A provider install works with `anonymous = []`.
- `GET /api/v1/audit` shows the provider download with `actor = alice`, where it
  showed nothing before.
- An expired or tampered URL answers `403` with code `signed-url.invalid` and a
  message that says which of the three it was — expired, wrong coordinate, or bad
  signature — because an operator debugging a clock-skewed runner should not have
  to guess.

### 4.3 What a reader of the JSON sees

`{version}.json`, mirror protocol — the only change is the query string:

```json
{
  "archives": {
    "linux_amd64": {
      "url": "../../../v1/providers/hashicorp/random/5.40.0/artifact/linux/amd64?bh_sig=1.eyJ2IjoxLCJyZWciOiJ0ZiIsInBrZyI6InByb3ZpZGVycy9oYXNoaWNvcnAvcmFuZG9tIiwidmVyIjoiNS40MC4wIiwiYXJ0IjoibGludXgvYW1kNjQiLCJzdWIiOiJhbGljZSIsInJvbGUiOiJ1c2VyIiwiZXhwIjoxNzY3MjI1NjAwfQ.G8mR…"
    }
  }
}
```

The relative-URL arithmetic RFC 0009 §12.3 confirmed is untouched: the query
string rides along, because a relative reference resolves the path and keeps the
query it was written with.

---

## 5. Architecture

Three pieces, one of them new:

```text
crates/core/src/services/signed_url.rs      ← new: mint(), verify()
crates/web/…/terraform/discovery.rs         ← mints, in the document handler
crates/web/…/terraform/providers/read.rs    ← verifies, before the rule chain
```

The minting site is the invariant that makes this safe: **only a handler that has
just authenticated a request for the same coordinate may mint a URL for it.**
`terraform_mirror_version` already runs `mirror_versions(…, identity)` through
the rule chain before it builds the `archives` map — so by the time it writes a
`url`, the caller has been authenticated *and* authorised for that provider. The
signature records that verdict; it does not create one.

Verification runs before anything else in the artifact handler and produces an
`Identity`, which is then fed to the existing path unchanged:

```text
request → [signature present?] ─no→  AuthIdentity (header)  ─┐
                              └yes→  verify → Identity      ─┴→ proxy_stream(rules, quota, audit)
```

Nothing downstream knows which branch it came from, which is the point: there is
one authorisation path, and this adds a second way to *authenticate* into it.

---

## 6. Detailed design

### 6.1 Token format

```text
bh_sig = "1." base64url(payload_json) "." base64url(mac)
```

`1` is the version *and* the algorithm selector, so a future move to a different
primitive is a new prefix rather than a negotiation.

```json
{
  "v": 1,
  "reg": "tf",
  "pkg": "providers/hashicorp/random",
  "ver": "5.40.0",
  "art": "linux/amd64",
  "sub": "alice",
  "role": "user",
  "grp": ["platform"],
  "exp": 1767225600
}
```

`grp` is present because `[registries.rbac.groups]` grants exist and a token that
dropped them would silently downgrade a group-authorised caller to their role's
permissions — a refusal at the last step of an install that worked yesterday.

### 6.2 What is signed

The MAC covers the canonical string

```text
"bh-signed-url:v1\n" + method + "\n" + registry + "\n" + package + "\n" +
version + "\n" + artifact + "\n" + subject + "\n" + role + "\n" +
groups.join(",") + "\n" + exp
```

built from the **path components of the request being verified**, not from the
payload's copy of them. A payload field that disagrees with the path therefore
fails the MAC, which is what stops a signature for `random/5.40.0` being replayed
against `aws/6.0.0` by editing the path and leaving the query alone.

`method` is in there so a `GET` signature cannot be presented to the `PUT`
publish route that shares the path shape
(`providers/{ns}/{type}/{ver}/artifact/{os}/{arch}` is both).

### 6.3 Primitive

HMAC-SHA256: `hmac` (RustCrypto) over the `sha2` already in the tree, verified
with `Mac::verify_slice`, which is constant-time. No new RSA-adjacent
dependency — `deny.toml` bans that family (RUSTSEC-2023-0071) and this stays
inside the ban.

Ed25519 is already a dependency (`crates/core/src/services/signature.rs`) and was
considered instead. Rejected here: asymmetric signing buys a verifier that does
not hold the secret, and the verifier *is* the signer in this design. It costs a
larger token and a slower verify for a property nothing needs.

**Per-registry subkeys.** The MAC key is not the configured secret but
`HMAC(secret, "registry:" || registry_name)`. A key that leaks out of one
registry's blast radius — a log, a memory dump of one worker — cannot mint for
another, and the derivation is free.

### 6.4 Expiry and replay

`exp` is absolute UNIX seconds, `ttl_seconds` from config, capped at 3600 so a
misconfiguration cannot mint a month-long credential. Verification allows 60
seconds of backward clock skew and none forward.

Until `exp`, a minted URL **is a bearer capability for one coordinate**. That is
the honest description and it is the security cost of the design (§7). Optional
single-use would need shared state across replicas — the `[cache]` backend can
provide it (Postgres or Redis, already supported for exactly this kind of
cross-replica fact) — but it is proposed **off**, as `single_use = false`,
because turning it on makes a provider install fail on a retried request, and
Terraform retries.

### 6.5 Which routes verify

All three that Terraform fetches after an authenticated document:

| route | fetched by | today |
| --- | --- | --- |
| `…/{version}/artifact/{os}/{arch}` | provider zip | rule chain, `RELEASES_READ` |
| `…/{version}/shasums` | checksum file | rule chain, `RELEASES_READ` |
| `…/{version}/shasums.sig` | checksum signature | rule chain, `RELEASES_READ` |

The last two matter as much as the first: RFC 0009 made them proxy through
BatleHub precisely so an air-gapped install does not reach the internet at its
last step, and they are fetched with the same absence of credentials. A design
that signed only the zip would leave `terraform init` failing one line later.

### 6.6 What verification does not do

It does not skip the rule chain, the block list, the release-age gate, the
licence gate, the visibility check or quota. It replaces exactly one thing — the
`Authorization` header — and hands the same `Identity` to the same code. A
blocked version stays blocked for a signed URL minted before the block, because
the block is evaluated at redemption, not at minting.

---

## 7. Security considerations

| Risk | Answer |
| --- | --- |
| A signed URL is a credential in a URL | Scoped to one registry, one package, one version, one platform, one method; expires in five minutes; grants no permission the subject lacks. A leak is a five-minute licence to download one file the subject could already download. |
| It appears in access logs and proxy logs | True, and unavoidable for this client. Mitigated by the TTL and the scope, and BatleHub's own request logging must be checked to confirm it does not record query strings for these routes — an audit item in phase 6, not an assumption. |
| Privilege escalation by editing the payload | The MAC covers subject, role and groups; any edit invalidates it. The minting handler copies the *caller's* identity verbatim and has no path that widens it. |
| Signature accepted when the feature is off | Verification is not wired in unless `signed_downloads = true`. A `bh_sig` on a registry with it off is an ignored query parameter, and the request is authenticated by header or not at all. |
| Key material in a config file | `secret` is `${VAR}`-interpolated like every other credential in this config (`docs/guide/configuration.md` §"Sensitive values"), and a literal shorter than 32 bytes is a startup error. |
| Replay within the TTL | Accepted, and stated. `single_use = true` is available for operators who want it and costs retry-safety; see §6.4. |
| Downgrade to anonymous | Removing the anonymous grant is what makes this worth doing; the RFC does not automate it. `task docs` should carry the migration note, and the config warning system (`crates/config/src/schema/warnings.rs`) can raise one when a terraform registry has both `signed_downloads = true` and a non-empty `anonymous` grant — the belt-and-braces state is legal but probably unintended. |

---

## 8. Alternatives considered

1. **Keep the anonymous grant (today).** Simple, documented, and the reason this
   RFC exists: it opens every read on the registry to make one work.
2. **Authenticating ingress.** Recommended on both registry pages; cannot
   authenticate a request that carries no credential.
3. **IP allowlist for the archive route.** Fails the case this is for — shared CI
   runners and developer laptops are the callers, and an allowlist that covers
   them covers the internet in a cloud estate.
4. **mTLS.** Terraform will not present a client certificate to a mirror.
5. **Serve the archive from an ungated route.** Explicitly rejected by RFC 0009
   §12.3, and rightly: it deletes the gate rather than moving it.
6. **Ed25519 instead of HMAC.** §6.3 — no verifier needs to be separate from the
   signer.
7. **A per-user URL prefix (`/u/{session}/…`).** Server-side state, a second
   routing dimension, and the same bearer-capability property with a longer life.

---

## 9. Rollout and compatibility

Off by default; no behaviour changes for any existing deployment until an
operator sets `signed_downloads = true`. With it on and the anonymous grant still
in place, both paths work — which is the recommended order: enable signing,
confirm an install works, *then* empty `anonymous`.

No client-side change, no version floor on Terraform, nothing to communicate to
users of the registry. The one visible artefact is a longer `url` in a document
nobody reads by hand.

---

## 10. Test plan

Unit (`crates/core/src/services/signed_url.rs`):

- round-trip mint → verify for each field combination, including empty groups;
- **tamper each field of the payload independently** and assert rejection;
- edit the *path* while keeping a valid signature (the §6.2 case) — rejected;
- present a `GET` signature to the `PUT` route — rejected;
- expiry, including the 60-second skew allowance and its boundary;
- rotation: a token minted under `previous_secrets[0]` verifies, and a token
  minted now uses `secret`;
- a signature minted for registry A rejected by registry B (subkey derivation).

Integration (`crates/web/tests/terraform.rs`):

- the full mirror flow with `anonymous = []`: `index.json` and `{version}.json`
  with a token, then the archive with **no** header and the minted query — `200`;
- the same archive with no header and no signature — `403`;
- a signed request for a **blocked** version — refused, proving the rule chain
  still runs at redemption;
- the audit row for a signed download names the subject, not `anonymous`;
- `shasums` and `shasums.sig` likewise.

Real client, per RFC 0009's standard, and the only test that can catch a document
that is well-formed and wrong:

- `terraform init` against a registry with `anonymous = []`, Terraform 1.8.5,
  through `examples/terraform/`, with the mirror on `https:` (§12.3's other
  finding). A conformance entry marked `observed` records what was captured.

---

## 11. Decisions and open questions

| # | Question | Recommendation |
| --- | --- | --- |
| O1 | Default TTL. | 300 s. Terraform follows the URL within milliseconds; the margin is for a slow runner, not for a human. |
| O2 | `single_use` on by default? | **No** (§6.4) — Terraform retries, and a one-shot URL turns a retry into a failed install. |
| O3 | Sign `shasums` / `shasums.sig` too? | **Yes** (§6.5). Signing only the zip leaves the install failing one step later, with a message that points at checksums rather than at auth. |
| O4 | Generalise the primitive now, or after RFC 0011 lands? | Build it registry-agnostic in `core` and wire it to Terraform only. 0011's client is patched to send a header, so it does not need this — but the *next* protocol of this shape should not have to invent it again. |
| O5 | Does the **registry** protocol have the same hole? | **Unmeasured, and it must be measured before phase 5.** `terraform_provider_download` returns a `download_url` on our own host; whether Terraform sends the `credentials` token when following it to the same host it just authenticated to is exactly the kind of assumption RFC 0009 §12.3 was written about. A reading is not an answer here. |
| O6 | Should the console show that a registry is closed but installable? | Deferred. The Setup Guide's Terraform snippet will need the `signed_downloads` line either way. |

---

## 12. Implementation phases

| # | Phase | Lands even if the rest slips |
| --- | --- | --- |
| 1 | `signed_url.rs` in `core` — mint, verify, subkey derivation, the full unit suite; `[server.signed_urls]` in `config` with the startup validation of §4.1. | Yes — a tested primitive with no caller. |
| 2 | Mint in `terraform_mirror_version`; `signed_downloads` per registry. | No effect until phase 3 verifies. |
| 3 | Verify on `terraform_provider_artifact`, ahead of the rule chain. | The goal in §3 is met for the zip. |
| 4 | The same for `shasums` and `shasums.sig`. | `terraform init` completes end to end. |
| 5 | Measure the registry protocol (O5); extend or record the answer. | Either way the finding is written down. |
| 6 | Docs: rewrite the warning in `docs/registries/terraform.md` from "you must open the registry" to "you may, or you may sign"; the configuration reference; the log-hygiene audit of §7. | — |
| 7 | Real-client run and a conformance entry marked `observed`. | The claim becomes a measurement. |

---

## See also

- [RFC 0009 — Every endpoint the client actually calls](/rfc/0009-protocol-coverage) — §12.3 is the measurement this RFC starts from, and the reason it was deferred
- [RFC 0011 — Authenticated OpenVSX registry access](/rfc/0011-openvsx-login) — the same shape with a client that can be patched
- [Terraform registry](/registries/terraform) — the constraint as it is documented today
