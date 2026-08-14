# OpenVSX

Proxy and cache VS Code extension VSIX downloads from [open-vsx.org](https://open-vsx.org), or host private extensions. Extension IDs follow the `{publisher}.{name}` convention, and a direct VSIX route lets you fetch any version by coordinate.

## At a glance

| | |
|---|---|
| **Config type** | `openvsx` |
| **Default upstream** | `open-vsx.org` |
| **Modes** | proxy · local · hybrid |
| **Addressing** | per-package |
| **Private publish** | ✅ VSIX upload (`PUT …/vsix`) |

## Proxy setup

Point VS Code at BatleHub by adding to `.vscode/settings.json` or user settings:

```json
{
  "vscode-extension-marketplace.serviceUrl": "https://batlehub.example.com/proxy/<registry>/openvsx"
}
```

Or download a VSIX directly by coordinate and install it. Replace `<registry>` with your configured registry name:

```sh
curl -H "Authorization: Bearer $BATLEHUB_TOKEN" \
  "https://batlehub.example.com/proxy/<registry>/ms-python.python/2024.2.1/vsix" \
  -o ms-python.python-2024.2.1.vsix

code --install-extension ms-python.python-2024.2.1.vsix
```

## Publishing (local / hybrid)

The registry must be in `local` or `hybrid` mode. Both `openvsx` and `vscode-marketplace` types share the same upload endpoint; extension IDs follow `{publisher}.{name}`:

```sh
curl -X PUT \
  -H "Authorization: Bearer $BATLEHUB_TOKEN" \
  -H "Content-Type: application/octet-stream" \
  --data-binary @my-org.my-extension-1.0.0.vsix \
  "https://batlehub.example.com/proxy/<registry>/my-org.my-extension/1.0.0/vsix"
```

Download it back the same way (`GET …/{publisher}.{name}/{version}/vsix`), then `code --install-extension`.

## Authentication

Pass a BatleHub token as a Bearer header on the VSIX request. Anonymous access works only when the registry's RBAC grants the `anonymous` role read access.

## Notes

- The direct VSIX route is `…/proxy/<registry>/{publisher}.{name}/{version}/vsix`.
- Full VS Code gallery protocol (`/vscode/gallery` for VSCodium `product.json`) is not implemented — only VSIX proxying is supported today.
- For extensions published only to Microsoft's marketplace and not mirrored on open-vsx.org, use the [VS Code Marketplace](/registries/vscode-marketplace) type instead.

## See also

- [User Guide → VS Code Extensions](/guide/user#registries)
- [Registries overview](/registries/) · [Caching](/guide/caching) · [Access Control](/guide/access-control)
