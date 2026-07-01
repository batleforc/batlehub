# Security Policy

## Supported versions

BatleHub is pre-1.0 (currently `0.x`). Only the **latest published release**
is supported with security fixes — there is no long-term-support branch and
no backport guarantee for older `0.x` versions. Upgrading to the latest
release is the recommended way to pick up a fix.

## Reporting a vulnerability

Please **do not** open a public GitHub issue for a suspected security
vulnerability. Instead, use one of the following:

- Open an issue using the [Security Issue template](.github/ISSUE_TEMPLATE/security-issue.md)
  and mark it clearly if it should be handled privately before any public
  detail is posted.
- If the finding is sensitive enough that even the existence of the report
  should stay private, contact the maintainer directly rather than filing a
  public issue.

When reporting, please include:

- A description of the vulnerability and its potential impact.
- Steps to reproduce, or a proof of concept.
- The affected component(s) (e.g. a specific registry adapter, the auth
  middleware, the RBAC rule engine).

## Response expectations

BatleHub is maintained by a single developer, not a security team with a
formal SLA. Reports will be triaged and acknowledged on a best-effort basis;
there is no guaranteed response time. Fixes for confirmed vulnerabilities
are prioritized ahead of feature work.

## Scope

This policy covers the BatleHub server, CLI, and web UI in this repository.
Vulnerability scanning of BatleHub's own dependencies and container images
is continuous — see [`docs/security-scanning.md`](docs/security-scanning.md)
for the full matrix of automated checks (`cargo audit`, `cargo deny`,
`npm audit`, CodeQL, Semgrep, gitleaks, Trivy).
