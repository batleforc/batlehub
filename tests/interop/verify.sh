#!/usr/bin/env bash
# apt/dnf interop verification for BatleHub's Deb/RPM repository hosting.
#
# Generates signed APT + RPM repositories using the PRODUCTION signing and
# index-generation code (the `repo_interop` generator in batlehub-adapters), then
# points **real** `apt` and `dnf` (in throwaway containers) at them over `file://`
# to confirm they accept BatleHub's hand-rolled Ed25519 OpenPGP signatures and
# generated metadata — end to end, including a full package install.
#
# Run via `task test:repo-interop` or directly. Requires podman or docker.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
ENGINE="${CONTAINER_ENGINE:-}"
if [[ -z "$ENGINE" ]]; then
  if command -v podman >/dev/null 2>&1; then ENGINE=podman
  elif command -v docker >/dev/null 2>&1; then ENGINE=docker
  else echo "ERROR: need podman or docker" >&2; exit 1; fi
fi

DEBIAN_IMAGE="${DEBIAN_IMAGE:-docker.io/library/debian:stable-slim}"
FEDORA_IMAGE="${FEDORA_IMAGE:-docker.io/library/fedora:41}"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
OUT="$WORK/repos"
mkdir -p "$OUT"

echo "==> Generating signed repos with the production signing/index code"
REPO_INTEROP_OUT="$OUT" cargo test -p batlehub-adapters --test repo_interop -- \
  --ignored --exact generate_signed_repos --nocapture

# Mount flag: add SELinux relabel (:Z) when the engine is podman on an SELinux host.
MNT_SUFFIX="ro"
if [[ "$ENGINE" == "podman" ]]; then MNT_SUFFIX="ro,Z"; fi

echo "==> Verifying APT accepts the signed repo (update + install)"
"$ENGINE" run --rm --user 0 -v "$OUT/apt:/srv/apt:$MNT_SUFFIX" "$DEBIAN_IMAGE" bash -c '
  set -e
  mkdir -p /tmp/lists/partial /tmp/cache/archives/partial /tmp/aptetc
  echo "deb [signed-by=/srv/apt/key.asc] file:/srv/apt stable main" > /tmp/b.list
  APTOPT="-o Dir::Etc::SourceList=/tmp/b.list -o Dir::Etc::SourceParts=/tmp/aptetc \
          -o Dir::State::Lists=/tmp/lists -o Dir::Cache=/tmp/cache"
  apt-get $APTOPT update
  apt-get $APTOPT install -y --no-install-recommends hello-batlehub
  test -f /usr/share/doc/hello-batlehub/README
  echo "APT-INTEROP-OK"
'

echo "==> Verifying DNF accepts the signed repodata (makecache + install)"
"$ENGINE" run --rm --user 0 -e HOME=/tmp -v "$OUT/yum:/srv/yum:$MNT_SUFFIX" "$FEDORA_IMAGE" bash -c '
  set -e
  mkdir -p /tmp/repos /tmp/dnfcache
  printf "[batlehub]\nname=batlehub\nbaseurl=file:///srv/yum\nenabled=1\nrepo_gpgcheck=1\ngpgcheck=0\ngpgkey=file:///srv/yum/repodata/repomd.xml.key\n" \
    > /tmp/repos/batlehub.repo
  DNFOPT="--setopt=reposdir=/tmp/repos --setopt=cachedir=/tmp/dnfcache --disablerepo=* --enablerepo=batlehub"
  dnf -y $DNFOPT makecache
  dnf -y $DNFOPT install hello-batlehub
  test -f /usr/share/hello-batlehub/data.txt
  echo "DNF-INTEROP-OK"
'

echo "==> apt + dnf interop PASSED"
