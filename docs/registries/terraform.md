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

BatleHub supports both **provider** and **module** private registries. Modules use a simple tarball upload. Providers follow a two-step process: upload a version manifest (JSON describing platforms and checksums), then upload each platform binary.

### Server configuration

```toml
[[registries]]
type = "terraform"
name = "internal-tf"
mode = "local"          # or "hybrid" to fall back to registry.terraform.io

[registries.rbac]
anonymous = []
user      = ["source:read"]
admin     = ["*"]
```

For hybrid mode add `upstreams = ["https://registry.terraform.io"]`.

### Publishing modules

A Terraform module is a `.tar.gz` archive of the module directory.

```sh
# Build the archive
tar -czf consul-aws-0.1.0.tar.gz -C /path/to/module .

# Upload
curl -X POST \
  -H "Authorization: Bearer <your-token>" \
  -H "Content-Type: application/gzip" \
  --data-binary @consul-aws-0.1.0.tar.gz \
  "https://batlehub.example.com/proxy/internal-tf/v1/modules/hashicorp/consul/aws/0.1.0"
```

### Using a private module

Add credentials to `~/.terraformrc`:

```hcl
credentials "batlehub.example.com" {
  token = "<your-token>"
}
```

Reference the module in Terraform:

```hcl
module "consul" {
  source  = "batlehub.example.com/proxy/internal-tf/hashicorp/consul/aws"
  version = "0.1.0"
}
```

### Publishing providers

**Step 1 — Upload version manifest** (JSON describing the version and its platforms):

```sh
curl -X POST \
  -H "Authorization: Bearer <your-token>" \
  -H "Content-Type: application/json" \
  -d '{
    "version": "1.0.0",
    "protocols": ["5.0"],
    "platforms": [
      {
        "os": "linux", "arch": "amd64",
        "filename": "terraform-provider-mycloud_1.0.0_linux_amd64.zip",
        "shasum": "<sha256-hex>"
      }
    ]
  }' \
  "https://batlehub.example.com/proxy/internal-tf/v1/providers/myorg/mycloud/versions"
```

**Step 2 — Upload platform binaries**:

```sh
curl -X PUT \
  -H "Authorization: Bearer <your-token>" \
  -H "Content-Type: application/zip" \
  --data-binary @terraform-provider-mycloud_1.0.0_linux_amd64.zip \
  "https://batlehub.example.com/proxy/internal-tf/v1/providers/myorg/mycloud/1.0.0/artifact/linux/amd64"
```

Repeat the binary upload for each supported platform.

### Using a private provider

```hcl
# ~/.terraformrc
credentials "batlehub.example.com" {
  token = "<your-token>"
}
```

```hcl
# main.tf
terraform {
  required_providers {
    mycloud = {
      source  = "batlehub.example.com/proxy/internal-tf/myorg/mycloud"
      version = "~> 1.0"
    }
  }
}
```

### Yank a version (admin)

Use the admin bulk-operations API (see [Administration guide](/guide/administration)):

```sh
curl -X POST \
  -H "Authorization: Bearer <admin-token>" \
  -H "Content-Type: application/json" \
  -d '{"packages": [{"name": "modules/hashicorp/consul/aws", "versions": ["0.1.0"]}]}' \
  "https://batlehub.example.com/api/v1/admin/registries/internal-tf/bulk-yank"
```

### Endpoint reference

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/proxy/{registry}/v1/modules/{ns}/{name}/{provider}/{version}` | Upload module tarball |
| `GET` | `/proxy/{registry}/v1/modules/{ns}/{name}/{provider}/{version}/artifact` | Download module tarball |
| `GET` | `/proxy/{registry}/v1/modules/{ns}/{name}/{provider}/versions` | List module versions |
| `GET` | `/proxy/{registry}/v1/modules/{ns}/{name}/{provider}/{version}/download` | Download redirect (`X-Terraform-Get`) |
| `POST` | `/proxy/{registry}/v1/providers/{ns}/{type}/versions` | Upload provider manifest |
| `PUT` | `/proxy/{registry}/v1/providers/{ns}/{type}/{version}/artifact/{os}/{arch}` | Upload platform binary |
| `GET` | `/proxy/{registry}/v1/providers/{ns}/{type}/{version}/artifact/{os}/{arch}` | Download platform binary |
| `GET` | `/proxy/{registry}/v1/providers/{ns}/{type}/versions` | List provider versions |
| `GET` | `/proxy/{registry}/v1/providers/{ns}/{type}/{version}/download/{os}/{arch}` | Provider download info JSON |

---

## Authentication

Terraform reads per-host credentials from the `credentials "batlehub.example.com"` block in `~/.terraformrc` (shown above) and sends the token as a Bearer header.

## Notes

- Providers are cached after first download in proxy/hybrid mode, or served entirely from local storage in local mode.
- The module upload response includes an `X-Terraform-Get` header pointing at the artifact download URL.

## See also

- [Using BatleHub](/use/) — tokens, publishing prerequisites, the CLI
- [Registries overview](/registries/) · [Caching](/guide/caching) · [Access Control](/guide/access-control)
