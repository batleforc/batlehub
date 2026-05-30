/**
 * Scenario 01 — At Rest Baseline
 *
 * 1 VU, 60 s. Measures idle memory/CPU via Prometheus and confirms the server
 * is responsive before any load is applied.
 *
 * Run: k6 run perf/k6/scenarios/01_at_rest.js
 * Watch: http://localhost:3000 (Grafana) → BatleHub Performance dashboard
 */
import http from "k6/http";
import { check, sleep } from "k6";
import { BASE_URL, ADMIN_TOKEN } from "../config.js";

export const options = {
  vus: 1,
  duration: "60s",
  thresholds: {
    http_req_failed: ["rate<0.01"],
    http_req_duration: ["p(95)<200"],
  },
};

export default function () {
  // Health check — no auth required.
  const health = http.get(`${BASE_URL}/healthz`);
  check(health, { "health 200": (r) => r.status === 200 });

  // Prometheus metrics endpoint.
  const metrics = http.get(`${BASE_URL}/metrics`);
  check(metrics, { "metrics 200": (r) => r.status === 200 });

  // Auth-protected current user endpoint.
  const me = http.get(`${BASE_URL}/api/v1/me`, {
    headers: { Authorization: `Bearer ${ADMIN_TOKEN}` },
  });
  check(me, { "me 200": (r) => r.status === 200 });

  sleep(1);
}
