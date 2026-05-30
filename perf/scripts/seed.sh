#!/usr/bin/env bash
# Seed the BatleHub perf server with warm-cache data.
#
# Pre-requisites (all must be running):
#   task compose:db        — PostgreSQL
#   task perf:upstream     — mock upstream on :9999
#   task perf:server       — BatleHub with perf/config.perf.toml
#
# Usage: bash perf/scripts/seed.sh [BASE_URL]
set -euo pipefail

BASE="${1:-http://localhost:8080}"
TOKEN="${BATLEHUB_TOKEN:-perf-admin-token}"
AUTH="Authorization: Bearer $TOKEN"
NPM_REG="perf-npm"
SEED_PKG="perf-pkg"
SEED_VER="1.0.0"
WARM_N="${WARM_N:-5}"   # how many times to fetch to ensure cache is hot

echo "==> BatleHub perf seed  base=$BASE  registry=$NPM_REG"

# ── 1. Wait for server ────────────────────────────────────────────────────────
echo -n "    waiting for server..."
for i in $(seq 1 30); do
  if curl -sf "$BASE/healthz" >/dev/null 2>&1; then
    echo " ok"
    break
  fi
  sleep 1
  if [ "$i" -eq 30 ]; then
    echo " TIMEOUT — is 'task perf:server' running?"
    exit 1
  fi
done

# ── 2. Verify auth ────────────────────────────────────────────────────────────
echo -n "    verifying token..."
STATUS=$(curl -sf -o /dev/null -w "%{http_code}" -H "$AUTH" "$BASE/api/v1/me" || echo "000")
if [ "$STATUS" != "200" ]; then
  echo " FAILED (HTTP $STATUS)"
  echo "    Check that perf/config.perf.toml has [[auth.tokens]] value=\"$TOKEN\""
  exit 1
fi
echo " ok"

# ── 3. Verify mock upstream ───────────────────────────────────────────────────
echo -n "    checking mock upstream..."
STATUS=$(curl -sf -o /dev/null -w "%{http_code}" "http://localhost:9999/health" || echo "000")
if [ "$STATUS" != "200" ]; then
  echo " NOT RUNNING (HTTP $STATUS)"
  echo "    Start it with: task perf:upstream"
  exit 1
fi
echo " ok"

# ── 4. Warm the artifact cache ────────────────────────────────────────────────
echo "    warming cache: $NPM_REG/$SEED_PKG@$SEED_VER (×$WARM_N)"
TARBALL_URL="$BASE/proxy/$NPM_REG/$SEED_PKG/$SEED_VER/tarball"
for i in $(seq 1 "$WARM_N"); do
  HTTP_CODE=$(curl -sf -o /dev/null -w "%{http_code}" \
    -H "$AUTH" \
    "$TARBALL_URL" || echo "000")
  printf "      attempt %d: HTTP %s\n" "$i" "$HTTP_CODE"
  if [ "$HTTP_CODE" != "200" ]; then
    echo "    WARN: unexpected status $HTTP_CODE — check mock upstream logs"
  fi
done

# ── 5. Summary ────────────────────────────────────────────────────────────────
cat <<EOF

==> Seed complete. Run k6 scenarios:

    export BATLEHUB_URL=$BASE
    export BATLEHUB_TOKEN=$TOKEN

    k6 run perf/k6/scenarios/01_at_rest.js   # baseline
    k6 run perf/k6/scenarios/02_warm_read.js  # cached reads
    k6 run perf/k6/scenarios/03_cache_miss.js # proxy-through
    k6 run perf/k6/scenarios/04_upload.js     # uploads
    k6 run perf/k6/scenarios/05_mixed.js      # 10-min mixed

    Grafana: http://localhost:3000  (admin/admin)

EOF
