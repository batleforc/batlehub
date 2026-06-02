# SBOM Support

BatleHub generates Software Bills of Materials (SBOMs) for every artifact it caches or hosts in a local / hybrid registry. SBOMs are stored in the database and served via a REST API. They can be consumed by vulnerability scanners, compliance tooling, and license audits without any changes to existing build pipelines.

---

## Supported formats

| Format | Spec version | File extension |
|--------|-------------|----------------|
| SPDX | 2.3 | `.spdx.json` |
| CycloneDX | 1.4 | `.cyclonedx.json` |

Both formats are generated for each artifact when SBOM is enabled. You can request either via the `?format=spdx` or `?format=cyclonedx` query parameter.

**When to use which:**
- **SPDX 2.3** — preferred for license compliance and OpenChain-conformant workflows (ISO/IEC 5962).
- **CycloneDX 1.4** — preferred for security tooling, vulnerability scanning, and SBOM-driven dependency analysis (OWASP standard).

---

## How BatleHub generates SBOMs

For each artifact, BatleHub tries the following sources in priority order:

1. **Upstream API** — fetch a pre-built SBOM from the upstream registry (GitHub dependency graph API, npm `bom.json`). Uses the highest-quality data when available.
2. **Archive extraction** — parse the dependency manifest inside the downloaded archive. Supported manifests:

   | Registry | Manifest file |
   |----------|--------------|
   | Cargo | `Cargo.toml` |
   | npm | `package.json` |
   | Maven | `pom.xml` |
   | Go | `go.mod` |
   | PyPI | `requirements.txt`, `pyproject.toml` |

3. **Minimal generation** — if neither upstream nor archive yields a manifest, BatleHub produces a minimal document from the package metadata (name, version, ecosystem PURL, checksum). No dependency list, but still a valid SBOM.

The `source` field in each stored SBOM record indicates which path was taken: `Upstream`, `Extracted`, or `Generated`.

---

## Configuration

Enable SBOM generation per registry in `config.toml`:

```toml
[[registries]]
type = "cargo"
name = "crates-io"

[registries.sbom]
enabled        = true
formats        = ["spdx", "cyclonedx"]   # default: both
fetch_upstream = true                    # try upstream APIs first
required       = false                   # only relevant for local/hybrid mode
```

### Option reference

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `enabled` | bool | `false` | Enable SBOM generation. Must be `true` for any SBOM functionality. |
| `formats` | `["spdx"]` / `["cyclonedx"]` / `["spdx", "cyclonedx"]` | both | Which formats to generate and store. |
| `fetch_upstream` | bool | `true` | Attempt to retrieve a pre-built SBOM from the upstream before extracting or generating. |
| `required` | bool | `false` | For `local` / `hybrid` registries only: deny the publish request with HTTP 422 if no dependency manifest is found in the uploaded archive. |

---

## API endpoints

### Per-artifact SBOM

```
GET /api/v1/sbom/{registry}/{name}/{version}?format=spdx|cyclonedx
```

Requires an authenticated (non-anonymous) user.

**Path parameters:**

| Parameter | Description |
|-----------|-------------|
| `registry` | Registry name as defined in `config.toml` |
| `name` | Package name |
| `version` | Exact version string |

**Query parameters:**

| Parameter | Default | Description |
|-----------|---------|-------------|
| `format` | `spdx` | SBOM format: `spdx` or `cyclonedx` |

**Responses:**

| Status | Description |
|--------|-------------|
| 200 | SBOM document as JSON |
| 400 | Unknown format string |
| 403 | Request is not authenticated |
| 404 | No SBOM stored for this artifact |

The endpoint tries both the proxy artifact key (`artifact:{registry}/{name}/{version}`) and the local key (`local:{registry}/{name}/{version}`) automatically.

**Example:**

```sh
curl -H "Authorization: Bearer $TOKEN" \
  "https://batlehub.example.com/api/v1/sbom/crates-io/serde/1.0.0?format=spdx" \
  | jq .spdxVersion
# "SPDX-2.3"

curl -H "Authorization: Bearer $TOKEN" \
  "https://batlehub.example.com/api/v1/sbom/npm/lodash/4.17.21?format=cyclonedx" \
  | jq .bomFormat
# "CycloneDX"
```

### Org-level SBOM export

```
GET /api/v1/sbom/export?registry=…&from=…&to=…&format=spdx|cyclonedx
```

Requires the `admin` role. Returns a single merged SBOM document covering all artifacts whose records fall within the requested time window. The response includes `Content-Disposition: attachment` so browsers and `curl -O` download it directly.

**Query parameters:**

| Parameter | Required | Description |
|-----------|----------|-------------|
| `format` | No (default: `spdx`) | SBOM format: `spdx` or `cyclonedx` |
| `registry` | No | Filter to one registry. Omit to include all. |
| `from` | No | Earliest artifact creation timestamp (ISO 8601, e.g. `2025-01-01T00:00:00Z`) |
| `to` | No | Latest artifact creation timestamp (ISO 8601) |

**Responses:**

| Status | Description |
|--------|-------------|
| 200 | Merged SBOM document; `Content-Disposition: attachment; filename="sbom-export-{registry}-{timestamp}.{ext}"` |
| 400 | Unknown format string |
| 403 | Admin role required |

