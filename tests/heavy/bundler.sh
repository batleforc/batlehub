#!/usr/bin/env bash
# Heavy Bundler integration test — the RubyGems compact index, against the real
# client.
#
# What it proves, in the order a Ruby developer meets it:
#
#   1. Starts a BatleHub server (Postgres required, `DATABASE_URL` env) with one
#      local-mode `rubygems` registry — see config.bundler.toml — behind a
#      logging tap, so the client's request sequence is observable.
#   2. Builds two gems with real `gem build`, the second depending on the first,
#      and publishes them through the registry's publish endpoint.
#   3. `bundle install` resolves and installs the first gem — proving a gem
#      published to a **local** registry is visible to Bundler at all, which it
#      was not before RFC 0009 §12.15: all three compact documents were served
#      from upstream in every mode.
#   4. After the second gem is published, `bundle install` again. Bundler asks
#      for the tail of the document it cached (`Range` + `If-None-Match`) and
#      must get a `206` it *accepts* — the assertion is that no full re-fetch of
#      `/versions` follows it, because a `206` Bundler rejects is a re-fetch and
#      costs more than the `200` it replaced (RFC 0009 §13.24).
#   5. `bundle update` with nothing changed upstream must get a `304`.
#
# Steps 4 and 5 are what a unit test cannot reach: `range.rs` decides what to
# answer, and only a client decides whether the answer was usable.
#
# Run via `task test:bundler-heavy` or directly. With `COVERAGE=1` the server
# runs under `cargo llvm-cov run --no-report`, so its execution counts toward
# the merged CI coverage report (the caller runs `cargo llvm-cov report`
# afterwards; the server must exit cleanly for the profile data to flush —
# SIGTERM triggers actix's graceful shutdown).
#
# Environment knobs:
#   DATABASE_URL     (required)  Postgres for the server
#   HEAVY_PORT       (default 8081)  the server's own port
#   HEAVY_TAP_PORT   (default 8091)  what Bundler is pointed at
#   BUNDLER_VERSION  (default 4.0.17) installed with `gem install` if absent
#   RUBY_VERSION     (default 3.3)   only used to pick a `mise` toolchain
#   COVERAGE=1       instrument the server with cargo-llvm-cov
#
# The port knobs are prefixed rather than named `PORT`: development containers
# and PaaS runtimes commonly export `PORT` for their own service, and inheriting
# one pointed this script's health check at an unrelated process that never
# became healthy — the same hazard as the `.env` inheritance note in CLAUDE.md's
# frontend section.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

HEAVY_PORT="${HEAVY_PORT:-8081}"
HEAVY_TAP_PORT="${HEAVY_TAP_PORT:-8091}"
ADMIN_TOKEN="${ADMIN_TOKEN:-heavy-admin-token}"
BUNDLER_VERSION="${BUNDLER_VERSION:-4.0.17}"
RUBY_VERSION="${RUBY_VERSION:-3.3}"
COVERAGE="${COVERAGE:-0}"

: "${DATABASE_URL:?DATABASE_URL must point at a reachable Postgres}"

WORK="$(mktemp -d)"
SERVER_PID=""
TAP_PID=""

# A fresh registry and fresh gem names per run: the database persists, and a
# leftover gem sorting after the one published mid-run turns step 4's append
# into a middle-insertion, which is answered `200` on purpose. See
# config.bundler.toml.
RUN="$(date +%H%M%S)"
REGISTRY="gems-$RUN"
GEM_A="aa${RUN}probe"
GEM_B="zz${RUN}probe"

BASE="http://127.0.0.1:$HEAVY_PORT"
SOURCE="http://127.0.0.1:$HEAVY_TAP_PORT/proxy/$REGISTRY"
LOG="$WORK/tap.log"

stop_server() {
  if [[ -n "$SERVER_PID" ]]; then
    # SIGTERM the whole process group: $SERVER_PID is the `cargo llvm-cov` /
    # `cargo run` wrapper and the server binary is a grandchild. cargo does not
    # forward signals, so signalling the wrapper alone strands the server — no
    # graceful shutdown, no llvm profile flush. `setsid` at launch made
    # $SERVER_PID the process-group id.
    kill -TERM -- "-$SERVER_PID" 2>/dev/null || kill -TERM "$SERVER_PID" 2>/dev/null || true
    wait "$SERVER_PID" 2>/dev/null || true
    for _ in $(seq 1 60); do
      pgrep -g "$SERVER_PID" >/dev/null 2>&1 || break
      sleep 1
    done
  fi
  SERVER_PID=""
}

