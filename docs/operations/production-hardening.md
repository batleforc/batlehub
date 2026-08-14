# Production hardening checklist

Settings that are **deliberately permissive or off by default** and should be revisited before
a deployment is exposed to real traffic. Nothing here is a bug; each default is chosen so a
first run works with no configuration. Production is where you trade that convenience away.

For the availability side (replicas, shared cache backend, rolling updates) see
[High availability](/guide/high-availability). For the automated scanning that runs on the
codebase itself, see [Security scanning](/guide/security-scanning).

---

## 1. Reverse-proxy trust — `[server].trusted_proxies`

**Default: unset, meaning forwarded headers are believed from any peer.**

`Forwarded` / `X-Forwarded-Host` / `X-Forwarded-Proto` decide the host in every URL BatleHub
generates, and `X-Forwarded-For` decides the client IP that IP blocking counts violations
against. With no list configured the first three are trusted unconditionally and the fourth is
ignored entirely.

Set it to the CIDR of whatever terminates TLS in front of you:

```toml
[server]
trusted_proxies = ["10.42.0.0/16"]
```

Host-based routing makes this mandatory — the server refuses to start with `subdomain_routing`
or a registry `hosts` entry and no trust policy, because routing would otherwise be decided by
a header any client can set.

## 2. CORS — `[server].cors_allowed_origins`

**Default (since 1.1.0): same-origin only.** Nothing to do unless the UI is served from a
different origin than the API, in which case name that origin explicitly. Avoid `["*"]`; it
raises a `cors.any-origin` config warning precisely so it does not pass unnoticed.

## 3. Rate limiting — `[registries.rate_limit]`

**Default: off.** Configure per registry, and note the store: the in-memory token bucket is
per-process, so with more than one replica each gets its own full allowance. Use the Redis or
Postgres store for any multi-replica deployment.

```toml
[registries.rate_limit]
requests_per_window = 600
window_secs         = 60
enforcement         = "block"
```

## 4. IP blocking — `[ip_blocking]`

**Default: `enabled = false`.** Turn it on for internet-facing deployments. It depends on the
client IP being correct, so it is only meaningful once §1 is set — otherwise a client can
name whichever address it wants to have blocked.

## 5. `/metrics` is unauthenticated

By design, so a Prometheus scraper needs no credentials. It exposes registry names, package
cardinality and error counts. Restrict it at the ingress rather than publishing it:

```yaml
# nginx ingress: expose /metrics only to the monitoring namespace
nginx.ingress.kubernetes.io/server-snippet: |
  location /metrics { deny all; }
```

Scrape it in-cluster over the Service instead.

## 6. HSTS at the ingress

BatleHub sends `X-Content-Type-Options`, `X-Frame-Options` and `Referrer-Policy` on every
response, but deliberately **not** `Strict-Transport-Security` — TLS terminates at the ingress
and the server itself usually speaks plaintext HTTP, so emitting HSTS from behind the proxy
risks pinning browsers to `https://` for a host that cannot serve it. Set it where TLS ends:

```yaml
ingress:
  annotations:
    nginx.ingress.kubernetes.io/configuration-snippet: |
      more_set_headers "Strict-Transport-Security: max-age=31536000; includeSubDomains";
```

## 7. Secrets

No credential should be a literal in `values.yaml` or `config.toml`. Use `${VAR}` placeholders
expanded from env vars sourced from a Secret — BatleHub exits at startup naming any placeholder
whose variable is missing, so a typo fails loudly rather than silently authenticating as
nobody.

Before going live, grep the rendered config for the placeholders that ship in the examples:
`change-me-admin-token`, `change-me-user-token`, `proxy-auth-secret`,
`batlehub-local-insecure-secret-key`. They exist so the example files run locally; none should
ever reach a cluster.

## 8. Storage backend vs. replica count

`persistence.accessMode: ReadWriteOnce` cannot be shared by pods on different nodes, so
filesystem storage is effectively single-replica. For `replicaCount > 1` use S3 storage — the
only backend all replicas can write to concurrently — or a `ReadWriteMany` storage class.

## 9. Pod security

The chart defaults are already restrictive as of 1.1.0 (`runAsNonRoot`, uid 65532,
`readOnlyRootFilesystem`, all capabilities dropped, `RuntimeDefault` seccomp) and the pod is
admissible in a Pod Security Admission `restricted` namespace unmodified. If you override
`podSecurityContext` or `securityContext`, you are replacing the whole block — re-state the
fields you still want.

`networkPolicy` is off by default because the correct egress set depends on which upstream
registries you proxy. When you enable it, the chart always emits a DNS egress rule first; add
your upstreams under `networkPolicy.egressTo`.

---

## Quick audit

```bash
# Rendered chart: confirm the defaults you expect actually made it in
helm template batlehub ./helm/batlehub -f prod-values.yaml \
  | grep -E 'runAsNonRoot|readOnlyRootFilesystem|allowPrivilegeEscalation|path: /(livez|healthz)'

# Running server: config warnings name anything left permissive
curl -sH "Authorization: Bearer $ADMIN_TOKEN" \
  https://batlehub.example.com/api/v1/admin/config/warnings | jq
```

`GET /api/v1/admin/config/warnings` is the fastest check, but it reports **non-fatal** problems
only — the server has to be running to answer it. Each warning carries a stable `code` and the
`path` of the offending key. A wildcard CORS origin (`cors.any-origin`) surfaces there, as does
an unstated proxy-trust policy (`proxy-trust.unconfigured`) on a deployment without host-based
routing.

Hard validation failures never reach that endpoint. Combining host-based routing with no
`trusted_proxies` is one: routing would be decided by a header with no policy about who may set
it, so `AppConfig::validate` refuses to start and prints the reason. Read the startup log for
those.
