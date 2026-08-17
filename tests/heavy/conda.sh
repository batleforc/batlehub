#!/usr/bin/env bash
# Heavy conda integration test — micromamba, against a local channel.
#
# Two defects, both shipped, both invisible to every test in the repository:
#
#   1. **`HEAD` never reached the handler** (RFC 0009 §12.4). conda probes
#      `repodata.json.zst` with `HEAD`, and actix rejects a `HEAD` at the
#      method guard of a `GET`-only route *before the handler runs* — so a real
#      client concluded the compressed document did not exist and fell back to
#      the plain one, exactly as before phase 3 added it. `curl -X GET` served
#      it perfectly throughout, which is why every test passed. This is the
#      sharpest finding of the whole RFC: the path was right, the handler was
#      right, and the request method never got there.
#   2. **The compressed channel could not see a publish** (§12.13). It was
#      cached under the blocked-set fingerprint with no expiry, and a publish
#      does not move the blocked set — so in local mode the `.zst` was pinned to
#      whatever the channel held the first time anyone asked, while
#      `repodata.json` was regenerated per request and correct. The two
#      encodings described different channels, and micromamba asks for the
#      compressed one first.
#
# The second only reproduces if the channel is **warmed before the publish**:
# without a read in between there is nothing stale, and the test passes against
# the bug. That ordering is the test.
#
# Run via `task test:conda-heavy` or directly. Needs network for the micromamba
# download (cached in HEAVY_CACHE afterwards).
#
# Environment knobs: DATABASE_URL (required), HEAVY_PORT (8085),
# HEAVY_TAP_PORT (8095), COVERAGE, MICROMAMBA_VERSION (2.9.0).

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

heavy_init conda 8085 8095
heavy_need python3 "python3"
heavy_need curl "curl"

MICROMAMBA_VERSION="${MICROMAMBA_VERSION:-2.9.0}"
REGISTRY="conda-local-$HEAVY_RUN"
# The subdir the packages are published into. conda always fetches `noarch`
# alongside it, so the channel answers two repodata documents either way.
SUBDIR="${CONDA_SUBDIR:-linux-64}"
PKG_A="heavy-probe-a-$HEAVY_RUN"
PKG_B="heavy-probe-b-$HEAVY_RUN"

MM_DIR="$(heavy_cached_dir "micromamba-$MICROMAMBA_VERSION" \
  "https://micro.mamba.pm/api/micromamba/linux-64/$MICROMAMBA_VERSION" tar.bz2)"
MM="$MM_DIR/bin/micromamba"
[[ -x "$MM" ]] || heavy_fail "micromamba was not where the archive was expected to put it ($MM)"

heavy_start_server tests/heavy/config.conda.toml
heavy_start_tap

CHANNEL="$HEAVY_TAP_BASE/proxy/$REGISTRY"

heavy_log "micromamba $("$MM" --version)"

# ── 1. Two packages, published one at a time ─────────────────────────────────

build_pkg() {  # name -> echoes the file path
  local name="$1"
  local out="$HEAVY_WORK/$name-1.0.0-0.tar.bz2"
  python3 tests/heavy/make_conda_package.py "$out" "$name" 1.0.0 0 "$SUBDIR" >&2 \
    || heavy_fail "building the conda package failed"
  echo "$out"
}

publish_pkg() {  # file
  curl -fsS -o /dev/null -X POST \
    -H "Authorization: Bearer $ADMIN_TOKEN" \
    --data-binary @"$1" \
    "$CHANNEL/$SUBDIR/" || heavy_fail "publishing $(basename "$1") failed"
}

PKG_A_FILE="$(build_pkg "$PKG_A")"
PKG_B_FILE="$(build_pkg "$PKG_B")"

heavy_log "Publishing $PKG_A"
publish_pkg "$PKG_A_FILE"

# ── 2. Install it — and watch how micromamba asks ────────────────────────────

# Each call gets its own root prefix, and that is the point. micromamba caches
# repodata under `$MAMBA_ROOT_PREFIX/pkgs/cache` and, on a second resolve
# minutes later, does not re-fetch it — the first version of this script asked
# the same client twice and measured its cache rather than the server's. A
# fresh root is the second developer, on the second machine, resolving against
# a channel BatleHub has already been asked about.
create_env() {  # root-suffix, env-name, package
  local root="$HEAVY_WORK/mamba-root-$1"
  mkdir -p "$root"
  MAMBA_ROOT_PREFIX="$root" "$MM" create -y --no-rc --override-channels \
    -c "$CHANNEL" --platform "$SUBDIR" -n "$2" "$3"
}

heavy_mark "first-create"
heavy_log "micromamba create (cold channel)"
create_env one probe-a "$PKG_A" >"$HEAVY_WORK/create-a.log" 2>&1 \
  || { tail -40 "$HEAVY_WORK/create-a.log" >&2; heavy_fail "micromamba could not install $PKG_A"; }
[[ -d "$HEAVY_WORK/mamba-root-one/envs/probe-a" ]] || heavy_fail "no environment was created"

# The probe, and the answer to it. `HEAD` is what conda sends; a `GET`-only
# route answers it `404` without the handler ever running.
heavy_wire_after "first-create" "HEAD /proxy/$REGISTRY/$SUBDIR/repodata.json.zst -> 200" \
  "micromamba's HEAD probe for the compressed repodata did not get a 200 — the method guard is where this fails, not the handler (RFC 0009 §12.4)"
heavy_log "CONDA-ZST-HEAD-OK (the compressed index answers the probe conda actually sends)"

# ── 3. Publish into a warmed channel ─────────────────────────────────────────
#
# The read above is what makes this meaningful: it put the compressed channel
# in the cache. A publish does not change the blocked set, and the entry had no
# expiry, so this is the step that used to fail — with the plain `repodata.json`
# showing the package present the whole time.

heavy_log "Publishing $PKG_B into the now-warm channel"
publish_pkg "$PKG_B_FILE"

heavy_mark "second-create"
heavy_log "micromamba create for the package published after the warm-up"
create_env two probe-b "$PKG_B" >"$HEAVY_WORK/create-b.log" 2>&1 \
  || {
    tail -40 "$HEAVY_WORK/create-b.log" >&2
    heavy_fail "$PKG_B is not installable — the compressed channel is pinned to its pre-publish contents (RFC 0009 §12.13)"
  }
[[ -d "$HEAVY_WORK/mamba-root-two/envs/probe-b" ]] || heavy_fail "no environment was created for $PKG_B"
heavy_wire_after "second-create" "$SUBDIR/repodata.json.zst" \
  "the second resolve did not consult the compressed index at all"
heavy_log "CONDA-PUBLISH-VISIBLE-OK ($PKG_B resolved from a channel that had already been read)"

heavy_done CONDA-HEAVY-OK
