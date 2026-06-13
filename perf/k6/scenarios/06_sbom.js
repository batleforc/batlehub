/**
 * Scenario 06 — SBOM Retrieval & Export
 *
 * Two concurrent named scenarios:
 *
 *   sbom_read   — per-artifact SBOM lookup (`GET /api/v1/sbom/{registry}/{name}/{version}`).
 *                  A single keyed read from the sbom repository (Postgres), alternating
 *                  between the spdx and cyclonedx formats. Exercises the path enabled by
 *                  `[registries.sbom]` and populated on every cache miss / publish
 *                  (see `crates/core/src/services/proxy/resolve.rs` and
 *                  `crates/core/src/services/local_registry/lifecycle.rs`).
 *
 *   sbom_export — org-level SBOM export (`GET /api/v1/sbom/export`, admin-only). Merges
 *                  every SBOM document recorded for a registry into one response, so its
 *                  cost grows with how many artifacts have been cached/published. Run
 *                  scenarios 02-05 first to build up a larger dataset and see how export
 *                  latency scales.
 *
 * Pre-requisite: `task perf:seed` (warms perf-npm/perf-pkg@1.0.0, which records an SBOM
 * on the cache-miss path since `[registries.sbom]` is enabled in config.perf*.toml).
 *
 * Run: k6 run perf/k6/scenarios/06_sbom.js
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
  scenarios: {
    sbom_read: {
      executor: "ramping-vus",
      stages: [
        { duration: "10s", target: 10 },
        { duration: "40s", target: 30 },
        { duration: "10s", target: 0 },
      ],
      exec: "sbomRead",
      tags: { scenario: "sbom_read" },
    },
    sbom_export: {
      executor: "constant-vus",
      vus: 2,
      duration: "60s",
      exec: "sbomExport",
      tags: { scenario: "sbom_export" },
    },
  },
  thresholds: {
    "http_req_duration{scenario:sbom_read}": ["p(95)<300"],
    "http_req_duration{scenario:sbom_export}": ["p(95)<5000"],
    http_req_failed: ["rate<0.01"],
  },
};

const HEADERS = { Authorization: `Bearer ${ADMIN_TOKEN}` };
const FORMATS = ["spdx", "cyclonedx"];

export function sbomRead() {
  const format = FORMATS[__ITER % FORMATS.length];
  const url = `${BASE_URL}/api/v1/sbom/${NPM_REGISTRY}/${SEED_PKG}/${SEED_VER}?format=${format}`;

  const res = http.get(url, { headers: HEADERS });
  check(res, {
    "status 200": (r) => r.status === 200,
    "body non-empty": (r) => r.body.length > 0,
  });
}

export function sbomExport() {
  const format = FORMATS[__ITER % FORMATS.length];
  const url = `${BASE_URL}/api/v1/sbom/export?registry=${NPM_REGISTRY}&format=${format}`;

  const res = http.get(url, { headers: HEADERS });
  check(res, {
    "status 200": (r) => r.status === 200,
    "content-disposition present": (r) =>
      r.headers["Content-Disposition"] !== undefined ||
      r.headers["content-disposition"] !== undefined,
  });
}
