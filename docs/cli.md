# batlehub-cli

`batlehub-cli` is the official command-line client for BatleHub. It provides both a traditional CLI for scripting and CI pipelines, and an interactive TUI for everyday browsing and management.

---

## Table of Contents

1. [Installation](#1-installation)
2. [Configuration](#2-configuration)
3. [Global flags](#3-global-flags)
4. [Commands — registry](#4-commands--registry)
5. [Commands — package](#5-commands--package)
6. [Commands — version](#6-commands--version)
7. [Commands — owners](#7-commands--owners)
8. [Commands — publish](#8-commands--publish)
9. [Commands — auth](#9-commands--auth)
10. [Commands — admin](#10-commands--admin)
11. [Commands — config](#11-commands--config)
12. [TUI mode](#12-tui-mode)

---

## 1. Installation

Build from source inside the repository:

```bash
cargo build -p batlehub-cli --release
# Binary at: target/release/batlehub-cli
```

Or run directly without installing:

```bash
# Using the Taskfile helpers
task cli -- registry list
task cli:tui
task cli:help
```

---

## 2. Configuration

`batlehub-cli` reads `~/.config/batlehub/config.toml`. Run the setup wizard to create it:

```bash
batlehub-cli config init
```

The file uses TOML and supports named profiles:

```toml
[default]
server_url = "http://localhost:8080"
token      = "my-secret-token"
registry   = "my-registry"        # optional default registry

[profiles.prod]
server_url = "https://batlehub.example.com"
token      = "prod-secret-token"
```

### Environment variable overrides

Every connection setting can be overridden by environment variables — useful in CI without touching the config file:

| Variable            | Equivalent flag  |
|---------------------|------------------|
| `BATLEHUB_SERVER`   | `--server`       |
| `BATLEHUB_TOKEN`    | `--token`        |
| `BATLEHUB_REGISTRY` | `--registry`     |
| `BATLEHUB_PROFILE`  | `--profile`      |

---

## 3. Global flags

These flags are available on every command:

| Flag | Short | Description |
|------|-------|-------------|
| `--profile <name>` | `-P` | Use a named config profile |
| `--server <url>` | | Override the server URL |
| `--token <tok>` | | Override the auth token |
| `--registry <name>` | `-r` | Set a default registry |
| `--json` | | Emit machine-readable JSON instead of tables |

---

## 4. Commands — registry

```
batlehub-cli registry list
batlehub-cli registry info <name>
```

### `registry list`

List all registries visible to the current identity.

```
$ batlehub-cli registry list
+----------+---------+--------+
| Name     | Type    | Mode   |
+----------+---------+--------+
| cargo    | cargo   | proxy  |
| internal | nuget   | hybrid |
| pypi     | pypi    | local  |
+----------+---------+--------+
3 registry/registries
```

### `registry info <name>`

Show type and mode for a single registry.

---

## 5. Commands — package

```
batlehub-cli package list   [--registry <r>] [--search <q>] [--blocked-only] [--page N] [--per-page N]
batlehub-cli package versions <registry> <name>
```

### `package list`

List packages across all accessible registries (or just one with `--registry`).

```
$ batlehub-cli package list --registry internal --search serilog
+----------+----------+-------------------+-----------+---------+
| Registry | Name     | Version           | Status    | Accesses|
+----------+----------+-------------------+-----------+---------+
| internal | Serilog  | 3.1.1             | available | 1234    |
| internal | Serilog  | 3.0.0             | blocked:… | 89      |
+----------+----------+-------------------+-----------+---------+
```

Use `--json` to get the raw JSON array — useful in scripts:

```bash
batlehub-cli --json package list --registry internal | jq '.[].name' | sort -u
```

The JSON items use an internally-tagged `status` field:

```json
[
  { "registry": "internal", "name": "serilog", "version": "3.1.1",
    "status": {"status": "available"}, "access_count": 1234 },
  { "registry": "internal", "name": "serilog", "version": "3.0.0",
    "status": {"status": "blocked", "reason": "yanked"}, "access_count": 89 }
]
```

Filter in scripts with `jq`:
```bash
# List only blocked packages
batlehub-cli --json package list | jq '[.[] | select(.status.status == "blocked")]'
```

### `package versions <registry> <name>`

List all cached versions of a package with their status and download count.

---

## 6. Commands — version

```
batlehub-cli version yank   <registry> <name> <version>
batlehub-cli version unyank <registry> <name> <version>
batlehub-cli version delete <registry> <name> <version> [--yes]
```

These commands require an admin token.

| Command | Effect |
|---------|--------|
| `yank` | Marks a version unavailable (kept in storage, download blocked) |
| `unyank` | Reverses a yank |
| `delete` | Permanently removes the version and its artifact |

> **Package name casing**: package names are normalized to lowercase when published (NuGet lowercases the package ID, cargo and npm use lowercase by convention). Use the lowercase form with `version yank/unyank/delete` to match the stored name — e.g. `serilog`, not `Serilog`.

`delete` prompts for confirmation unless `--yes` is passed:

```
$ batlehub-cli version delete internal Serilog 2.0.0
Permanently delete internal/Serilog@2.0.0? This cannot be undone. [y/N] y
Deleted internal/Serilog@2.0.0
```

---

## 7. Commands — owners

```
batlehub-cli owners list   <registry> <name>
batlehub-cli owners add    <registry> <name> <principal> [--type user|group] [--role admin|maintainer]
batlehub-cli owners remove <registry> <name> <principal> [--type user|group]
```

Ownership controls who can publish new versions to a local/hybrid registry. Requires an admin token.

```
$ batlehub-cli owners list internal Serilog
+------+------------------+------------+------------+
| Type | Principal        | Role       | Granted By |
+------+------------------+------------+------------+
| user | alice@example.com| admin      | -          |
| group| nuget-maintainers| maintainer | alice      |
+------+------------------+------------+------------+

$ batlehub-cli owners add internal Serilog bob --type user --role maintainer
Added user 'bob' as maintainer on internal/Serilog
```

---

## 8. Commands — publish

```
batlehub-cli publish <file> [--registry <r>] [--name <n>] [--version <v>]
```

Upload an artifact to a local or hybrid registry. The CLI auto-detects the registry type and package metadata from the file:

| Extension | Registry type | Metadata source |
|-----------|---------------|-----------------|
| `.nupkg` | nuget | embedded `.nuspec` |
| `.whl` | pypi | filename (`name-version-*.whl`) |
| `.gem` | rubygems | filename (`name-version.gem`) |

If auto-detection fails or the registry type is not yet natively supported, use your existing tooling (`dotnet nuget push`, `twine upload`, `cargo publish`, …) configured to point at the BatleHub endpoint. See [`docs/publishing.md`](publishing.md) for per-registry setup instructions.

```bash
# NuGet
batlehub-cli publish Serilog.3.1.1.nupkg --registry internal

# Override detected metadata
batlehub-cli publish dist/mylib-1.2.3.tar.gz --registry pypi --name mylib --version 1.2.3
```

---

## 9. Commands — auth

```
batlehub-cli auth whoami
batlehub-cli auth token list
batlehub-cli auth token create --name <n> [--days <d>] [--role user|admin]
batlehub-cli auth token revoke <uuid>
```

### `auth whoami`

Print the identity resolved from the current token:

```
$ batlehub-cli auth whoami
+----------+-----------------------+
| User ID  | alice@example.com     |
| Role     | admin                 |
| Provider | oidc                  |
| Groups   | nuget-maintainers, …  |
+----------+-----------------------+
```

### `auth token create`

Create a long-lived API token (requires an active OIDC session). The raw token is printed exactly once — store it immediately:

```
$ batlehub-cli auth token create --name ci-pipeline --days 90
Created token 'ci-pipeline' (role: user, expires: 2026-09-02)

Token (store this — it will not be shown again):
  bhub_XXXXXXXXXXXXXXXXXXXX
```

Use the resulting token as `BATLEHUB_TOKEN` in CI:

```yaml
# GitHub Actions example
- run: cargo publish --registry batlehub
  env:
    BATLEHUB_TOKEN: ${{ secrets.BATLEHUB_TOKEN }}
```

---

## 10. Commands — admin

These commands require an admin token.

### Quota

```
batlehub-cli admin quota list   [--registry <r>]
batlehub-cli admin quota reset  <registry> <user>
```

### IP blocks

```
batlehub-cli admin ip-block list
batlehub-cli admin ip-block add    <ip> [--reason <text>]
batlehub-cli admin ip-block remove <ip>
```

### Config

```
batlehub-cli admin config reload    # trigger hot reload on the server
batlehub-cli admin config changes   # view change history
```

### Cache

```
batlehub-cli admin cache warm  <registry> [--packages pkg1,pkg2]
batlehub-cli admin cache clear <registry>
```

### Banner

```
batlehub-cli admin banner set   "Maintenance at 22:00 UTC" [--level info|warning|error]
batlehub-cli admin banner clear
```

### Audit log

```
batlehub-cli admin audit-log [--registry <r>] [--user <id>] [--from <date>] [--to <date>] [--denied-only]
```

---

## 11. Commands — config

```
batlehub-cli config init           # interactive first-run wizard
batlehub-cli config show           # print resolved config (token is masked)
batlehub-cli config set server_url https://batlehub.example.com
batlehub-cli config set token      my-token [--profile prod]
batlehub-cli config set registry   internal [--profile prod]
```

Valid keys for `config set`: `server_url`, `token`, `registry`.

---

## 12. TUI mode

```
batlehub-cli tui
# or
task cli:tui
```

The TUI is a full-screen terminal interface built with [ratatui](https://ratatui.rs).

### Screens

```
╔ BatleHub — Registries ═══════════════════════════════╗
║ > cargo    (cargo  ) [proxy ]                         ║
║   internal (nuget  ) [hybrid]                         ║
║   pypi     (pypi   ) [local ]                         ║
╚══════════════════════════════════════════════════════╝
 q:quit  ↑↓:navigate  Enter:select  p:publish  ?:help
```

| Screen | How to reach |
|--------|--------------|
| Registry list | Launch / `Esc` from package list |
| Package list | `Enter` on a registry |
| Version detail | `Enter` on a package |
| Publish wizard | `p` from registry list |
| Help | `?` from any screen |

### Keyboard shortcuts

| Key | Action |
|-----|--------|
| `q` / `Ctrl-C` | Quit |
| `Esc` | Go back one screen |
| `↑` / `k` | Move selection up |
| `↓` / `j` | Move selection down |
| `Enter` | Open selected item |
| `/` | Toggle package search filter |
| `y` | Yank selected version (version detail screen) |
| `u` | Unyank selected version |
| `p` | Open publish wizard |
| `?` | Toggle help overlay |
| `Tab` / `Shift-Tab` | Cycle fields in publish wizard |
