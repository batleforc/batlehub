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

The registry must be in `local` or `hybrid` mode.

```sh
GEM_HOST_API_KEY="Bearer $BATLEHUB_TOKEN" \
  gem push my-gem-1.0.0.gem --host https://batlehub.example.com/proxy/<registry>/
```

Yank or unyank a version:

```sh
curl -X DELETE -H "Authorization: Bearer $BATLEHUB_TOKEN" \
  "https://batlehub.example.com/proxy/<registry>/api/v1/gems/yank?gem_name=my-gem&version=1.0.0"

curl -X PUT -H "Authorization: Bearer $BATLEHUB_TOKEN" \
  "https://batlehub.example.com/proxy/<registry>/api/v1/gems/unyank?gem_name=my-gem&version=1.0.0"
```

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

- [User Guide → RubyGems](/guide/user#registries)
- [Registries overview](/registries/) · [Caching](/guide/caching) · [Access Control](/guide/access-control)
