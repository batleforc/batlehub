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

Both registry types (`openvsx` and `vscode-marketplace`) use the same upload endpoint. There is no dedicated CLI tool — extensions are published with a plain `PUT` request carrying the raw VSIX bytes.

### Server configuration

```toml
[[registries]]
type = "openvsx"        # or "vscode-marketplace"
name = "internal-ext"
mode = "local"

[registries.rbac]
anonymous = []
user      = ["source:read"]
admin     = ["*"]
```

### Extension ID convention

Extension IDs follow the `{publisher}.{name}` format used by the VS Code Marketplace, e.g. `my-org.my-extension`.

### Upload

```sh
curl -X PUT \
  -H "Authorization: Bearer <your-token>" \
  -H "Content-Type: application/octet-stream" \
  --data-binary @my-org.my-extension-1.0.0.vsix \
  "https://batlehub.example.com/proxy/internal-ext/my-org.my-extension/1.0.0/vsix"
```

The server reads the publisher and extension name from the URL path. The `{extension_id}` segment is the full `{publisher}.{name}` identifier.

### Download / install

```sh
# Download the VSIX
curl -H "Authorization: Bearer <your-token>" \
  "https://batlehub.example.com/proxy/internal-ext/my-org.my-extension/1.0.0/vsix" \
  -o my-org.my-extension-1.0.0.vsix

# Install into VS Code
code --install-extension my-org.my-extension-1.0.0.vsix
```

### Verify

```sh
# Confirm the ZIP magic bytes (PK\x03\x04) to validate the upload was accepted
curl -s -H "Authorization: Bearer <your-token>" \
  "https://batlehub.example.com/proxy/internal-ext/my-org.my-extension/1.0.0/vsix" \
  | xxd | head -1
# Should show: 50 4b 03 04 ...
```

### Endpoint reference

| Method | Path | Description |
|--------|------|-------------|
| `PUT` | `/proxy/{registry}/{extension_id}/{version}/vsix` | Upload VSIX |
| `GET` | `/proxy/{registry}/{extension_id}/{version}/vsix` | Download VSIX |

---

## Authentication

Pass a BatleHub token as a Bearer header on the VSIX request. Anonymous access works only when the registry's RBAC grants the `anonymous` role read access.

## Notes

- The direct VSIX route is `…/proxy/<registry>/{publisher}.{name}/{version}/vsix`.
- Full VS Code gallery protocol (`/vscode/gallery` for VSCodium `product.json`) is not implemented — only VSIX proxying is supported today.
- For extensions published only to Microsoft's marketplace and not mirrored on open-vsx.org, use the [VS Code Marketplace](/registries/vscode-marketplace) type instead.

## See also

- [Using BatleHub](/use/) — tokens, publishing prerequisites, the CLI
- [Registries overview](/registries/) · [Caching](/guide/caching) · [Access Control](/guide/access-control)
