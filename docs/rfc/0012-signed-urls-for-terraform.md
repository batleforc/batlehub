# RFC 0012 — Signed URLs for the request that carries no credential

| Field      | Value                                                                       |
| ---------- | --------------------------------------------------------------------------- |
| Status     | **Implemented** — all seven phases landed; §12 records what each one changed. Two of this document's own claims were reversed by measurement rather than argument: O5 (§11) was settled by a real Terraform run rather than a reading, and §12's phase-4 row claimed an end-to-end completion the mirror protocol had already reached at phase 3. Two security reviews of the implementation found defects in it, both recorded in §6.2 and §7.1 with the tests that now pin them |
| Short      | Signed URLs for the credential-less request |
| Settles    | Letting a client that sends no credential — Terraform's provider archive — download from a registry that is closed to everyone else |
| Author     | batleforc                                                                   |
| Co-author  | Claude Opus 5 (1M context) <noreply@anthropic.com>                          |
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
- `GET /api/v1/admin/audit-log` shows the provider download with `actor = alice`,
  where it showed nothing before. (This section said `/api/v1/audit` until the
  phase-7 run tried it and got an empty body.)
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

**Measured, because §12.3 confirmed relative *path* resolution and this needs
one more property from it.** Terraform 1.8.5 against a network mirror serving
exactly the URL above, 2026-08-25:

```text
GET /registry.terraform.io/hashicorp/aws/index.json      query: (none)
GET /registry.terraform.io/hashicorp/aws/1.0.0.json      query: (none)
GET /v1/providers/hashicorp/aws/1.0.0/artifact/linux/amd64
                                                         query: bh_sig=1.PAYLOAD.MAC
```

The path resolved three levels up as intended and the query arrived byte-for-byte,
dots and all, with no percent-encoding applied. Phase 2 mints into this exact
shape, so the document it produces is one a real client follows — which is the
half of the design that cannot be proven by a unit test, and the half that would
have been discovered in phase 3 with the verifier already written.

The registry protocol's three URLs are **absolute** rather than relative, so they
do not depend on that resolution — but they are what phase 5 mints into, and the
same run measured them rather than assuming the easier case holds:

```text
GET /artifact/samehost.zip                query: bh_sig=1.ZIP.MAC
GET /artifact/samehost.SHA256SUMS         query: bh_sig=1.SUMS.MAC
GET /artifact/samehost.SHA256SUMS.sig     query: bh_sig=1.SIG.MAC
```

Three fields, three distinct queries, each arriving on its own URL, with
`terraform init` exiting `0`. Both minting sites therefore emit documents a real
client follows.

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

The MAC covers a canonical string in which **every field is length-prefixed**,
netstring-style — `<byte-length>:<value>`, with the group list preceded by its
own count:

```text
"bh-signed-url:v1\n"
  + len(method)   + ":" + method
  + len(registry) + ":" + registry
  + len(package)  + ":" + package
  + len(version)  + ":" + version
  + len(artifact) + ":" + artifact
  + len(subject)  + ":" + subject
  + len(role)     + ":" + role
  + len(count)    + ":" + count        // number of groups
  + (len(g) + ":" + g  for each group)
  + len(exp)      + ":" + exp
```

built from the **path components of the request being verified**, not from the
payload's copy of them. A payload field that disagrees with the path therefore
fails the MAC, which is what stops a signature for `random/5.40.0` being replayed
against `aws/6.0.0` by editing the path and leaving the query alone.

**The length prefixes are the security property, not formatting.** This RFC
originally specified `\n`-joined fields with `groups.join(",")`, and that
encoding is not injective: a value containing the delimiter shifts the boundary,
so a single MAC covers two different tuples. The security review of the
implementation found it, and found it reachable — `validate_path_safe` permits
control characters and actix percent-decodes path segments, so `%0A` in a
published package name arrives at the MAC as a real newline.

The attack it enables is worth stating, because it is the reason the encoding
matters. Mint at a coordinate you control whose package name carries the extra
lines; keep the MAC bytes; re-split the payload into the *victim's* coordinate
with `"role": "admin"`. Both reconstructions were byte-identical, so it
verified. And **minting and redemption do not run the same rules** — minting
authorizes through `authorize_listing`, which is the RBAC rule alone, while
redemption runs the full chain, every gate of which has a `bypass_roles` list
operators fill with `admin`. The comma had the same shape one field over: a
single group `"a,b"` re-splitting into two.

