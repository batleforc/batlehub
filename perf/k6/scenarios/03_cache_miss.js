/**
 * Scenario 03 — Cache Miss / Proxy-Through
 *
 * Each VU requests a unique package version that is guaranteed not to be in
 * cache (version = __VU-__ITER). The server must proxy to the mock upstream on
 * every request, so this measures: auth + DB check + upstream HTTP + storage
 * write + response streaming.
 *
 * Pre-requisite:
 *   - mock upstream running: `task perf:upstream`
 *   - perf-npm registry configured with upstream = http://localhost:9999/npm
 *
 * Run: k6 run perf/k6/scenarios/03_cache_miss.js
 */
import http from "k6/http";
import { check, sleep } from "k6";
import { BASE_URL, ADMIN_TOKEN, NPM_REGISTRY } from "../config.js";

export const options = {
  vus: 20,
  duration: "120s",
  thresholds: {
    http_req_failed: ["rate<0.05"],
    // Cache miss path is slower: upstream latency + disk write
    http_req_duration: ["p(95)<3000"],
  },
};

const HEADERS = { Authorization: `Bearer ${ADMIN_TOKEN}` };

export default function cache_miss() {
  // Each VU+iteration combo is a unique version → guaranteed cache miss.
  const version = `0.${__VU}.${__ITER}`;
  const pkg = `miss-pkg-${__VU}`;
  const url = `${BASE_URL}/proxy/${NPM_REGISTRY}/${pkg}/${version}/tarball`;

  const res = http.get(url, { headers: HEADERS });
  // 200 (fresh proxy) or 404 (mock returned not-found) are both acceptable;
  // 5xx from BatleHub itself is what we watch for.
  check(res, {
    "not 5xx": (r) => r.status < 500,
  });

  // Small pause to keep DB write queue from overwhelming in integration envs.
  sleep(0.1);
}
