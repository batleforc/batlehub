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

The registry must be in `local` or `hybrid` mode. Create a ZIP with a `composer.json` at its root (or inside a single top-level directory, GitHub-archive style) and POST it:

```sh
zip -r my-vendor-my-pkg-1.0.0.zip my-vendor-my-pkg-1.0.0/

curl -X POST \
  -H "Authorization: Bearer $BATLEHUB_TOKEN" \
  -H "Content-Type: application/zip" \
  --data-binary @my-vendor-my-pkg-1.0.0.zip \
  "https://batlehub.example.com/proxy/<registry>/api/upload"
```

The `name` field must use the `vendor/package` format; the `version` field sets the published version (overridable with `?version=` on the upload URL). Yank a version with `DELETE …/api/packages/{vendor}/{pkg}/versions/{version}`.

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

- `composer audit` works automatically — BatleHub proxies the Packagist security advisory API (`/api/security-advisories/`) transparently. See [User Guide → Security audit](/guide/user#registries).
- Yanked versions are hidden from version listings and return `404` on download.

## See also

- [User Guide → Composer (PHP)](/guide/user#registries)
- [Registries overview](/registries/) · [Caching](/guide/caching) · [Access Control](/guide/access-control)