Length-prefixing removes the class rather than the instance. There is no
delimiter to hide inside, because none is being looked for: read digits to the
`:`, take exactly that many bytes. `distinct_inputs_never_share_a_canonical_string`
in `crates/core/src/services/signed_url.rs` asserts injectivity over a table of
adversarial values, and `a_payload_reslit_across_field_boundaries_is_rejected`
pins the specific attack; both fail against the encoding this section used to
specify.

Two things this deliberately does **not** do. It does not reject control
characters in coordinates — that belongs in `validate_path_safe`, and is worth
doing separately as defence in depth for every registry type rather than for
this one MAC. And it does not bump the token version: the change is fail-closed,
because a token minted under the old encoding no longer verifies rather than
being misinterpreted, and the feature has never shipped enabled.

`method` is in there so a `GET` signature cannot be presented to the `PUT`
publish route that shares the path shape
(`providers/{ns}/{type}/{ver}/artifact/{os}/{arch}` is both).

In the implementation this is the **second** line rather than the first: the
publish handler never calls the verifier, so a `bh_sig` on a `PUT` is an ignored
query parameter rather than a signature that fails to match. Both are kept
deliberately. Verification is wired per route, and the day someone has a reason
to wire it into a write path, the method binding is what stops a download URL
becoming an upload credential — the outer defence would be gone and nobody would
have had to think about the inner one. `a_download_signature_cannot_authenticate_a_publish`
in `crates/web/tests/terraform.rs` pins the outer one and says the question was
asked.

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
| It appears in access logs and proxy logs | True, and unavoidable for this client. Mitigated by the TTL and the scope. **The phase-6 audit is done and it did not confirm what this row hoped it would — see §7.2.** |
| Privilege escalation by editing the payload | The MAC covers subject, role and groups; any edit invalidates it. The minting handler copies the *caller's* identity verbatim and has no path that widens it. |
| Signature accepted when the feature is off | Verification is not wired in unless `signed_downloads = true`. A `bh_sig` on a registry with it off is an ignored query parameter, and the request is authenticated by header or not at all. |
| Key material in a config file | `secret` is `${VAR}`-interpolated like every other credential in this config (`docs/guide/configuration.md` §"Sensitive values"), and a literal shorter than 32 bytes is a startup error. |
| Replay within the TTL | Accepted, and stated. `single_use = true` is available for operators who want it and costs retry-safety; see §6.4. |
| Downgrade to anonymous | Removing the anonymous grant is what makes this worth doing; the RFC does not automate it. `task docs` should carry the migration note, and the config warning system (`crates/config/src/schema/warnings.rs`) can raise one when a terraform registry has both `signed_downloads = true` and a non-empty `anonymous` grant — the belt-and-braces state is legal but probably unintended. |

### 7.1 A signed URL must never name a host we do not control

The second security review of the implementation found this, and it is the one
finding in the set that was a live credential leak rather than a hardening gap.

`sign_download_document` had always declined to sign a URL that was not ours.
The check was `url.starts_with(base)`, and the comment above it called the guard
belt and braces, "because the three fields are ours by construction". **Both
halves were wrong.**

**The fields are not ours.** In local and hybrid mode the download document is
`platform.clone()` of the publisher's own `platforms[]` entry
(`local_registry/eco_terraform.rs`), and only `download_url` is overwritten —
`shasums_url` and `shasums_signature_url` come through verbatim from a manifest
uploaded over HTTP by anyone holding `Role::User` with publish rights. The proxy
path *does* overwrite all three, which is why this was invisible: the only test
covering it exercised the proxy path.

**And the check did not hold.** `registry_public_base` returns a **bare origin**
(`https://tf.acme.io`) when the request is host-routed — which the Terraform
registry protocol requires — and a URL's authority ends at `/`, `?`, `#` or `@`.
So `https://tf.acme.io.attacker.example/s`, `https://tf.acme.io-evil.net/s` and
`https://tf.acme.io@attacker.example/s` all passed a prefix match against it.

The attack: publish a provider whose platform entry points `shasums_url` at a
host you own. When *anyone else* runs `terraform init` on it, the handler signs
your URL with a token minted for **them**, and Terraform fetches it
credential-lessly. You now hold a token bearing their identity — and the
payload is plain base64url JSON, so you can simply read their `sub`, `role` and
`grp`. Redemption is still coordinate-pinned to your own package, so there is no
cross-package escalation; the loss is a credential and an identity disclosure to
an arbitrary external host.

