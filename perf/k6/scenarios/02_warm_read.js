/**
 * Scenario 02 — Warm Cache Read Load
 *
 * Reads a pre-cached artifact across a ramp from 10 VU → 200 VU → back to 0.
 * Every VU hits the same URL, so after the first request the artifact is in the
 * filesystem cache and all subsequent requests are served from disk.
 *
 * Approximate RPS mapping (single core laptop, adjust VU counts for your hardware):
 *   10  VU  ≈  100 req/s
 *   50  VU  ≈  500 req/s
 *   100 VU  ≈ 1000 req/s
 *   200 VU  ≈ 2000 req/s
 *
 * Pre-requisite: run `task perf:seed` first.
 * Run: k6 run perf/k6/scenarios/02_warm_read.js
 */
import http from "k6/http";
import { check } from "k6";
import {
  BASE_URL,
  ADMIN_TOKEN,
  NPM_REGISTRY,
  SEED_PKG,
  SEED_VER,
} from "../config.js";

export const options = {
  stages: [
    { duration: "30s", target: 10 }, // warm up
    { duration: "60s", target: 50 }, // 500 req/s
    { duration: "60s", target: 100 }, // 1k req/s
    { duration: "60s", target: 200 }, // 2k req/s
    { duration: "30s", target: 0 }, // cool down
  ],
  thresholds: {
    // Error rate must stay below 1 % — hard requirement.
    http_req_failed: ["rate<0.01"],
    // Latency thresholds: informational targets, not hard gates in perf:run:all.
    // Run `task perf:run:read` individually to enforce them as a CI gate.
    //   p95 < 500 ms — achievable on a dev machine under 200 VU
    //   p99 < 2 s   — tail-latency guard
    http_req_duration: ["p(95)<500", "p(99)<2000"],
  },
};

const URL = `${BASE_URL}/proxy/${NPM_REGISTRY}/${SEED_PKG}/${SEED_VER}/tarball`;
const HEADERS = { Authorization: `Bearer ${ADMIN_TOKEN}` };

export default function warm_read() {
  const res = http.get(URL, { headers: HEADERS });
  check(res, {
    "status 200": (r) => r.status === 200,
    "body non-empty": (r) => r.body.length > 0,
  });
}
