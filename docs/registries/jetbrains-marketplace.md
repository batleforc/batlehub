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

The registry must be in `local` or `hybrid` mode. The upload endpoint is marketplace-compatible: plain `curl`, JetBrains' `plugin-repository-rest-client`, and the Gradle `publishPlugin` task all work with a BatleHub Bearer token. The plugin id/version are read from `META-INF/plugin.xml` inside the archive:

```bash
curl -X POST -H "Authorization: Bearer $BATLEHUB_TOKEN" \
  -F "xmlId=com.example.myplugin" \
  -F "file=@my-plugin-1.0.0.zip" \
  "https://batlehub.example.com/proxy/<registry>/api/updates/upload"
```

```kotlin
// Gradle (intellij-platform plugin)
tasks.publishPlugin {
    host.set("https://batlehub.example.com/proxy/<registry>")
    token.set(providers.environmentVariable("BATLEHUB_TOKEN"))
}
```

Publish with `-F "isHidden=true"` to keep a version out of every listing while remaining downloadable by exact coordinate.

## Authentication

Pass a BatleHub token as a Bearer header on upload requests. Read access is governed by the registry's RBAC — anonymous access works only when the `anonymous` role is granted read.

## Notes

- Metadata, plugin artifacts, and the fixed JSON blobs the IDE fetches at startup are all cached with stale fallback.
- Do **not** point the path-addressed [`jetbrains`](/registries/jetbrains) IDE-archive type at plugins.jetbrains.com — use this type for the plugin ecosystem.

## See also

- [User Guide → JetBrains Marketplace](/guide/user#registries)
- [Registries overview](/registries/) · [Caching](/guide/caching) · [Access Control](/guide/access-control)
