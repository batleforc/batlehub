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

### Server configuration

```toml
[[registries]]
type = "cargo"
name = "internal"
mode = "local"          # or "hybrid" to fall back to crates.io

[registries.rbac]
anonymous = []
user      = ["source:read"]
admin     = ["*"]
```

For hybrid mode add:
```toml
upstreams = ["https://static.crates.io/crates"]
index_url = "https://index.crates.io"
```

### Client setup

Edit `~/.cargo/config.toml` or `.cargo/config.toml` in the project root:

```toml
[registries.internal]
index = "sparse+https://batlehub.example.com/proxy/internal/registry/"
token = "<your-token>"
```

Alternatively export the token as an environment variable (useful in CI):

```sh
export CARGO_REGISTRIES_INTERNAL_TOKEN=<your-token>
```

### Publish

```sh
cargo publish --registry internal
```

Cargo serialises crate metadata + the `.crate` archive into a single binary payload and sends it to `PUT /proxy/internal/api/v1/crates/new`. The checksum is verified server-side.

### Depend on a privately published crate

```toml
# Cargo.toml
[dependencies]
my-lib = { version = "0.1", registry = "internal" }
```

### Yank / unyank a version

```sh
cargo yank --registry internal my-lib@0.1.0
cargo yank --undo --registry internal my-lib@0.1.0
```

### Verify

```sh
cargo add my-lib --registry internal
```

### Endpoint reference

| Method | Path | Description |
|--------|------|-------------|
| `PUT` | `/proxy/{registry}/api/v1/crates/new` | `cargo publish` |
| `DELETE` | `/proxy/{registry}/api/v1/crates/{name}/{version}/yank` | `cargo yank` |
| `PUT` | `/proxy/{registry}/api/v1/crates/{name}/{version}/unyank` | `cargo yank --undo` |
| `GET` | `/proxy/{registry}/registry/config.json` | Sparse index config |
| `GET` | `/proxy/{registry}/registry/{path}` | Sparse index entries |
| `GET` | `/proxy/{registry}/{name}/{version}/download` | `.crate` download |

---

## Authentication

Cargo sends the `token` from the `[registries.<name>]` block. In CI, set it via the environment instead: `export CARGO_REGISTRIES_INTERNAL_TOKEN=$BATLEHUB_TOKEN` (uppercase the registry name).

## Notes

If `cargo publish` fails with "invalid token", verify the `index` URL ends with `/registry/`. Checksums returned by the sparse index match the cached `.crate` files, so `cargo verify-project` continues to work.

## See also

- [Using BatleHub](/use/) — tokens, publishing prerequisites, the CLI
- [Registries overview](/registries/) · [Caching](/guide/caching) · [Access Control](/guide/access-control)
