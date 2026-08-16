# JetBrains Marketplace

Proxy and cache the JetBrains plugin ecosystem ([plugins.jetbrains.com](https://plugins.jetbrains.com)) — IDE search, compatible updates, `meta.json` blobs, and plugin downloads — or host private plugins. Everything is cached with stale fallback, so any plugin seen once keeps resolving even if upstream is unreachable. Distinct from the path-addressed [`jetbrains`](/registries/jetbrains) IDE-archive type.

## At a glance

| | |
|---|---|
| **Config type** | `jetbrains-marketplace` |
| **Default upstream** | `plugins.jetbrains.com` |
| **Modes** | proxy · local · hybrid |
| **Addressing** | per-package |
| **Private publish** | ✅ marketplace-compatible upload |

## Proxy setup

Point an IDE at the proxy. Replace `<registry>` with your configured registry name.

**Additive** — keep the public marketplace and add this registry's local plugins. Settings → Plugins → ⚙ → **Manage Plugin Repositories…** → add:

```text
https://batlehub.example.com/proxy/<registry>/updatePlugins.xml
```

(`updatePlugins.xml` lists locally published plugins; it returns 404 on a pure proxy-mode registry.)

**Full replacement** — the IDE talks only to BatleHub (search, updates, downloads). Help → **Edit Custom Properties…**, then add:

```properties
idea.plugins.host=https://batlehub.example.com/proxy/<registry>
```

Download a plugin directly:

```bash
curl -fL -o rust-plugin.zip \
  "https://batlehub.example.com/proxy/<registry>/plugin/download?pluginId=org.rust.lang&version=241.25026.107"

# Or let the proxy pick the newest version compatible with your IDE build:
curl -fL -o plugin.zip \
  "https://batlehub.example.com/proxy/<registry>/pluginManager?action=download&id=org.rust.lang&build=IU-241.14494"
```

## Publishing (local / hybrid)

`jetbrains-marketplace` registries in `local`/`hybrid` mode accept the same multipart upload as plugins.jetbrains.com, so both plain `curl` and JetBrains' own publishing tooling work.

### Server configuration

```toml
[[registries]]
type = "jetbrains-marketplace"
name = "internal-plugins"
mode = "local"

[registries.rbac]
anonymous = []
user      = ["releases:read"]
admin     = ["*"]
```

### Upload (curl)

The plugin id and version are read from `META-INF/plugin.xml` inside the archive (`.jar`, or `.zip` distribution with `lib/*.jar`). An `xmlId` form field, when present, must match the descriptor.

```sh
curl -X POST \
  -H "Authorization: Bearer <your-token>" \
  -F "xmlId=com.example.myplugin" \
  -F "channel=" \
  -F "file=@my-plugin-1.0.0.zip" \
  "https://batlehub.example.com/proxy/internal-plugins/api/updates/upload"
# → 201 {"id":"com.example.myplugin","pluginId":"com.example.myplugin","version":"1.0.0","channel":""}
```

Pass `-F "isHidden=true"` to publish a version hidden from listings (still downloadable by exact coordinate).

### Upload (Gradle / plugin-repository-rest-client)

Point the tooling's host at the proxy and use your BatleHub token:

```kotlin
// build.gradle.kts (org.jetbrains.intellij / intellij-platform plugin)
tasks.publishPlugin {
    host.set("https://batlehub.example.com/proxy/internal-plugins")
    token.set(providers.environmentVariable("BATLEHUB_TOKEN"))
}
```

### Install from the IDE

Settings → Plugins → ⚙ → **Manage Plugin Repositories…** → add
`https://batlehub.example.com/proxy/internal-plugins/updatePlugins.xml`.
For a full marketplace replacement instead, set `idea.plugins.host=https://batlehub.example.com/proxy/internal-plugins` in Help → Edit Custom Properties….

### Verify

```sh
# The custom-repo XML lists the published plugin
curl -s -H "Authorization: Bearer <your-token>" \
  "https://batlehub.example.com/proxy/internal-plugins/updatePlugins.xml"

# Download it back
curl -s -H "Authorization: Bearer <your-token>" \
  "https://batlehub.example.com/proxy/internal-plugins/plugin/download?pluginId=com.example.myplugin&version=1.0.0" \
  -o roundtrip.zip
```

### Endpoint reference

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/proxy/{registry}/api/updates/upload` | Publish a plugin archive (multipart) |
| `GET` | `/proxy/{registry}/updatePlugins.xml` | Custom-repository XML (local content) |
| `GET` | `/proxy/{registry}/plugins/list?pluginId={xmlId}` | Plugin-repository XML (all versions) |
| `GET` | `/proxy/{registry}/plugin/download?pluginId=&version=[&channel=]` | Download a plugin archive |
| `GET` | `/proxy/{registry}/pluginManager?action=download&id=&build=` | Newest build-compatible download |
| `GET` | `/proxy/{registry}/api/searchPlugins?search=` | Search (`{plugins, total}` shape) |
| `POST` | `/proxy/{registry}/api/search/updates/compatible` | Compatible updates for a build |
| `GET` | `/proxy/{registry}/files/{xmlId}/{version}/{file}` | Artifact passthrough |

---

## Blocked versions

All three plugin listings hide a blocked build: the `updatePlugins.xml`
custom-repository document an IDE polls, the classic `/plugins/list`, and
`/api/plugins/{id}/updates`. They are rendered from one version list, and the
filter sits on that list — so an IDE never offers a blocked build as an
available update and then fails to install it.

The upstream document is cached for the registry's `metadata_ttl`; blocks are
applied on top of the cached copy on every request, so blocking a version takes
effect immediately rather than when the cache expires.

See [blocking a package version](/guide/admin-policies#block-a-package-version) for the two halves of a block, and [which listings are filtered](/guide/admin-policies#which-listings-are-filtered) for the full table.

## Authentication

Pass a BatleHub token as a Bearer header on upload requests. Read access is governed by the registry's RBAC — anonymous access works only when the `anonymous` role is granted read.

## Notes

- Metadata, plugin artifacts, and the fixed JSON blobs the IDE fetches at startup are all cached with stale fallback.
- Do **not** point the path-addressed [`jetbrains`](/registries/jetbrains) IDE-archive type at plugins.jetbrains.com — use this type for the plugin ecosystem.

## See also

- [Using BatleHub](/use/) — tokens, publishing prerequisites, the CLI
- [Registries overview](/registries/) · [Caching](/guide/caching) · [Access Control](/guide/access-control)
