# RubyGems

Proxy and cache rubygems.org for Bundler and the `gem` CLI, or host private gems. BatleHub serves gem downloads, the version index, and the REST info API, gated by RBAC and the release-age gate.

## At a glance

| | |
|---|---|
| **Config type** | `rubygems` |
| **Default upstream** | `rubygems.org` |
| **Modes** | proxy · local · hybrid |
| **Addressing** | per-package |
| **Private publish** | ✅ `gem push` |

## Proxy setup

Install from your registry. Replace `<registry>` with your configured registry name:

```sh
gem install rake --source https://batlehub.example.com/proxy/<registry>/
```

Or in a `Gemfile`:

```ruby
source "https://batlehub.example.com/proxy/<registry>" do
  gem "rake"
end
```

## Publishing (local / hybrid)

### Server configuration

```toml
[[registries]]
type = "rubygems"
name = "internal-gems"
mode = "local"          # or "hybrid" to fall back to rubygems.org

[registries.rbac]
anonymous = []
user      = ["source:read"]
admin     = ["*"]
```

For hybrid mode add `upstreams = ["https://rubygems.org"]`.

### Client setup

**Option A — environment variable (recommended for CI):**

```sh
export GEM_HOST_API_KEY="Bearer <your-token>"
```

gem sends the value of `GEM_HOST_API_KEY` verbatim as the `Authorization` header, so the `Bearer ` prefix is required.

**Option B — `~/.gem/credentials` (create if absent, `chmod 600` after):**

```yaml
---
:batlehub: "Bearer <your-token>"
```

The symbol (`:batlehub:`) is an arbitrary name you choose. The value must include the `Bearer ` prefix because gem sends it verbatim as the `Authorization` header. Reference the entry by name with `--key` when pushing.

### Publish

```sh
# Using GEM_HOST_API_KEY (no --key needed)
GEM_HOST_API_KEY="Bearer <your-token>" \
  gem push my-gem-1.0.0.gem --host https://batlehub.example.com/proxy/internal-gems/

# Using ~/.gem/credentials with a named key
gem push my-gem-1.0.0.gem \
  --host https://batlehub.example.com/proxy/internal-gems/ \
  --key batlehub
```

### Install

```sh
# Using GEM_HOST_API_KEY
GEM_HOST_API_KEY="Bearer <your-token>" \
  gem install my-gem --source https://batlehub.example.com/proxy/internal-gems/

# Using a named credentials key
gem install my-gem \
  --source https://batlehub.example.com/proxy/internal-gems/ \
  --key batlehub
```

Or in a `Gemfile`:

```ruby
source "https://batlehub.example.com/proxy/internal-gems" do
  gem "my-gem"
end
```

### Yank / unyank

```sh
# Yank
curl -X DELETE \
  -H "Authorization: Bearer <your-token>" \
  "https://batlehub.example.com/proxy/internal-gems/api/v1/gems/yank?gem_name=my-gem&version=1.0.0"

# Unyank
curl -X PUT \
  -H "Authorization: Bearer <your-token>" \
  "https://batlehub.example.com/proxy/internal-gems/api/v1/gems/unyank?gem_name=my-gem&version=1.0.0"
```

### Endpoint reference

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/proxy/{registry}/api/v1/gems` | `gem push` |
| `DELETE` | `/proxy/{registry}/api/v1/gems/yank` | Yank version |
| `PUT` | `/proxy/{registry}/api/v1/gems/unyank` | Unyank version |
| `GET` | `/proxy/{registry}/gems/{name}-{version}.gem` | Download gem |
| `GET` | `/proxy/{registry}/api/v1/gems/{name}.json` | Gem info |
| `GET` | `/proxy/{registry}/api/v1/versions/{name}.json` | All versions |

---

## Blocked versions

`/api/v1/versions/{name}.json` drops the blocked entry.

`/api/v1/gems/{name}.json` describes the gem at exactly one version, so it is
**rebuilt** around the newest version that is still allowed. The gem-level
fields survive; the fields that describe the hidden *release* — its checksum,
its download URL, its own dates — are removed, because carrying a checksum onto
a different version would hand a client a hash that can never match.

The Marshal indexes (`specs.4.8.gz`, `quick/Marshal.4.8/*`) are **not** filtered.
Hiding a version from them would need a Ruby Marshal encoder, to hide what the
JSON APIs above already hide for every client released this decade.

The upstream document is cached for the registry's `metadata_ttl`; blocks are
applied on top of the cached copy on every request, so blocking a version takes
effect immediately rather than when the cache expires.

See [blocking a package version](/guide/admin-policies#block-a-package-version) for the two halves of a block, and [which listings are filtered](/guide/admin-policies#which-listings-are-filtered) for the full table.

## Authentication

`gem` sends `GEM_HOST_API_KEY` verbatim as the `Authorization` header, so the `Bearer ` prefix is required:

```sh
export GEM_HOST_API_KEY="Bearer $BATLEHUB_TOKEN"
```

Alternatively, store it in `~/.gem/credentials` (`chmod 600`) under a key name and reference it with `--key`:

```yaml
---
:batlehub: "Bearer <your-token>"
```

## Notes

- Gems are cached after the first download.
- To mirror rubygems.org transparently for an existing `Gemfile`, use `bundle config set mirror.https://rubygems.org/ …` instead of editing the `source`.

## See also

- [Using BatleHub](/use/) — tokens, publishing prerequisites, the CLI
- [Registries overview](/registries/) · [Caching](/guide/caching) · [Access Control](/guide/access-control)
