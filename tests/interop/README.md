# Deb/RPM/pacman repository interop test

End-to-end proof that **real `apt`, `dnf`, and `pacman` accept** the Debian APT,
RPM/YUM, and Arch Linux repositories BatleHub hosts — specifically the **hand-rolled
Ed25519 OpenPGP signatures** (`InRelease`, `Release.gpg`, `repomd.xml.asc`,
`<repo>.db.sig`, the embedded `%PGPSIG%`) and the generated indexes
(`Packages`/`Release`, `repodata/`, `<repo>.db`).

## Run

```bash
task test:repo-interop      # or: bash tests/interop/verify.sh
```

Requires **podman or docker** (auto-detected; override with `CONTAINER_ENGINE`).
Pulls `debian:stable-slim`, `fedora:41`, and `archlinux:latest`.

## What it does

1. `verify.sh` runs the `#[ignore]`d `generate_signed_repos` test in
   `crates/adapters/tests/repo_interop.rs`, which builds a fixture `.deb`, `.rpm`,
   and `.pkg.tar.zst` and writes **signed** repos using the *production* code
   (`batlehub_adapters::repo` + `OpenPgpSigner`).
2. A Debian container adds the repo via `file://` with `signed-by=key.asc`, then
   `apt-get update` (verifies the `InRelease` signature) and
   `apt-get install hello-batlehub`.
3. A Fedora container adds a `.repo` with `repo_gpgcheck=1`, then `dnf makecache`
   (verifies `repomd.xml.asc`) and `dnf install hello-batlehub`.
4. An Arch container imports the key with `pacman-key --add`/`--lsign-key`, sets
   `SigLevel = Required`, then `pacman -Sy` (verifies `<repo>.db.sig`) and
   `pacman -S hello-batlehub` (verifies the package's embedded `%PGPSIG%`).

Any signature or metadata defect makes `apt`/`dnf`/`pacman` fail and the script
exits non-zero. CI runs this via `.github/workflows/repo-interop.yaml`.

## Why hand-rolled OpenPGP

Every OpenPGP library (rpgp, sequoia) and the `rpm` crate's default features pull
in the `rsa` crate, which is hard-banned in `deny.toml` (RUSTSEC-2023-0071). So the
signer in `crates/adapters/src/repo/openpgp.rs` emits just enough OpenPGP (Ed25519 /
EdDSA, algo 22) by hand. This test is how we keep that wire format honest — it has
already caught a missing **Key Flags** subpacket and the **cleartext
canonicalization** (CRLF / trailing-whitespace) rule that `apt`'s Sequoia verifier
enforces.
