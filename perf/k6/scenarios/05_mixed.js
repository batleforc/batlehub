/**
 * Scenario 05 — Realistic Mixed Workload (10 min)
 *
 * Simulates a production traffic mix:
 *   80% warm cache reads  (scenario: warm_read)
 *   15% cache miss reads  (scenario: cache_miss)
 *    5% artifact uploads  (scenario: upload)
 *
 * k6 named scenarios run concurrently so the load profile overlaps naturally.
 *
 * Pre-requisites: same as 02, 03, 04 combined.
 * Run: k6 run perf/k6/scenarios/05_mixed.js
 */
import http from "k6/http";
import { check, sleep } from "k6";
import {
  BASE_URL, ADMIN_TOKEN, NPM_REGISTRY, SEED_PKG, SEED_VER,
} from "../config.js";
import { npmPublishPayload } from "../helpers.js";

export const options = {
  scenarios: {
    warm_read: {
      executor: "ramping-vus",
      stages: [
        { duration: "1m",  target: 80 },
        { duration: "7m",  target: 80 },
        { duration: "2m",  target: 0  },
      ],
      exec: "warmRead",
      tags: { scenario: "warm_read" },
    },
    cache_miss: {
      executor: "constant-vus",
      vus: 10,
      duration: "10m",
      exec: "cacheMiss",
      tags: { scenario: "cache_miss" },
    },
    upload: {
      executor: "constant-vus",
      vus: 3,
      duration: "10m",
      exec: "upload",
      tags: { scenario: "upload" },
    },
  },
  thresholds: {
    "http_req_duration{scenario:warm_read}": ["p(95)<200"],
    "http_req_duration{scenario:cache_miss}": ["p(95)<3000"],
    "http_req_duration{scenario:upload}": ["p(95)<5000"],
    http_req_failed: ["rate<0.02"],
  },
};

const AUTH = { Authorization: `Bearer ${ADMIN_TOKEN}` };
const WARM_URL = `${BASE_URL}/proxy/${NPM_REGISTRY}/${SEED_PKG}/${SEED_VER}/tarball`;

export function warmRead() {
  const res = http.get(WARM_URL, { headers: AUTH });
  check(res, { "200": (r) => r.status === 200 });
}

export function cacheMiss() {
  const version = `0.${__VU}.${__ITER}`;
  const pkg = `mix-miss-${__VU}`;
  const res = http.get(
    `${BASE_URL}/proxy/${NPM_REGISTRY}/${pkg}/${version}/tarball`,
    { headers: AUTH }
  );
  check(res, { "not 5xx": (r) => r.status < 500 });
  sleep(0.05);
}

export function upload() {
  const name = `mix-upload-${__VU}`;
  const version = `1.0.${__ITER}`;
  const res = http.put(
    `${BASE_URL}/proxy/perf-local-npm/${name}`,
    npmPublishPayload(name, version, 32),
    { headers: { ...AUTH, "Content-Type": "application/json" } }
  );
  check(res, { "published": (r) => r.status === 200 });
  sleep(1);
}
