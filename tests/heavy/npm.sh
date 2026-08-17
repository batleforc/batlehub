#!/usr/bin/env bash
# Heavy npm integration test — the npm CLI surface, against the real client.
#
# RFC 0009 §12.1 captured npm 11.17.0's paths by hand and §5.2 concluded that
# the capture, not the fixture table, is the check that finds this class of bug.
# This is that capture as a test.
#
# What it proves, in the order a JavaScript developer meets it:
#
#   1. `npm publish` puts a package into a **local** registry — one no public
#      registry has heard of — and `npm install` gets it back, tarball and all.
#      The tarball URL is one BatleHub wrote into the packument, so this also
#      pins that the rewrite points at the proxy rather than at npmjs.org.
#   2. `npm whoami` and `npm ping` answer as themselves. Both used to be
#      swallowed by the `{package}/{version}` catch-all and answered `200` with
#      a package document — `whoami` taken for package `-` at version `whoami`
#      (crates/web/src/handlers/proxy/npm/cli.rs).
#   3. `npm dist-tag ls` names the newest published version, and follows it when
#      a newer one is published. Dist-tags are derived from the version set, not
#      stored, so this is the derivation being right rather than a field echoing.
#   4. `npm search` finds the package it just published. §5.1's `must_find`
#      class, asked of the client instead of of the route: an endpoint that can
#      only be observed returning an empty list is indistinguishable from a stub.
#   5. `npm audit` gets a real answer on the path npm actually sends
#      (`/-/npm/v1/security/advisories/bulk`). §12.1 measured npm exiting 1 with
#      *"audit endpoint returned an error"* against the invented path this
#      server used to serve, with no fallback to the quick endpoint — so the
#      assertion is that npm never prints that, and that the JSON it got back
#      is an audit report.
#
# Run via `task test:npm-heavy` or directly. With `COVERAGE=1` the server runs
# under `cargo llvm-cov run --no-report` (see lib.sh).
#
# Environment knobs: DATABASE_URL (required), HEAVY_PORT (8082),
# HEAVY_TAP_PORT (8092), COVERAGE. Needs network: the proxy registry's upstream
# is registry.npmjs.org, which is the only way to exercise `npm audit` at all.

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

heavy_init npm 8082 8092
heavy_need npm "nodejs"
heavy_need python3 "python3"

LOCAL="npm-local-$HEAVY_RUN"
PROXY="npm-proxy-$HEAVY_RUN"
PKG="heavy-probe-$HEAVY_RUN"
# A dependency with a dependency of its own, so the proxy install resolves a
# graph rather than one document, and `npm audit` has something to ask about.
PROXY_DEP="is-odd@3.0.1"

heavy_start_server tests/heavy/config.npm.toml
heavy_start_tap

LOCAL_URL="$HEAVY_TAP_BASE/proxy/$LOCAL/"
PROXY_URL="$HEAVY_TAP_BASE/proxy/$PROXY/"

# npm keys auth by `//host/path/` — the same shape as the registry URL with the
# scheme removed. A token on the wrong key is silently no token at all, and
# publish then fails as anonymous.
NPMRC="$HEAVY_WORK/npmrc"
cat > "$NPMRC" <<EOF
registry=$LOCAL_URL
//127.0.0.1:$HEAVY_TAP_PORT/proxy/$LOCAL/:_authToken=$ADMIN_TOKEN
//127.0.0.1:$HEAVY_TAP_PORT/proxy/$PROXY/:_authToken=$ADMIN_TOKEN
EOF
export npm_config_userconfig="$NPMRC"
# Keep npm's own cache inside the run: a cached packument from a previous run
# is an answer this server did not give. The *publish* phase gets a cache of
# its own, and the consumer a different one, because `npm publish` writes the
# tarball it packed into cacache under its integrity hash — install it from the
# same cache and npm never asks for the tarball at all. Measured: the first
# version of this script asserted the tarball fetch and found no request in the
# transcript, having proved only that npm can read its own cache.
export npm_config_cache="$HEAVY_WORK/npm-cache-publish"
export npm_config_fund=false npm_config_audit=false npm_config_update_notifier=false

