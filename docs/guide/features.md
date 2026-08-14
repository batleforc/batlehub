# Features

Everything the server does, in one list. The home page shows three of these
because a visitor deciding whether to install needs an introduction; this is the
page for the reader who has decided and wants the specification.

Each entry says what the feature is. Where it is configured, the reference is
[Configuration](/guide/configuration).

## Caching and storage

**Artifact caching.** First download is fetched from upstream and stored locally
or in S3. Every subsequent request is served from cache — fast and
bandwidth-free. See [Caching](/guide/caching).

**Cache warming and eviction.** Pre-fetch packages at startup to eliminate
cold-start latency. Evict by TTL, idle time, version count, or storage size cap —
mix and match per registry.

**Storage deduplication.** Identical artifact bytes are stored once, regardless
of how many registries or package names reference them. Ref-counted and
backwards-compatible.

**Multi-upstream fanout.** List multiple upstreams per registry. A 404 from one
automatically falls through to the next — no single point of failure.

## Publishing

**Private registries.** Publish private npm packages, Cargo crates, Go modules,
VS Code extensions, Python wheels, conda packages, NuGet packages, and more
directly to BatleHub. Use local or hybrid mode per registry. See
[Publishing](/use/publishing) and the [Registries reference](/registries/).

**Beta / pre-release channel.** Gate pre-release versions (e.g. `1.0.0-beta.1`)
to approved users or groups. Non-members see only stable versions — no separate
publish step needed. See [Access Control](/guide/access-control).

## Access control and identity

**Role-based access control.** Per-registry permissions for anonymous, user, and
admin roles. Group-based access from OIDC claims, Kubernetes service accounts,
or GitHub/Forgejo Actions OIDC tokens. See [Access Control](/guide/access-control).

**Actions OIDC auth.** Validate GitHub and Forgejo workflow JWTs without
long-lived secrets. Map any claim — repo, branch, environment — to groups and
roles via glob/regex rules. Dynamic group names like
`{name}/{repository}/{ref_name}` enable wildcard RBAC grants across all CI jobs.

**Hashed static tokens.** Store Argon2id PHC hashes in config instead of raw
token strings. Run `batlehub hash-token <value>` to generate a hash. Plain-text
tokens keep working — both formats coexist.

## Protecting the instance

**Release age gate.** Block packages published less than N seconds ago. Creates
a delay window against supply-chain attacks without blocking known-good
versions.

**Distributed rate limiting.** Fixed-window per-user and per-group rate limits.
Back counters with InMemory, PostgreSQL, or Redis — shared limits survive
restarts and scale across replicas.

**IP-based blocking.** Fail2ban-style auto-blocking. IPs that exceed a violation
threshold (rate-limit hits, auth failures) are blocked automatically. Manual
ban/unban via admin API.

## Running it

**OpenTelemetry.** Optional distributed tracing via OTLP/gRPC. Works out of the
box with Jaeger, Tempo, or any OTLP-compatible backend.

**High availability.** Multiple replicas behind one address, with shared cache
and rate-limit state. See [High Availability](/guide/high-availability).

**SBOMs and vulnerability data.** Generate and serve SBOMs per artifact, and
proxy the advisory databases your tools already query. See [SBOM](/guide/sbom)
and [Vulnerability proxy](/use/vulnerability-proxy).
