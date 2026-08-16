# Composer (PHP)

Proxy and cache Packagist for PHP Composer, or host private packages. BatleHub implements the Packagist v2 protocol (`packages.json` + `p2/` metadata endpoints), so Composer treats it as a native Composer repository — gated by RBAC and the release-age gate.

## At a glance

| | |
|---|---|
| **Config type** | `composer` |
| **Default upstream** | `repo.packagist.org` |
| **Modes** | proxy · local · hybrid |
| **Addressing** | per-package |
| **Private publish** | ✅ `curl -X POST …/api/upload` |

## Proxy setup

Add a repository entry to `composer.json`. Replace `<registry>` with your configured registry name:

```json
{
  "repositories": [
    {
      "type": "composer",
      "url": "https://batlehub.example.com/proxy/<registry>/"
    }
  ]
}
```

Install as usual:

```sh
composer install
composer require symfony/console
```

## Publishing (local / hybrid)

Composer packages are uploaded as ZIP archives containing a `composer.json`. BatleHub reads `name` (format `vendor/package`) and `version` from `composer.json` when a package is uploaded, so no separate metadata step is required.

### Server configuration

```toml
[[registries]]
type = "composer"
name = "internal-composer"
mode = "local"          # or "hybrid" to fall back to repo.packagist.org

[registries.rbac]
anonymous = []
user      = ["source:read"]
admin     = ["*"]
```

For hybrid mode add `upstreams = ["https://repo.packagist.org"]`.

### Package format

A Composer package is a ZIP archive with a `composer.json` at the archive root (or inside a single top-level subdirectory — standard practice when archiving a git checkout). The `composer.json` must include `name` and `version`:

```json
{
  "name": "my-vendor/my-package",
  "version": "1.0.0",
  "description": "My private library",
  "autoload": {
    "psr-4": { "MyVendor\\MyPackage\\": "src/" }
  }
}
```

Build the archive from your project directory:

```sh
# Archive from the current directory (top-level files directly in ZIP)
zip -r my-vendor-my-package-1.0.0.zip . -x "*.git*" -x "vendor/*"

# Or use git archive for a clean export
git archive --format=zip HEAD -o my-vendor-my-package-1.0.0.zip
```

If your `composer.json` has no `version` field (common in version-controlled projects), pass it as a query parameter when uploading.

### Upload

```sh
# composer.json contains a "version" field
curl -X POST \
  -H "Authorization: Bearer <your-token>" \
  -H "Content-Type: application/zip" \
  --data-binary @my-vendor-my-package-1.0.0.zip \
  "https://batlehub.example.com/proxy/internal-composer/api/upload"

# Override (or supply) the version via query parameter
curl -X POST \
  -H "Authorization: Bearer <your-token>" \
  -H "Content-Type: application/zip" \
  --data-binary @my-vendor-my-package.zip \
  "https://batlehub.example.com/proxy/internal-composer/api/upload?version=1.0.0"
```

### Client setup

Composer supports two ways to supply credentials. Prefer `auth.json` over inline headers so credentials stay out of source control.

**`auth.json`** (place in the project root or `~/.composer/auth.json` for global use):

```json
{
  "http-basic": {
    "batlehub.example.com": {
      "username": "token",
      "password": "<your-token>"
    }
  }
}
```

Composer sends this as `Authorization: Basic base64("token:<your-token>")`. BatleHub extracts the password field and matches it against your configured token.

**Inline header in `composer.json`** (alternative when `auth.json` is not an option):

```json
{
  "repositories": [
    {
      "type": "composer",
      "url": "https://batlehub.example.com/proxy/internal-composer/",
      "options": {
        "http": {
          "header": ["Authorization: Bearer <your-token>"]
        }
      }
    }
  ]
}
```

### Install

With credentials configured, add the repository to `composer.json` and require the package:

```json
{
  "repositories": [
    {
      "type": "composer",
      "url": "https://batlehub.example.com/proxy/internal-composer/"
    }
  ],
  "require": {
    "my-vendor/my-package": "^1.0"
  }
}
```

```sh
composer install
# or
composer require my-vendor/my-package
```

### Yank a version

```sh
curl -X DELETE \
  -H "Authorization: Bearer <your-token>" \
  "https://batlehub.example.com/proxy/internal-composer/api/packages/my-vendor/my-package/versions/1.0.0"
```

### Endpoint reference

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/proxy/{registry}/api/upload[?version=X.Y.Z]` | Upload package ZIP |
| `DELETE` | `/proxy/{registry}/api/packages/{vendor}/{package}/versions/{version}` | Yank version |
| `GET` | `/proxy/{registry}/packages.json` | Packagist v1 root |
| `GET` | `/proxy/{registry}/p2/{vendor}/{package}.json` | Packagist v2 metadata |
| `GET` | `/proxy/{registry}/dist/{vendor}/{package}/{version}` | Download artifact |

---

## Authentication

Store HTTP Basic credentials in `auth.json` (project root or `~/.config/composer/` — never commit it):

```json
{
  "http-basic": {
    "batlehub.example.com": {
      "username": "user",
      "password": "<your-token>"
    }
  }
}
```

When `auth.json` is present, no `Authorization` header is needed in `composer.json`.

## Notes

- `composer audit` works automatically — BatleHub proxies the Packagist security advisory API (`/api/security-advisories/`) transparently. See [Using BatleHub → security auditing](/use/#security-audit).
- Yanked versions are hidden from version listings and return `404` on download.

## See also

- [Using BatleHub](/use/) — tokens, publishing prerequisites, the CLI
- [Registries overview](/registries/) · [Caching](/guide/caching) · [Access Control](/guide/access-control)
