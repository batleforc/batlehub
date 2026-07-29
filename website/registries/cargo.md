# Cargo

Proxy and cache crates.io, or host private crates. BatleHub implements the Cargo [sparse registry protocol](https://doc.rust-lang.org/cargo/reference/registry-protocols.html#sparse-protocol), serving the index and `.crate` downloads through the cache. Index checksums match the cached `.crate` files, so verification keeps working.

## At a glance

| | |
|---|---|
| **Config type** | `cargo` |
| **Default upstream** | `crates.io` |
| **Modes** | proxy · local · hybrid |
| **Addressing** | per-package |
| **Private publish** | ✅ `cargo publish` |

## Proxy setup

Replace the default crates.io source so all `cargo add` / `cargo build` requests go through BatleHub:

```toml
# .cargo/config.toml (project) or ~/.cargo/config.toml (global)
[source.crates-io]
replace-with = "batlehub"

[source.batlehub]
registry = "sparse+https://batlehub.example.com/proxy/<registry>/registry/"
```

The `index` / `registry` URL uses the `sparse+` prefix and must end with `/registry/`.

## Publishing (local / hybrid)

The registry must be in `local` or `hybrid` mode. Declare it as a named registry:

```toml
# .cargo/config.toml
[registries.internal]
index = "sparse+https://batlehub.example.com/proxy/<registry>/registry/"
token = "<your-token>"
```

Then publish (and yank) against that registry:

```bash
cargo publish --registry internal
cargo yank --registry internal my-lib@0.1.0
cargo yank --undo --registry internal my-lib@0.1.0
```

Depend on a privately published crate with `my-lib = { version = "0.1", registry = "internal" }`.

## Authentication

Cargo sends the `token` from the `[registries.<name>]` block. In CI, set it via the environment instead: `export CARGO_REGISTRIES_INTERNAL_TOKEN=$BATLEHUB_TOKEN` (uppercase the registry name).

## Notes

If `cargo publish` fails with "invalid token", verify the `index` URL ends with `/registry/`. Checksums returned by the sparse index match the cached `.crate` files, so `cargo verify-project` continues to work.

## See also

- [User Guide → Cargo](/guide/user#registries)
- [Registries overview](/registries/) · [Caching](/guide/caching) · [Access Control](/guide/access-control)