**Merge behaviour:** packages are deduplicated by `name@version`. The resulting document lists each unique package once, regardless of how many registries served it. Relationships are preserved from the source SBOMs.

**Examples:**

```sh
# Export all SBOMs from the last 30 days (SPDX)
FROM=$(date -u -d '30 days ago' +%Y-%m-%dT%H:%M:%SZ 2>/dev/null \
     || date -u -v-30d +%Y-%m-%dT%H:%M:%SZ)  # macOS fallback

curl -H "Authorization: Bearer $ADMIN_TOKEN" \
  "https://batlehub.example.com/api/v1/sbom/export?from=${FROM}&format=spdx" \
  -O -J   # saves as sbom-export-all-<timestamp>.spdx.json

# Export only the internal-npm registry, CycloneDX
curl -H "Authorization: Bearer $ADMIN_TOKEN" \
  "https://batlehub.example.com/api/v1/sbom/export?registry=internal-npm&format=cyclonedx" \
  -O -J
```

---

## Admin UI

The admin panel at `/admin/sbom` provides a form to configure and download the org-level SBOM export without needing to construct a URL manually:

- **Registry** — optional filter
- **From / To** — date range pickers
- **Format** — SPDX 2.3 or CycloneDX 1.4
- **Download** button — triggers the export and saves the file

Per-artifact SBOMs are accessible from the **Package Explorer** (`/explore`): open any package's detail page, find the version row, and click the **SPDX** or **CDX** button. A "No SBOM" label appears if no SBOM was generated for that version (e.g. the registry has SBOM disabled, or the upstream provided no manifest).

---

## Package URL (PURL) mapping

Each package entry in the generated SBOM includes a [PURL](https://github.com/package-url/purl-spec) for interoperability with vulnerability scanners (Grype, Trivy, OSV-Scanner):

| Registry type | PURL example |
|---------------|-------------|
| `cargo` | `pkg:cargo/serde@1.0.0` |
| `npm` | `pkg:npm/lodash@4.17.21` |
| `maven` | `pkg:maven/org.springframework/spring-core@6.0.0` |
| `pypi` | `pkg:pypi/requests@2.31.0` |
| `rubygems` | `pkg:gem/rails@7.1.0` |
| `goproxy` | `pkg:golang/github.com/gin-gonic/gin@v1.9.1` |
| `terraform` | `pkg:terraform/hashicorp/aws@5.0.0` |
| `composer` | `pkg:composer/symfony/console@7.0.0` |
| `conda` | `pkg:conda/numpy@1.26.0` |
| everything else | `pkg:generic/{name}@{version}` |

---

## Worked examples

### Compliance audit — export everything for the last quarter

```sh
YEAR=$(date +%Y)
Q_START="${YEAR}-01-01T00:00:00Z"
Q_END="${YEAR}-03-31T23:59:59Z"

curl -H "Authorization: Bearer $ADMIN_TOKEN" \
  "https://batlehub.example.com/api/v1/sbom/export?from=${Q_START}&to=${Q_END}&format=spdx" \
  -O -J

# Feed into an SPDX-compatible auditor, e.g. FOSSA or Dependency-Track
```

### Vulnerability scan with Grype

```sh
# Download the CycloneDX SBOM for one artifact
curl -H "Authorization: Bearer $TOKEN" \
  "https://batlehub.example.com/api/v1/sbom/npm/express/4.18.0?format=cyclonedx" \
  -o express-4.18.0.cyclonedx.json

# Scan with Grype
grype sbom:express-4.18.0.cyclonedx.json
```

### Private registry — enforce SBOM at publish time

```toml
[[registries]]
type = "npm"
name = "internal-npm"
mode = "local"

[registries.rbac]
user  = ["releases:read", "source:read"]
admin = ["*"]

[registries.sbom]
enabled  = true
required = true
```

With `required = true`, publishing a tarball that does not contain a `package.json` returns:

```
HTTP 422 Unprocessable Entity
{"error": "no dependency manifest found in archive"}
```

### CI pipeline integration

```yaml
# .github/workflows/sbom.yml
- name: Download SBOM
  run: |
    curl -fsSL \
      -H "Authorization: Bearer ${{ secrets.BATLEHUB_TOKEN }}" \
      "${{ vars.BATLEHUB_URL }}/api/v1/sbom/export?format=cyclonedx" \
      -o sbom.cyclonedx.json

- name: Upload SBOM artifact
  uses: actions/upload-artifact@v4
  with:
    name: sbom
    path: sbom.cyclonedx.json
```

---

## Database schema

SBOMs are stored in the `artifact_sboms` table (migration `020_artifact_sboms.sql`):

| Column | Type | Description |
|--------|------|-------------|
| `id` | UUID | Primary key |
| `artifact_key` | text | `artifact:{registry}/{name}/{version}` or `local:{registry}/{name}/{version}` |
| `registry` | text | Registry name |
| `package_name` | text | Package name |
| `version` | text | Version string |
| `format` | text | `spdx` or `cyclonedx` |
| `spec_version` | text | Format spec version (`SPDX-2.3`, `1.4`) |
| `document` | jsonb | Full SBOM document |
| `source` | text | `Generated`, `Extracted`, or `Upstream` |
| `created_at` | timestamptz | When the SBOM was stored |

One row per `(artifact_key, format)` pair. Upsert semantics: re-proxying the same artifact replaces the previous SBOM if a better source is now available.
