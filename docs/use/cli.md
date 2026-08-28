# batlehub-cli

`batlehub-cli` is the official command-line client for BatleHub. It provides both a traditional CLI for scripting and CI pipelines, and an interactive TUI for everyday browsing and management.

## 1. Installation

**via mise** (recommended — manages version automatically):

```bash
mise use "github:batleforc/batlehub[asset_pattern=batlehub-cli-*]"
```

**via cargo** (builds from source — requires Rust toolchain):

```bash
cargo install --git https://github.com/batleforc/batlehub batlehub-cli
```

**Pre-built binaries** — download from [GitHub Releases](https://github.com/batleforc/batlehub/releases/latest):

```bash
# Linux x86_64
curl -fSL https://github.com/batleforc/batlehub/releases/latest/download/batlehub-cli-linux-amd64.tar.gz | tar xz
sudo mv batlehub-cli /usr/local/bin/batlehub-cli

# Linux aarch64
curl -fSL https://github.com/batleforc/batlehub/releases/latest/download/batlehub-cli-linux-arm64.tar.gz | tar xz
sudo mv batlehub-cli /usr/local/bin/batlehub-cli

# macOS Apple Silicon (M1/M2/M3)
curl -fSL https://github.com/batleforc/batlehub/releases/latest/download/batlehub-cli-darwin-arm64.tar.gz | tar xz
sudo mv batlehub-cli /usr/local/bin/batlehub-cli

# macOS Intel
curl -fSL https://github.com/batleforc/batlehub/releases/latest/download/batlehub-cli-darwin-amd64.tar.gz | tar xz
sudo mv batlehub-cli /usr/local/bin/batlehub-cli

# Windows (PowerShell)
Invoke-WebRequest https://github.com/batleforc/batlehub/releases/latest/download/batlehub-cli-windows-amd64.zip -OutFile batlehub-cli.zip
Expand-Archive batlehub-cli.zip -DestinationPath .
Move-Item batlehub-cli.exe "$env:LOCALAPPDATA\Microsoft\WindowsApps\batlehub-cli.exe"
```

Or run directly without installing (inside the repository):

```bash
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
batlehub-cli registry suggest [--dir <path>] [--depth N] [--client-env] [--mise [--mise-commented]] [--include-existing]
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

### `registry suggest`

Work out which registries a project actually needs, and print the
`[[registries]]` blocks to paste into `config.toml`.

Two inputs, in decreasing order of precision:

- **`mise.lock`** — the best source available: it records the exact download URL
  of every tool, per platform. Each URL maps either onto a typed registry (a
  `github.com` release asset → `type = "github"`) or, for hosts that speak no
  package protocol at all, onto a `generic` mirror of that host.
- **`mise.toml`** and the usual project manifests (`Cargo.toml`, `go.mod`,
  `package.json`, `pyproject.toml`, `pom.xml`, `composer.json`, `*.gemspec`,
  `*.nuspec`, `*.csproj`, `*.tf`, `environment.yml`) — no URLs, so the mapping
  goes by backend prefix / tool name and is best-effort. When a lock file is
  present it takes precedence, since it names the same tools more precisely.

```console
$ batlehub-cli registry suggest --client-env
+------------------+---------+--------------------------------------+----------------------------+
| Name             | Type    | Upstream                             | Detected from              |
+------------------+---------+--------------------------------------+----------------------------+
| cargo            | cargo   | (adapter default)                    | manifest, mise.lock: …     |
| github           | github  | (adapter default)                    | mise.lock: gitleaks, …     |
| node-dist        | generic | https://nodejs.org/dist              | mise.lock: node            |
| rust-dist        | generic | https://static.rust-lang.org         | mise.lock: rust            |
| helm-bin         | generic | https://get.helm.sh                  | mise.lock: helm            |
+------------------+---------+--------------------------------------+----------------------------+

Add to config.toml:
…
Point clients at the proxy:

# node-dist (generic)
export NODEJS_ORG_MIRROR="https://batlehub.example.com/proxy/node-dist/generic"
```

| Flag | Description |
|------|-------------|
| `--dir <path>`, `-d` | Directory to scan (default: current working directory) |
| `--depth N` | Subdirectory levels to scan for manifests (default 0 = root only). Does not affect `mise.lock`, which is only read from the root. |
| `--client-env` | Also print the environment variables that point each toolchain at the proxy |
| `--mise` | Also print a mise `[settings.url_replacements]` block routing mise itself through the proxy |
| `--mise-commented` | Comment out every line of the `--mise` block, for committing into a shared `mise.toml` |
| `--include-existing` | Emit suggestions even when the server already has a registry of that type |

#### Routing mise itself (`--mise`)

`[settings.url_replacements]` rewrites the URLs **mise's own HTTP layer**
fetches, which covers the aqua/ubi/GitHub-release backends and every `generic`
mirror. Verified against `mise install`:

- aqua resolves *and downloads* assets through `api.github.com/repos/…/releases/assets/{id}`,
  so the `api.github.com` rule is the load-bearing one — not the
  `github.com/…/releases/download/…` rule.
- `core:node` fetches **both** the platform tarball and the `node-v<ver>.tar.gz`
  source tarball, which is why the suggested `path_allow` is `v*/**` rather than
  a platform-only glob.

Backends that shell out to another tool (`cargo:`, `pipx:`, `npm:`, `go:`) are
**not** covered — those processes read their own config, not mise's. The
generated block names them explicitly rather than leaving them silently absent;
use `--client-env` and the per-ecosystem config for those.

> The regex keys must reach the file with **doubled** backslashes
> (`\\.`). TOML treats a lone `\` as an invalid escape, and mise responds by
> logging one line and running on with the entire settings block dropped — a
> silent no-op. The generator handles this; hand-edits should keep it in mind.

`--mise-commented` prefixes every line, including the generator's own header
comments, so that stripping exactly one `#` (and its trailing space) per line
yields a valid file. This
repo's own `mise.toml` carries such a block as a worked example.

Every generated `generic` block carries the `upstreams` and `path_allow` fields
the server requires for that type, so the output is directly usable. Note the
difference in allowlist precision:

- For hosts with a **curated preset** (`nodejs.org`, `static.rust-lang.org`,
  `dl.google.com`, `get.helm.sh`, `dl.min.io`, `binaries.sonarsource.com`), the
  allowlist is a version-agnostic glob and keeps working across version bumps.
- For any **other host**, the allowlist is the set of exact paths found in the
  lock — narrow and provably sufficient for the pinned versions, but needing a
  re-run (or a manual widening) when those versions change. The generated TOML
  says so in a comment above the block.

On object-storage hosts (`storage.googleapis.com`, `s3.amazonaws.com`) the
bucket segment is folded into `upstreams`, not left in the path — otherwise the
mirror would relay every other public bucket on the same host.

Scanning is entirely local: the server is contacted only to annotate which types
are already configured, and an unreachable server degrades that annotation
rather than failing the command. `--json` emits the structured suggestions plus
the rendered TOML under a `toml` key.

---

## 5. Commands — package

```
batlehub-cli package list   [--registry <r>] [--search <q>] [--blocked-only] [--page N] [--per-page N]
batlehub-cli package versions <registry> <name>
batlehub-cli package readme   <registry>/<name>[@<version>] [--no-upstream]
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

### `package readme <registry>/<name>[@<version>]`

Print a version's README — the **source**, not a rendering. Markdown in a
terminal is readable, and turning it into ANSI is a separate concern.

```
$ batlehub-cli package readme internal/mylib@1.4.2
# mylib

Does a thing.
```

Without a version, the newest one that has a README answers. When the version
you asked for ships none, the newest that does answers instead — and says so:

```
$ batlehub-cli package readme internal/mylib@2.0.0-rc1 > README.md
note: showing 1.4.2's README; version 2.0.0-rc1 ships none
```

**Every qualification goes to stderr**, so redirecting stdout writes the document
and nothing else. The notes you may see: a fallback from another version, a
README that is the *package's* rather than this version's, one read from the
upstream's own answer because nothing of this version is held here, one
truncated at the registry's `max_bytes`, and one that is not markdown.

`--no-upstream` answers from what this instance holds, without asking the
registry's upstream about a version it holds nothing of — for a script, or for a
host with no route off site. See
[what leaves this instance](/operations/egress#the-console-s-discovery-read).

`--json` prints the whole response, so a script can read `is_fallback`, `stored`
and `truncated` rather than parsing the notes:

```bash
batlehub-cli --json package readme internal/mylib@1.4.2 | jq -r .source_text
```

Which registries carry a README at all, and where each one's comes from, is in
the [README support table](/registries/#readmes).

---

## 6. Commands — version {#commands-version}

```
batlehub-cli version yank   <registry> <name> <version>
batlehub-cli version unyank <registry> <name> <version>
batlehub-cli version delete <registry> <name> <version> [--yes]
batlehub-cli version pin    <registry> <name> <version>
batlehub-cli version unpin  <registry> <name> <version>
```

These commands require an admin token.

| Command | Effect |
|---------|--------|
| `yank` | Marks a version unavailable (kept in storage, download blocked) |
| `unyank` | Reverses a yank |
| `delete` | Drops the artifact **and spends the version number permanently** |
| `pin` | Exempts a version from retention — it is never reclaimed automatically |
| `unpin` | Releases the pin, so the registry's retention policy applies again |

> **Package name casing**: package names are normalized to lowercase when published (NuGet lowercases the package ID, cargo and npm use lowercase by convention). Use the lowercase form with `version yank/unyank/delete` to match the stored name — e.g. `serilog`, not `Serilog`.

`delete` prompts for confirmation unless `--yes` is passed:

```
$ batlehub-cli version delete internal serilog 2.0.0
Delete internal/serilog@2.0.0? The artifact is dropped and the version number is
spent permanently — 2.0.0 can never be published again. [y/N] y
Deleted internal/serilog@2.0.0
```

A deleted version number is never reused. Publishing `2.0.0` again is refused
with `409`, whoever asks and however long afterwards, so "delete and re-upload to
fix it" is not a plan — publish `2.0.1`, or `yank` instead if you only need the
version to stop being installed. The reasoning, and what the deletion leaves
behind for an auditor, are in
[Deleting a published version](/guide/admin-policies#deleting-versions).

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
batlehub-cli publish <file> [--registry <r>] [--name <n>] [--version <v>] [--type <t>]
                             [--distribution <d>] [--component <c>] [--platform <p>]
```

Upload an artifact to a local or hybrid registry. The CLI auto-detects the registry type and package metadata from the file:

| Extension | Registry type | Metadata source |
|-----------|---------------|-----------------|
| `.nupkg` | nuget | embedded `.nuspec` |
| `.whl` | pypi | filename (`name-version-*.whl`) |
| `.gem` | rubygems | filename (`name-version.gem`) |
| `.pkg.tar.{zst,xz,gz}` | pacman | filename (`name-pkgver-pkgrel-arch.pkg.tar.*`) |
| `.tgz` | npm | filename (`name-version.tgz`, as produced by `npm pack`) |
| `.crate` | cargo | filename (`name-version.crate`, as produced by `cargo package`) |
| `.vsix` | openvsx | filename (`extension_id-version.vsix`) |
| `.deb` | deb | server-side, from the package's control file — requires `--distribution` and `--component` |
| `.rpm` | rpm | server-side, from the package's header |
| `.tar.bz2` / `.conda` | conda | server-side, from the package's own `info/index.json` (`--platform` is only a fallback) |

Composer ZIPs share the generic `.zip` extension with other formats and are not auto-detected — pass `--type composer` explicitly. Composer, like conda/deb/rpm, parses name/version server-side from `composer.json`, so no `--name`/`--version` is needed (an optional `--version` overrides the archive's own version).

Use `--type` to override auto-detection entirely — useful for ambiguous extensions or when a file doesn't follow the expected naming convention.

Maven (separate jar+pom+checksum files), Terraform (providers need shasums/signature files; modules need a packaging step), and Go modules (need an `.info`/`.mod`/`.zip` triad) don't fit this command's single-file model by design. Use your existing tooling (`mvn deploy`, Terraform registry publishing conventions, `go mod`) configured to point at the BatleHub endpoint — see [`docs/use/publishing.md`](publishing.md) for per-registry setup instructions.

```bash
# NuGet
batlehub-cli publish Serilog.3.1.1.nupkg --registry internal

# Override detected metadata
batlehub-cli publish dist/mylib-1.2.3.tar.gz --type pypi --name mylib --version 1.2.3

# Composer (ambiguous .zip extension — type must be explicit)
batlehub-cli publish acme-widget.zip --type composer --registry internal

# Debian (distribution/component aren't in the filename)
batlehub-cli publish hello_1.0-1_amd64.deb --registry internal --distribution stable --component main

# Conda (platform is only a fallback for packages with no embedded subdir)
batlehub-cli publish numpy-1.26.0-py311h0.conda --registry internal --platform linux-64
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

### Retention

```
batlehub-cli admin retention <registry> [--show-kept] [--reclaim]
```

Reclaims locally published versions the registry's `[registries.retention]`
policy no longer keeps. **Reports by default** — `--reclaim` is only half the
interlock, and the registry also needs `dry_run = false`. Two decisions in two
places, because a reclaimed artifact may exist nowhere else.

`--show-kept` prints every surviving version and the condition that saved it,
which is how you check a policy against what it actually does before arming it.

Pinning a single version against retention is
[`version pin`](#commands-version).

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

## 12. Commands — setup

```
batlehub-cli setup detect [--dir <path>] [--depth <n>] [--offline] [--json]
batlehub-cli setup ide [--offline] [--json]
```

`setup detect` scans a directory for project manifests (`Cargo.toml`, `go.mod`,
`package.json`, `pyproject.toml`, `pom.xml`, `composer.json`, `*.gemspec`,
`*.nuspec`, `*.csproj`, `*.tf`, `environment.yml`) and prints the configuration
snippet for each package manager it finds. `setup ide` does the same for the
editor you are running in (VS Code / VSCodium → OpenVSX or the VS Code
Marketplace; JetBrains → the JetBrains Marketplace).

Both ask the server which registries exist, so the snippets carry the real
registry name and the URL that registry actually answers on — its own subdomain
when [host-based routing](../rfc/0001-subdomain-routing.md) advertises one,
`{server}/proxy/{name}` otherwise. Each run ends with the matching `~/.netrc`
stanzas, one per host: credentials are matched by hostname, so a host-routed
registry needs its own entry.

If the server cannot be reached the commands still work — they print `<registry>`
placeholders and say so on stderr. `--offline` skips the request entirely.

---

## 13. TUI mode

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
