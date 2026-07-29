# Terraform

Proxy and cache the Terraform provider and module registry protocol (v1 API), or host private modules and providers. BatleHub serves provider version listings, provider download info, module version listings, and module source downloads, gated by RBAC and the release-age gate.

## At a glance

| | |
|---|---|
| **Config type** | `terraform` |
| **Default upstream** | `registry.terraform.io` |
| **Modes** | proxy · local · hybrid |
| **Addressing** | per-package |
| **Private publish** | ✅ module + provider upload |

## Proxy setup

Configure a provider network mirror and per-host credentials in `~/.terraformrc` (or `%APPDATA%/terraform.rc` on Windows). Replace `<registry>` with your configured registry name:

```hcl
# ~/.terraformrc
provider_installation {
  network_mirror {
    url = "https://batlehub.example.com/proxy/<registry>/"
  }
}

credentials "batlehub.example.com" {
  token = "<your-token>"
}
```

## Publishing (local / hybrid)

The registry must be in `local` or `hybrid` mode.

**Modules** are a `.tar.gz` of the module directory:

```sh
tar -czf consul-aws-0.1.0.tar.gz -C /path/to/module .

curl -X POST \
  -H "Authorization: Bearer $BATLEHUB_TOKEN" \
  -H "Content-Type: application/gzip" \
  --data-binary @consul-aws-0.1.0.tar.gz \
  "https://batlehub.example.com/proxy/<registry>/v1/modules/hashicorp/consul/aws/0.1.0"
```

**Providers** are a two-step upload — a version manifest, then each platform binary:

```sh
# 1. Version manifest (platforms + checksums)
curl -X POST \
  -H "Authorization: Bearer $BATLEHUB_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{ "version": "1.0.0", "protocols": ["5.0"], "platforms": [
        { "os": "linux", "arch": "amd64",
          "filename": "terraform-provider-mycloud_1.0.0_linux_amd64.zip",
          "shasum": "<sha256-hex>" } ] }' \
  "https://batlehub.example.com/proxy/<registry>/v1/providers/myorg/mycloud/versions"

# 2. Platform binary (repeat per platform)
curl -X PUT \
  -H "Authorization: Bearer $BATLEHUB_TOKEN" \
  -H "Content-Type: application/zip" \
  --data-binary @terraform-provider-mycloud_1.0.0_linux_amd64.zip \
  "https://batlehub.example.com/proxy/<registry>/v1/providers/myorg/mycloud/1.0.0/artifact/linux/amd64"
```

Reference them by the `batlehub.example.com/proxy/<registry>/…` source address in your Terraform config.

## Authentication

Terraform reads per-host credentials from the `credentials "batlehub.example.com"` block in `~/.terraformrc` (shown above) and sends the token as a Bearer header.

## Notes

- Providers are cached after first download in proxy/hybrid mode, or served entirely from local storage in local mode.
- The module upload response includes an `X-Terraform-Get` header pointing at the artifact download URL.

## See also

- [User Guide → Terraform](/guide/user#registries)
- [Registries overview](/registries/) · [Caching](/guide/caching) · [Access Control](/guide/access-control)