heavy_log "npm $(npm --version), node $(node --version)"

# ── 1. Publish into a registry nothing else has heard of ─────────────────────

make_package() {  # dir, version
  mkdir -p "$1"
  cat > "$1/package.json" <<EOF
{
  "name": "$PKG",
  "version": "$2",
  "description": "RFC 0009 heavy test probe",
  "license": "MIT",
  "main": "index.js"
}
EOF
  echo "module.exports = '$PKG';" > "$1/index.js"
}

make_package "$HEAVY_WORK/pkg" "1.0.0"
heavy_log "npm publish $PKG@1.0.0 -> $LOCAL"
(cd "$HEAVY_WORK/pkg" && npm publish --registry "$LOCAL_URL") \
  || heavy_fail "npm publish failed"
heavy_wire "PUT /proxy/$LOCAL/$PKG -> 200" "npm publish did not reach the publish route"

# ── 2. The CLI surface that is not a package read ────────────────────────────

heavy_log "npm ping"
npm ping --registry "$LOCAL_URL" || heavy_fail "npm ping failed"
heavy_wire "GET /proxy/$LOCAL/-/ping -> 200" "npm ping did not reach /-/ping"

heavy_log "npm whoami"
WHO="$(npm whoami --registry "$LOCAL_URL")" || heavy_fail "npm whoami failed"
[[ "$WHO" == "ci-admin" ]] \
  || heavy_fail "npm whoami printed '$WHO', expected the token's user_id 'ci-admin'"
heavy_wire "GET /proxy/$LOCAL/-/whoami -> 200" "npm whoami did not reach /-/whoami"

heavy_log "npm view"
VIEWED="$(npm view "$PKG" version --registry "$LOCAL_URL")" || heavy_fail "npm view failed"
[[ "$VIEWED" == "1.0.0" ]] || heavy_fail "npm view reported version '$VIEWED', expected 1.0.0"

# ── 3. dist-tags are derived, so publish a second version and look again ─────

heavy_log "npm dist-tag ls (one version published)"
npm dist-tag ls "$PKG" --registry "$LOCAL_URL" > "$HEAVY_WORK/tags-1.txt" \
  || heavy_fail "npm dist-tag ls failed"
grep -q "latest: 1.0.0" "$HEAVY_WORK/tags-1.txt" \
  || { cat "$HEAVY_WORK/tags-1.txt" >&2; heavy_fail "dist-tag ls did not name 1.0.0 as latest"; }
heavy_wire "GET /proxy/$LOCAL/-/package/$PKG/dist-tags -> 200" \
  "npm dist-tag ls did not reach the dist-tags route"

make_package "$HEAVY_WORK/pkg2" "2.0.0"
heavy_log "npm publish $PKG@2.0.0, then dist-tag ls again"
(cd "$HEAVY_WORK/pkg2" && npm publish --registry "$LOCAL_URL") \
  || heavy_fail "publishing 2.0.0 failed"
heavy_mark "after-2.0.0"
npm dist-tag ls "$PKG" --registry "$LOCAL_URL" > "$HEAVY_WORK/tags-2.txt" \
  || heavy_fail "npm dist-tag ls failed after the second publish"
grep -q "latest: 2.0.0" "$HEAVY_WORK/tags-2.txt" \
  || { cat "$HEAVY_WORK/tags-2.txt" >&2; heavy_fail "latest did not move to 2.0.0 — dist-tags are derived from the version set, so this is the derivation, not a stored field"; }

# ── 4. Search must find what we just published ───────────────────────────────
#
# The one assertion class that separates an implemented collection endpoint from
# one stubbed to `200` with an empty list (RFC 0009 §5.1) — asked of npm rather
# than of the route, because npm is what decides whether the shape is usable.

heavy_log "npm search $PKG"
npm search "$PKG" --json --registry "$LOCAL_URL" > "$HEAVY_WORK/search.json" \
  || heavy_fail "npm search failed"
