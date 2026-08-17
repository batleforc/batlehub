#!/usr/bin/env bash
# Heavy Composer integration test — `composer`, against the real client.
#
# Composer produced the largest shipped bug of RFC 0009's verification and one
# of its subtlest, and both are about what the *repository document* claims
# rather than about any route:
#
#   1. **`available-packages` is a claim of completeness** (§12.10). Composer
#      treats it as authoritative: a package absent from the list is never
#      requested, whatever `metadata-url` would have answered. BatleHub sent it
#      in every mode — `[]` in proxy mode, meaning "this repository is empty".
#      Measured against Composer 2.10.2, a `composer update` fetched
#      `packages.json`, stopped, and reported *"could not be found in any
#      version"* without ever requesting `p2/`. **A proxy-mode Composer registry
#      could not resolve anything at all**, which is the entire purpose of the
#      mode — and a test was pinning the field:
#      `assert_eq!(body["available-packages"], json!([]))`.
#   2. **Every endpoint but `packages.json` is discovered from a URL template**
#      in that document (§12.5). BatleHub advertised `metadata-url` and
#      `available-packages` only, so `composer search` answered from its cached
#      package list and made no request, and `list.json` was equally
#      unreachable. Phase 6's search route and phase 7's list route both shipped
#      correct and undiscoverable.
#
# So this test disables Packagist explicitly. Composer adds `packagist.org` to
# every project implicitly, and a search that BatleHub never answered still
# returns results — the "green from something other than the thing under test"
# hazard that §12.10 had to get right before its transcript meant anything.
#
# Run via `task test:composer-heavy` or directly. Needs network: the proxy
# repository's upstream is repo.packagist.org, and a static PHP is downloaded
# when the machine has none.
#
# Environment knobs: DATABASE_URL (required), HEAVY_PORT (8088),
# HEAVY_TAP_PORT (8098), COVERAGE, COMPOSER_VERSION (2.10.2),
# STATIC_PHP_VERSION (8.3.28), HEAVY_COMPOSER_PROBE (monolog/monolog).

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

heavy_init composer 8088 8098
heavy_need python3 "python3"
heavy_need curl "curl"

COMPOSER_VERSION="${COMPOSER_VERSION:-2.10.2}"
STATIC_PHP_VERSION="${STATIC_PHP_VERSION:-8.3.28}"
PROBE_PACKAGE="${HEAVY_COMPOSER_PROBE:-monolog/monolog}"

REGISTRY_LOCAL="composer-local-$HEAVY_RUN"
REGISTRY_PROXY="composer-proxy-$HEAVY_RUN"
# Composer names are `vendor/package`, lowercase, and the vendor cannot be a
# bare digit string — hence the prefix on the run id.
PKG="heavyprobe/p$HEAVY_RUN"

# ── 0. A PHP and a composer.phar ─────────────────────────────────────────────
#
# The runner image usually has both. `mise`'s PHP backends compile from source
# and need autoconf/bison/re2c, which is not something a test may assume, so
# the fallback is the static build RFC 0009 §12.5 used.

if php --version >/dev/null 2>&1; then
  PHP="$(command -v php)"
else
  PHP_DIR="$(heavy_cached_dir "static-php-$STATIC_PHP_VERSION" \
    "https://dl.static-php.dev/static-php-cli/common/php-$STATIC_PHP_VERSION-cli-linux-x86_64.tar.gz" tar.gz)"
  PHP="$PHP_DIR/php"
  [[ -x "$PHP" ]] || heavy_fail "the static PHP archive did not contain a php binary at $PHP"
fi

if command -v composer >/dev/null 2>&1 && composer --version >/dev/null 2>&1; then
  COMPOSER=(composer)
else
  PHAR="$HEAVY_CACHE/composer-$COMPOSER_VERSION.phar"
  if [[ ! -f "$PHAR" ]]; then
    heavy_log "Downloading composer $COMPOSER_VERSION"
    mkdir -p "$HEAVY_CACHE"
    curl -fsSL --proto '=https' --proto-redir '=https' \
      -o "$PHAR" "https://getcomposer.org/download/$COMPOSER_VERSION/composer.phar" \
      || heavy_fail "could not download composer.phar $COMPOSER_VERSION"
  fi
  COMPOSER=("$PHP" "$PHAR")
fi

export COMPOSER_HOME="$HEAVY_WORK/composer-home"
export COMPOSER_CACHE_DIR="$HEAVY_WORK/composer-cache"
export COMPOSER_NO_INTERACTION=1
mkdir -p "$COMPOSER_HOME" "$COMPOSER_CACHE_DIR"

heavy_start_server tests/heavy/config.composer.toml
heavy_start_tap

LOCAL_URL="$HEAVY_TAP_BASE/proxy/$REGISTRY_LOCAL"
PROXY_URL="$HEAVY_TAP_BASE/proxy/$REGISTRY_PROXY"

heavy_log "php $("$PHP" -r 'echo PHP_VERSION;'), composer $("${COMPOSER[@]}" --version 2>/dev/null | head -1)"

# ── 1. Publish into the local repository ─────────────────────────────────────

ZIP="$HEAVY_WORK/probe.zip"
python3 tests/heavy/make_composer_zip.py "$ZIP" "$PKG" 1.0.0 \
  || heavy_fail "building the composer zip failed"

heavy_mark "publish"
heavy_log "Publishing $PKG to $REGISTRY_LOCAL"
curl -fsS -o /dev/null -X POST \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  --data-binary @"$ZIP" \
  "$LOCAL_URL/api/upload" || heavy_fail "publishing failed"