**The fix is a structural origin comparison**, not a better prefix: parse both
sides and require identical scheme, host and port, and that the path is under
the base's prefix. That also disposes of `https://good.example@evil.example/`,
where the authority is the second host and every textual comparison says
otherwise. `is_on_origin` in `handlers/proxy/terraform/shared.rs`, with unit
tests for each bypass string and an integration test
(`a_publisher_supplied_off_host_url_is_never_signed`) that publishes a hostile
manifest and asserts the field comes back unsigned while `download_url` in the
same response is signed — so the test cannot pass by signing being off.

Two further recommendations from that review are **deliberately not taken**, and
this is the reasoning rather than an oversight:

- *Overwrite `shasums_url` / `shasums_signature_url` with our own URLs, as the
  proxy path does.* It would remove publisher control entirely, and it would
  break the only way local-mode checksums work today: the shasums routes have no
  local branch at all, so a `local` publisher who wants `terraform init` to
  verify anything must name an off-host URL. Overwriting turns a working,
  now-harmless deployment into a broken one. The token leak is closed without
  it.
- *Strip URL-bearing keys at the publish edge.* Same breakage, one step earlier.

What both would buy over the origin check is stopping BatleHub from *naming* a
third-party host in a document. That is worth doing the day the shasums routes
can serve from local storage — and measurement says that day is further off than
it looks. Terraform 1.8.5, against the same harness:

| Download document | Result |
| --- | --- |
| Checksum URLs + `signing_keys` naming a real key | installs |
| Checksum URLs **absent** | never fetched, archive downloaded, then *"checksum list has no SHA-256 hash for …"* |
| `signing_keys: {"gpg_public_keys": []}` | *"signature from unknown issuer"* |

So a checksum list is mandatory *and* it must be verifiable against a key in
`signing_keys`. `eco_terraform.rs` lets a publisher's own `signing_keys` through
(`entry(…).or_insert_with`) exactly as it lets their `shasums_url` through: the
two are a matched set, and supplying both on a host they control is **the only
configuration in which a local-mode provider installs at all**. Serving the
checksums ourselves would not replace it, because BatleHub has no key to put in
`signing_keys` — the deb and pacman signers are Ed25519 precisely because `rsa`
is banned by `deny.toml` (RUSTSEC-2023-0071), and whether Terraform's vendored
OpenPGP would accept an Ed25519 key is unmeasured.

**What was done instead.** The residual after the origin check is not a
credential leak — it is that the document names a third party, so that host sees
every `terraform init` for the provider and an air-gapped install reaches it.
That predates this RFC; what RFC 0012 added was the risk of *signing* such a
URL, which §7.1 removes. Refusing the URL would break the only working
configuration, so `terraform_provider_upload` now **tells the publisher at
publish time**: the `201` carries the field and the host it names, and a
`tracing::warn!` records it for the operator. Visibility without breakage, which
is the most the origin check can be paired with until the signing question is
answered.

### 7.2 The log-hygiene audit

This section asked phase 6 to *"confirm [BatleHub's request logging] does not
record query strings for these routes"*. It was done, and the answer is the
other one: **it does, on every request, at `INFO`.**

`server/src/server_factory.rs` wraps the app in
`TracingLogger::<BatleHubSpanBuilder>`, whose `on_request_start` calls
`tracing_actix_web::root_span!`. That macro hard-codes

```rust
http.target = %$request.uri().path_and_query().map(|p| p.as_str()).unwrap_or(""),
```

(`tracing-actix-web-0.7.22/src/root_span_macro.rs:110`, and again at `:134` for
the remote-parent arm). `path_and_query()` includes the query, so `bh_sig`
reaches the fmt subscriber and the OTLP exporter along with every other request
target.

**Not cheaply fixable in this RFC.** The field is inside the macro and
`$($field)*` only appends — re-declaring `http.target` makes `span!` reject the
duplicate. The available fixes are all larger than this change: hand-roll the
twenty-odd fields of a third-party macro and inherit its drift, add a global
middleware outside the logger that strips the parameter and stashes it in
request extensions, or write a `tracing` layer that rewrites the field. Each is
a real design decision about the logging stack, and none of them belongs in a
phase whose scope is documentation.