cleanup() {
  [[ -n "$TAP_PID" ]] && kill "$TAP_PID" 2>/dev/null
  stop_server
  rm -rf "$WORK"
}
trap cleanup EXIT

log() { printf '\n==> %s\n' "$*"; }
fail() { echo "ERROR: $*" >&2; echo "── wire transcript ──" >&2; cat "$LOG" >&2; exit 1; }

# ── 0. Ruby toolchain ────────────────────────────────────────────────────────
#
# `mise` pins tool versions per directory in this repo, so `ruby` on PATH may be
# a shim that exits non-zero because no version is set for this directory —
# `command -v` finds it and running it fails. Probe by *running* it, and fall
# back to asking mise for a toolchain rather than failing on a workstation that
# has one.
RB=()
if ruby -v >/dev/null 2>&1; then
  :
elif command -v mise >/dev/null 2>&1 && mise x "ruby@$RUBY_VERSION" -- ruby -v >/dev/null 2>&1; then
  RB=(mise x "ruby@$RUBY_VERSION" --)
else
  echo "ERROR: no working ruby (and no mise toolchain for ruby@$RUBY_VERSION)" >&2
  exit 1
fi
ruby_run() { "${RB[@]}" "$@"; }

log "Ruby: $(ruby_run ruby -v)"
if ! ruby_run gem list -i bundler -v "$BUNDLER_VERSION" >/dev/null 2>&1; then
  log "Installing bundler $BUNDLER_VERSION"
  ruby_run gem install bundler -v "$BUNDLER_VERSION" --no-document >/dev/null
fi
BUNDLE=(bundle "_${BUNDLER_VERSION}_")

# ── 1. Start the server and the tap ──────────────────────────────────────────

sed -e "s/^name = \"gems\"$/name = \"$REGISTRY\"/" \
    -e "s/^port = 8081$/port = $HEAVY_PORT/" \
  tests/heavy/config.bundler.toml > "$WORK/config.toml"

log "Starting BatleHub server (coverage=$COVERAGE, registry=$REGISTRY)"
if [[ "$COVERAGE" == "1" ]]; then
  setsid cargo llvm-cov run --no-report -p batlehub-server -- \
    --config "$WORK/config.toml" >"$WORK/server.log" 2>&1 &
else
  setsid cargo run -p batlehub-server -- \
    --config "$WORK/config.toml" >"$WORK/server.log" 2>&1 &
fi
SERVER_PID=$!

for i in $(seq 1 180); do
  if curl -sf "$BASE/healthz" >/dev/null 2>&1; then break; fi
  if ! kill -0 "$SERVER_PID" 2>/dev/null; then
    echo "ERROR: server exited early; log follows" >&2
    cat "$WORK/server.log" >&2
    exit 1
  fi
  sleep 2
  if [[ "$i" == 180 ]]; then
    echo "ERROR: server did not become healthy; log follows" >&2
    cat "$WORK/server.log" >&2
    exit 1
  fi
done
log "Server healthy at $BASE"

python3 tests/heavy/http_tap.py "$LOG" "$HEAVY_TAP_PORT" "$HEAVY_PORT" >"$WORK/tap.err" 2>&1 &
TAP_PID=$!
for _ in $(seq 1 30); do
  curl -sf -o /dev/null "http://127.0.0.1:$HEAVY_TAP_PORT/healthz" && break
  sleep 1
done
log "Tap listening on $HEAVY_TAP_PORT, Bundler will use $SOURCE"

# ── 2. Build two gems, the second depending on the first ─────────────────────

build_gem() {  # name, dependency line
  local name=$1 dep=${2:-}
  mkdir -p "$WORK/gems/$name/lib"
  echo "module Probe; end" > "$WORK/gems/$name/lib/$name.rb"
  cat > "$WORK/gems/$name/$name.gemspec" <<EOF
Gem::Specification.new do |s|
  s.name        = "$name"
  s.version     = "1.0.0"
  s.summary     = "compact index probe"
  s.authors     = ["batlehub heavy tests"]
  s.files       = ["lib/$name.rb"]
  $dep
end
EOF
  (cd "$WORK/gems/$name" && ruby_run gem build "$name.gemspec" >/dev/null)
}