python3 - "$HEAVY_WORK/search.json" "$PKG" <<'PY' || heavy_fail "npm search did not find the package published one command earlier"
import json, sys
hits = json.load(open(sys.argv[1]))
names = [h.get("name") for h in hits]
sys.exit(0 if sys.argv[2] in names else 1)
PY
heavy_wire "GET /proxy/$LOCAL/-/v1/search" "npm search did not reach /-/v1/search"

# ── 5. Install it back, from a clean project ─────────────────────────────────

heavy_mark "local-install"
export npm_config_cache="$HEAVY_WORK/npm-cache-consumer"
mkdir -p "$HEAVY_WORK/consumer"
cat > "$HEAVY_WORK/consumer/package.json" <<EOF
{ "name": "consumer", "version": "1.0.0", "private": true }
EOF
heavy_log "npm install $PKG@1.0.0 from the local registry"
(cd "$HEAVY_WORK/consumer" && npm install "$PKG@1.0.0" --registry "$LOCAL_URL") \
  || heavy_fail "npm install from the local registry failed"
[[ -f "$HEAVY_WORK/consumer/node_modules/$PKG/index.js" ]] \
  || heavy_fail "$PKG was not unpacked into node_modules"
# The tarball URL came out of the packument BatleHub rendered. A request for it
# arriving here is that rewrite being right; an unrewritten one would have gone
# to npmjs.org, where this package does not exist.
heavy_wire_after "local-install" "GET /proxy/$LOCAL/$PKG/1.0.0/tarball -> 200" \
  "the tarball was not fetched through the proxy — dist.tarball may point upstream"
heavy_log "NPM-LOCAL-OK (publish, install, whoami, ping, dist-tags, search)"

# ── 6. npm audit, on the path npm actually sends ─────────────────────────────

heavy_mark "proxy-install"
mkdir -p "$HEAVY_WORK/audited"
cat > "$HEAVY_WORK/audited/package.json" <<EOF
{ "name": "audited", "version": "1.0.0", "private": true }
EOF
heavy_log "npm install $PROXY_DEP through the proxy registry"
(cd "$HEAVY_WORK/audited" && npm install "$PROXY_DEP" --registry "$PROXY_URL") \
  || heavy_fail "npm install through the proxy registry failed"
heavy_wire_after "proxy-install" "GET /proxy/$PROXY/is-odd -> 200" \
  "the packument was not fetched through the proxy"

heavy_mark "audit"
heavy_log "npm audit"
set +e
(cd "$HEAVY_WORK/audited" && npm audit --json --registry "$PROXY_URL") \
  >"$HEAVY_WORK/audit.json" 2>"$HEAVY_WORK/audit.err"
AUDIT_RC=$?
set -e
# npm exits 1 when it finds vulnerabilities, which is a working audit, and 1
# when the endpoint errors, which is the bug §12.1 measured. The exit code
# cannot tell them apart; what it printed can.
if grep -q "audit endpoint returned an error" "$HEAVY_WORK/audit.err"; then
  cat "$HEAVY_WORK/audit.err" >&2
  heavy_fail "npm reported the audit endpoint as failing — the exact §12.1 signature"
fi
python3 - "$HEAVY_WORK/audit.json" <<'PY' || { cat "$HEAVY_WORK/audit.err" >&2; heavy_fail "npm audit produced no report (rc=$AUDIT_RC)"; }
import json, sys
report = json.load(open(sys.argv[1]))
# An audit report always carries these two, whether or not anything is wrong.
sys.exit(0 if "vulnerabilities" in report and "metadata" in report else 1)
PY
heavy_wire_after "audit" "POST /proxy/$PROXY/-/npm/v1/security/advisories/bulk -> 200" \
  "npm audit did not reach the bulk advisory path, or it did not answer 200"
# The invented paths are still served as deprecated aliases, and npm must not be
# the reason they are: an assertion that only the real path is exercised is what
# keeps the aliases removable.
heavy_wire_not "POST /proxy/$PROXY/-/npm/v1/audit/bulk" \
  "npm used the legacy alias — the real path must be the one it selects"
heavy_log "NPM-AUDIT-OK (advisories/bulk answered, report parsed)"

heavy_done NPM-HEAVY-OK
