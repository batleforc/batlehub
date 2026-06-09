/**
 * Scenario 04 — Artifact Upload / Publish Load
 *
 * Publishes uniquely-versioned npm packages concurrently.
 * Each VU publishes to the perf-npm LOCAL registry (not a proxy registry).
 *
 * Tests: payload collection, version policy, quota check, DB insert (pending →
 * published), filesystem write, ownership initialization.
 *
 * Pre-requisite:
 *   - perf-local-npm registry must exist (created by `task perf:seed`)
 *   - admin token must have publish rights
 *
 * Run: k6 run perf/k6/scenarios/04_upload.js
 *
 * Adjust ARTIFACT_KB to test different payload sizes.
 */
import http from "k6/http";
import { check } from "k6";
import { BASE_URL, ADMIN_TOKEN } from "../config.js";
import { npmPublishPayload } from "../helpers.js";

const ARTIFACT_KB = Number.parseInt(__ENV.ARTIFACT_KB || "64");
const REGISTRY = "perf-local-npm";

export const options = {
  vus: 10,
  duration: "60s",
  thresholds: {
    http_req_failed: ["rate<0.05"],
    // Publishing involves disk I/O + multiple DB writes; 5 s P95 is generous.
    http_req_duration: ["p(95)<5000"],
  },
};

const HEADERS = {
  Authorization: `Bearer ${ADMIN_TOKEN}`,
  "Content-Type": "application/json",
};

export default function upload() {
  const name = `perf-upload-${__VU}`;
  const version = `1.0.${__ITER}`;

  const payload = npmPublishPayload(name, version, ARTIFACT_KB);

  const res = http.put(`${BASE_URL}/proxy/${REGISTRY}/${name}`, payload, {
    headers: HEADERS,
  });

  check(res, {
    "published 200": (r) => r.status === 200,
    "quota header present": (r) =>
      r.headers["X-Quota-Remaining"] !== undefined ||
      r.headers["x-quota-remaining"] !== undefined,
  });
}