log "Building $GEM_A and $GEM_B (which depends on it)"
build_gem "$GEM_A"
build_gem "$GEM_B" "s.add_dependency \"$GEM_A\", \">= 1.0.0\""

publish() {
  curl -fsS -o /dev/null -X POST \
    -H "Authorization: Bearer $ADMIN_TOKEN" \
    --data-binary @"$WORK/gems/$1/$1-1.0.0.gem" \
    "$SOURCE/api/v1/gems" || fail "publishing $1 failed"
}

# ── 3. First install: is a locally published gem visible to Bundler at all ───

export BUNDLE_USER_HOME="$WORK/bundle-home"
mkdir -p "$BUNDLE_USER_HOME" "$WORK/proj"

log "Publishing $GEM_A"
publish "$GEM_A"

cat > "$WORK/proj/Gemfile" <<EOF
source "$SOURCE"
gem "$GEM_A"
EOF

cd "$WORK/proj"
"${RB[@]}" "${BUNDLE[@]}" config set --local path "$WORK/proj/vendor" >/dev/null

log "bundle install (cold cache)"
"${RB[@]}" "${BUNDLE[@]}" install || fail "the first bundle install failed"
grep -q "$GEM_A" Gemfile.lock || fail "$GEM_A missing from Gemfile.lock"
log "COMPACT-INDEX-LOCAL-OK ($GEM_A resolved from a local registry)"

# ── 4. Second install: the incremental fetch ─────────────────────────────────

log "Publishing $GEM_B, then bundle install again"
publish "$GEM_B"
cat > "$WORK/proj/Gemfile" <<EOF
source "$SOURCE"
gem "$GEM_A"
gem "$GEM_B"
EOF

: > "$WORK/mark-step4"
"${RB[@]}" "${BUNDLE[@]}" install || fail "the second bundle install failed"

# The dependency edge proves the gemspec's runtime dependencies survived the
# publish and reached `/info/{gem}` — an empty list resolves to a gem installed
# without the gems it needs (RFC 0009 §12.15).
grep -q "$GEM_B" Gemfile.lock || fail "$GEM_B missing from Gemfile.lock"
grep -A 2 "$GEM_B (1.0.0)" Gemfile.lock | grep -q "$GEM_A" \
  || { cat Gemfile.lock >&2; fail "$GEM_B's dependency on $GEM_A is missing from the lock"; }

# The conditional request, and the answer to it.
grep -q "GET /proxy/$REGISTRY/versions -> 206" "$LOG" \
  || fail "no 206 for /versions — Bundler asked for a range and got the whole document"
grep "GET /proxy/$REGISTRY/versions -> 206" "$LOG" | grep -q "If-None-Match" \
  || fail "the 206 was served without a validator being presented"

# The assertion that matters: a 206 Bundler could not use shows up as a full
# re-fetch immediately after it. Its absence is the client saying it verified
# the digest and appended.
#
# The verdict is carried to END rather than `exit 0` from the rule: `exit` in a
# rule still runs the END block, so an `END{exit 1}` there overrides it and the
# check can never fail — which is how the first version of this assertion passed
# against a transcript that plainly showed the re-fetch.
if awk -v reg="$REGISTRY" '
    $0 ~ ("GET /proxy/" reg "/versions -> 206") { seen = 1; next }
    seen && $0 ~ ("GET /proxy/" reg "/versions -> 200") { refetch = 1 }
    END { exit refetch ? 0 : 1 }' "$LOG"; then
  fail "Bundler re-fetched /versions in full after the 206 — it rejected the partial answer"
fi
log "COMPACT-INDEX-INCREMENTAL-OK (206 accepted, no re-fetch)"

# ── 5. Nothing changed: the cheap answer ─────────────────────────────────────

log "bundle update with nothing published in between"
"${RB[@]}" "${BUNDLE[@]}" update || fail "bundle update failed"
grep -q "GET /proxy/$REGISTRY/versions -> 304" "$LOG" \
  || fail "no 304 for /versions — a current copy was sent the whole document again"
log "COMPACT-INDEX-CONDITIONAL-OK (304 for a current copy)"

log "Wire transcript"
cat "$LOG"
log "BUNDLER-HEAVY-OK"
