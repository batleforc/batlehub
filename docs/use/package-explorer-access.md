# Package Explorer — Access control

## Access control {#access-control}

### Proxy access vs. explore access {#access-separation}

By default, any user who can proxy from a registry can also browse it in the Explorer. You can restrict browsing independently of proxying using `[registries.rbac.explore]`.

This is useful when:

- You want CI/CD tokens to be able to download packages but not enumerate what's in the registry.
- You have a sensitive internal registry that should be accessible by tooling but not visible in the UI.

### Configuration {#rbac-config}

Add an `explore` block inside `[registries.rbac]`:

```toml
[[registries]]
name = "internal-cargo"
type = "cargo"
mode = "hybrid"
upstreams = ["https://index.crates.io"]

[registries.rbac]
user  = ["read"]    # regular users can proxy/download
admin = ["read"]    # admins can proxy/download

[registries.rbac.explore]
anonymous = false   # anonymous users cannot browse
user      = false   # regular users cannot browse (proxy-only)
admin     = true    # admins can browse
```

All three fields default to `true`, so omitting the `explore` block (or omitting individual fields) grants browse access to every role that already has proxy access.

### What "cannot browse" covers {#rbac-surface}

Every catalogue endpoint, not only the listing. A role denied explore on a registry gets:

- no rows for it in the package list and no counts in the registry stats;
- nothing from README search, whose scope is the same set;
- `404` on any of its package detail pages — including the upstream version list the page would otherwise have fetched on the caller's behalf;
- `404` on the README of any of its packages, and on any image inside that README.

Every one of those is the same `404` an absent package returns, so a denial does not confirm that the name exists.

The README matters here because it is the one catalogue answer that carries prose: it usually names the package's homepage, its dependencies and its build. A registry taken out of the Explorer is out of it by every door.

Proxying is untouched. `user = false` under `[registries.rbac.explore]` means *"this registry is for package managers, not for reading"* — a token that may `GET` an artifact still may, and it still resolves the registry's own protocol documents (an npm packument, a PyPI simple page), because those are how a package manager works rather than a browse.

### Inheritance {#rbac-inheritance}

Explore access is always capped by proxy access. A role that cannot proxy from a registry cannot explore it either, regardless of the `explore` flags:

```txt
effective explore access = proxy access AND explore permission
```

Group-level explore permissions are not separately configurable — group members inherit the explore access of their role (user or anonymous).
