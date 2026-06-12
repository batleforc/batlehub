/**
 * Scenario 07 — Cache Eviction Sweep
 *
 * Two concurrent named scenarios:
 *
 *   cache_growth   — same cache-miss pattern as scenario 03: each iteration
 *                     requests a new version of `evict-pkg-{VU}`, so
 *                     artifact_meta accumulates several versions per package
 *                     for the eviction sweep to act on
 *                     (see `crates/core/src/services/eviction/mod.rs::run_keep_latest_n`
 *                     and `::run_ttl`).
 *
 *   eviction_sweep — calls `POST /api/v1/admin/registries/{registry}/evict`
 *                     (admin-only, see `crates/web/src/handlers/back_office/eviction.rs`)
 *                     at a steady cadence and measures sweep latency while
 *                     cache_growth produces new artifacts concurrently.
 *
 * Pre-requisite: `[registries.cache]` for perf-npm must set at least one of
 * artifact_ttl_secs / idle_days / max_size_bytes / keep_latest_n (see
 * perf/config.perf.toml — both artifact_ttl_secs and keep_latest_n are set by
 * default), otherwise `/evict` returns 404 "eviction not configured for this
 * registry".
 *
 * Run: k6 run perf/k6/scenarios/07_eviction.js
 */
import http from "k6/http";
import { check, sleep } from "k6";
import { BASE_URL, ADMIN_TOKEN, NPM_REGISTRY } from "../config.js";

export const options = {
  scenarios: {
    cache_growth: {
      executor: "constant-vus",
      vus: 10,
      duration: "60s",
      exec: "cacheGrowth",
      tags: { scenario: "cache_growth" },
    },
    eviction_sweep: {
      executor: "constant-arrival-rate",
      rate: 1,
      timeUnit: "5s",
      duration: "60s",
      preAllocatedVUs: 1,
      maxVUs: 2,
      exec: "evictionSweep",
      tags: { scenario: "eviction_sweep" },
    },
  },
  thresholds: {
    "http_req_duration{scenario:cache_growth}": ["p(95)<3000"],
    "http_req_duration{scenario:eviction_sweep}": ["p(95)<5000"],
    http_req_failed: ["rate<0.05"],
  },
};

const HEADERS = { Authorization: `Bearer ${ADMIN_TOKEN}` };
const EVICT_URL = `${BASE_URL}/api/v1/admin/registries/${NPM_REGISTRY}/evict`;

export function cacheGrowth() {
  // Same package per VU, new version every iteration → multiple cached
  // versions per package for keep_latest_n to trim.
  const pkg = `evict-pkg-${__VU}`;
  const version = `0.0.${__ITER}`;
  const url = `${BASE_URL}/proxy/${NPM_REGISTRY}/${pkg}/${version}/tarball`;

  const res = http.get(url, { headers: HEADERS });
  check(res, { "not 5xx": (r) => r.status < 500 });
  sleep(0.1);
}

export function evictionSweep() {
  const res = http.post(EVICT_URL, null, { headers: HEADERS });
  check(res, {
    "status 200": (r) => r.status === 200,
    "report has total": (r) => typeof JSON.parse(r.body).total === "number",
  });
}