**So the honest position is the one now in the docs, not a claim that it is
handled.** What an operator is accepting: a five-minute, single-coordinate
capability granting no permission its subject lacked, visible to anything that
reads the request span — log shipping, OTLP, and any TLS-terminating proxy in
front of BatleHub, which was always going to see it regardless of what BatleHub
logged. The levers are `ttl_seconds`, and dropping or rewriting `http.target` in
the log pipeline. Both are documented in `docs/guide/configuration.md`
(`[server.signed_urls]`) and on the Terraform registry page.

One thing the audit *did* confirm: the **audit trail is clean**. `AccessEvent`
(`crates/core/src/entities/access_log.rs:76`) carries a `PackageId`, an actor, a
result, an IP and a user agent — no URI and no query — so
`GET /api/v1/audit` cannot leak a token however long it is retained.

Whether to redact `http.target` is recorded as §11 O7 — deferred, with the
reasoning there rather than left implicit here.

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

### Resolved

| # | Question | Decision |
| --- | --- | --- |
| O1 | Default TTL. | 300 s. Terraform follows the URL within milliseconds; the margin is for a slow runner, not for a human. |
| O2 | `single_use` on by default? | **No** (§6.4) — Terraform retries, and a one-shot URL turns a retry into a failed install. |
| O3 | Sign `shasums` / `shasums.sig` too? | **Yes** (§6.5). Signing only the zip leaves the install failing one step later, with a message that points at checksums rather than at auth. |
| O4 | Generalise the primitive now, or after RFC 0011 lands? | Build it registry-agnostic in `core` and wire it to Terraform only. 0011's client is patched to send a header, so it does not need this — but the *next* protocol of this shape should not have to invent it again. |
| O5 | Does the **registry** protocol have the same hole? | **Yes, and wider than the zip. Measured, not read** — Terraform 1.8.5, 2026-08-25; transcript below. Every protocol document is authenticated and **every artifact fetch is not**, including on the same host authenticated on the immediately preceding request. Phase 5 is therefore *extend*, not *record*. |
| O6 | Should the console show that a registry is closed but installable? | Deferred. The Setup Guide's Terraform snippet will need the `signed_downloads` line either way. |
| O7 | Redact the signature from `http.target` in the request span? | **Deferred, and documented instead** (§7.2). Raised by the phase-6 audit, which found the token *is* logged rather than confirming it is not. Every available fix — hand-rolling a third-party macro's twenty fields, a global middleware that strips the parameter before the logger, a `tracing` layer that rewrites it — is a design decision about the logging stack rather than about this feature, and would be a larger change than the feature. The operator-facing levers (`ttl_seconds`, filtering the field) are documented on both the configuration and Terraform pages; a redaction belongs in its own change if the estate wants one. |

### Still open

*None — every question above is answered. The RFC is ready for sign-off.*

### The phase-7 run

The claim in §3 — *"`terraform init` completes against a registry whose
`anonymous` grant is empty"* — measured against a live BatleHub rather than a
mock. Terraform 1.8.5, `hashicorp/null` 3.2.2 from the real
`registry.terraform.io`, through a TLS terminator (Terraform refuses a
plain-HTTP mirror, and BatleHub never terminates TLS itself), with
`signed_downloads = true` and `[registries.rbac] anonymous = []`.

`terraform init` exited `0`. The wire, as the terminator saw it:

```text
STATUS  AUTH   SIG       BYTES  PATH
200     yes    no          209  /proxy/tf/registry.terraform.io/hashicorp/null/index.json
200     yes    no         1606  /proxy/tf/registry.terraform.io/hashicorp/null/3.2.2.json
200     no     yes    5 057 172  /proxy/tf/v1/providers/hashicorp/null/3.2.2/artifact/linux/amd64
```

Five megabytes of provider archive, fetched with **no `Authorization` header**,
served `200` by a registry that grants anonymous nothing. The controls, same URL
on the same server:

| Request | Result |
| --- | --- |
| No signature, no header | `403` |
| `?bh_sig=1.AAAA.BBBB` | `403` |
| Header auth, no signature | `200` |

And the audit trail, which is §2's second motivation and §4.2's promise:

```text
alice   user       download  allowed  providers/hashicorp/null@3.2.2 linux/amd64
—       anonymous  download  denied   providers/hashicorp/null@3.2.2 linux/amd64
        reason: "role 'anonymous' is not permitted to perform 'releases:read' on this registry"
```

The same coordinate, twice, distinguished only by the signature — and the signed
one is attributed to `alice` rather than to nobody. That is the whole design in
two rows: the signature supplied an identity, and the rule chain then did its
own job with it.

The conformance table gains the signed request lines and one that was simply
missing — the mirror's `{version}.json`, request 2 of 3, which nothing had
recorded. What those entries pin is narrow: **a query string must not change
which route matches.** Nothing about `bh_sig` is special to the router, which is
why a `web::Query<T>` extractor added later would break signed downloads and
nothing else — for the one client that cannot fall back to a header.

### The O5 measurement

Terraform 1.8.5 against a mock registry serving the provider protocol over TLS
on two svchosts, **each holding its own credential** in the CLI config. Two
controls, because the obvious single-host run cannot tell "declined to send"
from "had nothing to send":

- `localhost:8443` — registry; token `TOKEN-REGISTRY-8443-…`
- `localhost:8444` — a second svchost (same process, same certificate, different
  port, and therefore a different svchost to Terraform); token
  `TOKEN-ARTIFACT-8444-…`

`terraform init` exited `0`, all three providers installed and GPG-verified.

```text
PORT  KIND      PATH                                            AUTH   TOKEN
8443  discovery /.well-known/terraform.json                      yes   REGISTRY-8443
8443  protocol  /v1/providers/acme/samehost/versions             yes   REGISTRY-8443
8443  protocol  /v1/providers/acme/crosshost/versions            yes   REGISTRY-8443
8444  discovery /.well-known/terraform.json                      yes   ARTIFACT-8444
8444  protocol  /v1/providers/acme/altreg/versions               yes   ARTIFACT-8444
8443  protocol  …/samehost/1.0.0/download/linux/amd64            yes   REGISTRY-8443
8443  artifact  /artifact/samehost.SHA256SUMS                     no   —
8443  artifact  /artifact/samehost.SHA256SUMS.sig                 no   —
8443  artifact  /artifact/samehost.zip                            no   —
8443  protocol  …/crosshost/1.0.0/download/linux/amd64           yes   REGISTRY-8443
8444  artifact  /artifact/crosshost.SHA256SUMS                    no   —
8444  artifact  /artifact/crosshost.SHA256SUMS.sig                no   —
8444  artifact  /artifact/crosshost.zip                           no   —
8444  protocol  …/altreg/1.0.0/download/linux/amd64              yes   ARTIFACT-8444
8444  artifact  /artifact/altreg.SHA256SUMS                       no   —
8444  artifact  /artifact/altreg.SHA256SUMS.sig                   no   —
8444  artifact  /artifact/altreg.zip                              no   —

9 artifact fetches, 0 authenticated.
```

Three things this settles that a reading of the docs would not have:

1. **The hole is the same one.** `samehost` is the O5 case exactly — the
   `download_url` is on the host authenticated one request earlier, and the
   fetch arrives bare. The registry protocol needs signed URLs for the same
   reason the mirror protocol does.
2. **It is three URLs per provider, not one.** `shasums_url` and
   `shasums_signature_url` are unauthenticated too. This is the measurement that
   turns **O3 from a preference into a requirement**: sign only the zip and a
   closed registry fails one step later, at the checksum, with an error that
   points at checksums rather than at auth — which is the failure mode O3
   predicted and this run confirms.
3. **It is a client policy, not a credential-resolution accident.** The
   `altreg` rows are the control: `:8444` receives its *own* token on discovery
   and on the protocol documents, so that credential is loaded and live — and
   its artifact fetches are still bare. Terraform does not authenticate package
   URLs, on any host, whether or not it holds a credential for it.

`/.well-known/terraform.json` **is** authenticated, on both hosts. Nothing in
this RFC depends on that, but it is the kind of detail that costs an afternoon
when assumed the other way.

The probe is a mock rather than a real BatleHub because the question is entirely
about client behaviour: what BatleHub returns in `download_url` is what this RFC
is choosing. A run against the real handlers belongs in phase 7, where it
verifies the implementation rather than the premise.

