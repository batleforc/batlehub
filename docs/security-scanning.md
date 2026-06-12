# Vulnerability scanning & SBOMs

batlehub is scanned for CVEs continuously, across every layer it ships. This page describes the
layers, how to reproduce them locally, and how to match a **future-disclosed** CVE against a build
you have already deployed.

## Layers

| Layer | Tool | Where it runs | Gate |
| --- | --- | --- | --- |
| Rust advisories | `cargo audit` (RUSTSEC) | `back-dep-audit.yaml` (PR + daily) | block |
| Rust advisories + bans + licenses + sources | `cargo deny` (`deny.toml`) | `back-dep-audit.yaml` | block |
| JS dependencies | `npm audit --audit-level=high` | `dep-audit-frontend.yaml` (PR + daily) | block on high/critical |
| Container / OS layers | Trivy | `image-scan.yaml` (PR + daily, GitHub) and `.forgejo/workflows/build.yaml` (both images) | block on fixable HIGH/CRITICAL |
| Static analysis | CodeQL + Semgrep | `codeql.yaml`, `semgrep.yaml` | CodeQL report / Semgrep block on ERROR |
| Secrets | gitleaks | `secret-scan.yaml` (PR + push) | block |
| Lint / unsafe hygiene | clippy `-D warnings` | `test.yaml` `lint` job | block |

The **daily** schedules are what turn this from a build-time snapshot into *future* CVE detection: a
CVE disclosed against a pinned dependency or a base-image layer **after** the last commit still trips
CI the next morning, with nothing in the repo having changed.

## Run the gate locally

```bash
task security        # cargo audit + cargo deny + ui/website npm audit + Rust SBOM
task deny            # just the cargo-deny supply-chain gate
task audit           # just cargo audit
task ui:audit        # just the frontend audit
```

Image scanning, secret scanning and SAST need their own tools (all provisioned by `mise install`):

```bash
# Build and scan the container image exactly as CI does
podman build -f Containerfile -t batlehub:scan .
trivy image --severity HIGH,CRITICAL --ignore-unfixed batlehub:scan

gitleaks detect --config gitleaks.toml          # secret scan
semgrep scan --config p/rust --config p/typescript
```

## SBOMs — matching a *future* CVE against a shipped build

Every release publishes two CycloneDX SBOMs:

- `sbom-rust.cdx.json` — the shipped server's Rust dependency closure (crate-level), attached to the
  GitHub release.
- `sbom-image.cdx.json` — the full container image (OS packages + binaries), attached to the
  release **and** pushed to the registry as an attestation (`actions/attest-sbom`).

When a new CVE is disclosed months later, you don't need to rebuild to know whether a deployed
version is affected — scan its SBOM:

```bash
# Match the latest advisory DB against an already-shipped SBOM
trivy sbom sbom-image.cdx.json
trivy sbom sbom-rust.cdx.json

# Or with grype
grype sbom:sbom-image.cdx.json
```

Verify the image SBOM/provenance attestation before trusting it:

```bash
gh attestation verify oci://ghcr.io/<owner>/batlehub:<version> --owner <owner>
```

## Suppressions

The stance is **no suppressions**: `.cargo/audit.toml` and `deny.toml` (`advisories.ignore = []`)
both keep the ignore list empty. If an advisory is genuinely non-actionable, prefer upgrading or
patching the dependency; only add an ignore with an inline justification and a tracking issue.
