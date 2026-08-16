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

### Inheritance {#rbac-inheritance}

Explore access is always capped by proxy access. A role that cannot proxy from a registry cannot explore it either, regardless of the `explore` flags:

```txt
effective explore access = proxy access AND explore permission
```

Group-level explore permissions are not separately configurable — group members inherit the explore access of their role (user or anonymous).