<!--
This section was a single table headed "Recommendation" until the RFC was read
for sign-off. Nothing in it changed then; it was split into the template's two
sections, which is the shape `docs/build/rfc-meta.mjs` reads. Until that split,
`task rfc:status` matched no `### Still open` heading, counted zero open
questions, and printed this document's readiness marker — reporting the one RFC
with an explicitly unmeasured prerequisite as the one with nothing left to
settle. The lesson is the same one §5 of this RFC makes about Terraform: a
document that does not use the shape a checker reads is not checked, and reads
as passing.
-->

O5 is the only thing between this document and sign-off, and it is a
measurement rather than an argument — a real Terraform run against a closed
registry, observing whether the `credentials` token accompanies the
same-host `download_url`. Phases 1–4 do not depend on the answer; phase 5 is
defined by it.

---

## 12. Implementation phases

Every phase landed. The third column is what it changed; the second is what it
was claimed it would land on its own, kept so the two can be compared.

| # | Phase | Lands even if the rest slips | What it changed |
| --- | --- | --- | --- |
| 1 | `signed_url.rs` in `core` — mint, verify, subkey derivation, the full unit suite; `[server.signed_urls]` in `config` with the startup validation of §4.1. | Yes — a tested primitive with no caller. | `crates/core/src/services/signed_url.rs` (new), `hmac` on the workspace, `[server.signed_urls]` + `signed_downloads` in `config`, six startup errors, two warning codes |
| 2 | Mint in `terraform_mirror_version`; `signed_downloads` per registry. | No effect until phase 3 verifies. | `HotConfig.signed_downloads` / `.signed_url`, `terraform/shared.rs` (`signer_for`, `sign_artifact_url`), minting in `terraform_mirror_version`, `server/hot_config.rs` |
| 3 | Verify on `terraform_provider_artifact`, ahead of the rule chain. | **Yes — and this is where the mirror protocol becomes complete.** A network-mirror install fetches `index.json`, `{version}.json` and the archive, and nothing else: it authenticates archives by the `hashes` in the document rather than by `SHA256SUMS`, so phases 2–3 alone let `terraform init` finish against a closed registry through that protocol. Measured in the §4.3 probe — three requests, no checksum fetch. | `identity_for_artifact` + `signed_identity`, verification on `terraform_provider_artifact` |
| 4 | The same for `shasums` and `shasums.sig`. | Nothing new on its own, and the original claim here — *"`terraform init` completes end to end"* — was written before O5 was measured and is wrong in both directions. The mirror protocol never asks for these (row 3); the registry protocol does, but no signature reaches them until phase 5 mints into the download document. This phase makes the two routes ready for that, and its tests mint what phase 5 will emit. | the same on `terraform_provider_shasums` and `..._shasums_sig` |
| 5 | **Extend to the registry protocol** — O5 is measured and the answer is yes. Mint in `terraform_provider_download` (`handlers/proxy/terraform/providers/read.rs:75`) for all three URLs it returns, and verify in `terraform_provider_artifact` (`:354`), `terraform_provider_shasums` (`:266`) and `terraform_provider_shasums_sig` (`:310`). Phases 3–4 already built the verification for the mirror protocol's routes; this is the same primitive on the registry protocol's four. | Yes — this is what makes a closed registry work for `required_providers`, which is how providers are actually declared. | `sign_download_document` + `DownloadCoords`, minting in `terraform_provider_download` **and** `try_local_provider_download`, a `PROVIDER_DOWNLOAD` fixture |
| 6 | Docs: rewrite the warning in `docs/registries/terraform.md` from "you must open the registry" to "you may, or you may sign"; the configuration reference; the log-hygiene audit of §7. | — | `docs/registries/terraform.md`, `docs/guide/configuration.md`, `config.example.toml`, and the §7.2 log-hygiene audit — which found the opposite of what §7 assumed |
| 7 | Real-client run and a conformance entry marked `observed`. | The claim becomes a measurement. | a live `terraform init` against a closed registry (§11), and five conformance entries incl. one the table never had |

---

## See also

- [RFC 0009 — Every endpoint the client actually calls](/rfc/0009-protocol-coverage) — §12.3 is the measurement this RFC starts from, and the reason it was deferred
- [RFC 0011 — Authenticated OpenVSX registry access](/rfc/0011-openvsx-login) — the same shape with a client that can be patched
- [Terraform registry](/registries/terraform) — the constraint as it is documented today