# ── 2. composer require, against the local repository ────────────────────────
#
# `secure-http: false` because this instance is plain HTTP; Composer refuses
# one otherwise (§12.5 — the third of the clients that require TLS, after
# Terraform, which has no opt-out, and NuGet).
# `packagist.org: false` because Composer adds it implicitly, and a resolve it
# satisfies is a resolve BatleHub was not asked for.

PROJECT="$HEAVY_WORK/project"
mkdir -p "$PROJECT"
cat > "$PROJECT/composer.json" <<EOF
{
  "name": "heavyprobe/consumer",
  "description": "RFC 0009 heavy test consumer",
  "config": { "secure-http": false },
  "repositories": [
    { "type": "composer", "url": "$LOCAL_URL" },
    { "packagist.org": false }
  ],
  "require": { "$PKG": "1.0.0" }
}
EOF

heavy_mark "local-install"
heavy_log "composer update (local repository, Packagist disabled)"
(cd "$PROJECT" && "${COMPOSER[@]}" update --no-progress) >"$HEAVY_WORK/update-local.log" 2>&1 \
  || { tail -30 "$HEAVY_WORK/update-local.log" >&2; heavy_fail "composer update against the local repository failed"; }

[[ -f "$PROJECT/vendor/$PKG/composer.json" ]] \
  || heavy_fail "$PKG was not installed into vendor/"
"$PHP" -r "require '$PROJECT/vendor/autoload.php'; exit(\HeavyProbe\Probe::NAME === '$PKG' ? 0 : 1);" \
  || heavy_fail "the installed package's class does not load — the zip layout reached vendor/ wrong"

heavy_wire_after "local-install" "GET /proxy/$REGISTRY_LOCAL/packages.json -> 200" \
  "composer did not read packages.json"
heavy_wire_after "local-install" "GET /proxy/$REGISTRY_LOCAL/p2/$PKG.json -> 200" \
  "composer did not read the p2 metadata document"
heavy_log "COMPOSER-LOCAL-OK ($PKG resolved, installed and autoloadable)"

# ── 3. The mode that could not resolve anything ──────────────────────────────

PROXIED="$HEAVY_WORK/proxied"
mkdir -p "$PROXIED"
cat > "$PROXIED/composer.json" <<EOF
{
  "name": "heavyprobe/proxied",
  "description": "RFC 0009 heavy test proxy consumer",
  "config": { "secure-http": false },
  "repositories": [
    { "type": "composer", "url": "$PROXY_URL" },
    { "packagist.org": false }
  ],
  "require": { "$PROBE_PACKAGE": "*" }
}
EOF

heavy_mark "proxy-install"
heavy_log "composer update (proxy repository, Packagist disabled)"
set +e
(cd "$PROXIED" && "${COMPOSER[@]}" update --no-progress) >"$HEAVY_WORK/update-proxy.log" 2>&1
PROXY_RC=$?
set -e
if [[ $PROXY_RC -ne 0 ]]; then
  tail -30 "$HEAVY_WORK/update-proxy.log" >&2
  if grep -q "could not be found in any version" "$HEAVY_WORK/update-proxy.log"; then
    heavy_fail "Composer stopped at packages.json — 'available-packages' is claiming this proxy repository is empty (RFC 0009 §12.10)"
  fi
  heavy_fail "composer update against the proxy repository failed"
fi

grep -q "$PROBE_PACKAGE" "$PROXIED/composer.lock" \
  || heavy_fail "$PROBE_PACKAGE is missing from the lock file"
heavy_wire_after "proxy-install" "GET /proxy/$REGISTRY_PROXY/p2/$PROBE_PACKAGE.json -> 200" \
  "the p2 document was never requested — Composer decided the repository does not hold this package"
heavy_log "COMPOSER-PROXY-OK ($PROBE_PACKAGE resolved through a proxy repository)"

# ── 4. Search has to be discoverable, not merely implemented ─────────────────

heavy_mark "search"
heavy_log "composer search $PROBE_PACKAGE"
set +e
(cd "$PROXIED" && "${COMPOSER[@]}" search --format json "${PROBE_PACKAGE##*/}") \
  >"$HEAVY_WORK/search.json" 2>"$HEAVY_WORK/search.err"
SEARCH_RC=$?
set -e
[[ $SEARCH_RC -eq 0 ]] || { cat "$HEAVY_WORK/search.err" >&2; heavy_fail "composer search failed"; }

# The request is the assertion. A search that answers from Composer's own cached
# package list looks identical in the output and never touches the server, which
# is exactly how the unreachable `search.json` route went unnoticed.
heavy_wire_after "search" "GET /proxy/$REGISTRY_PROXY/search.json" \
  "composer search made no request — the 'search' URL template is missing from packages.json (RFC 0009 §12.5)"
python3 - "$HEAVY_WORK/search.json" "$PROBE_PACKAGE" <<'PY' || heavy_fail "composer search returned no hit naming the package"
import json, sys
doc = json.load(open(sys.argv[1]))
names = [e.get("name", "") for e in (doc if isinstance(doc, list) else doc.get("results", []))]
sys.exit(0 if any(sys.argv[2] in n for n in names) else 1)
PY
heavy_log "COMPOSER-SEARCH-OK (the client found the route, and the route found the package)"

heavy_done COMPOSER-HEAVY-OK
