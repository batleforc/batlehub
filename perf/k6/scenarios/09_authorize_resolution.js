/**
 * Scenario 09 — RFC 0015 §11.7's *second* number: what resolution costs
 *
 * > Everything above measures the **document** — its size, and how well it
 * > caches. It does not measure the resolver, and the two can fail
 * > independently. §6.3 describes "the SQL visibility predicate becomes a grant
 * > predicate", which understates the change: today's predicate is a comparison
 * > against a column on the row, where a grant predicate has to walk registry →
 * > namespace → package → version and union what matches at each.
 *
 * **Threshold:** *"a single-coordinate `authorize` adding more than 2 ms at p99
 * on the M corpus sends the storage design back before phase 4 builds the
 * `policy` table on top of it."* Deliberately stricter than the 20 % document
 * threshold, because a listing is served occasionally and cached, where
 * `authorize` runs on every request that reaches this server and has nowhere to
 * hide.
 *
 * # What this measures, and what it cannot
 *
 * A single-coordinate read on the smallest document the registry serves — a
 * RubyGems `/info/{gem}`, one package, a handful of versions. Total latency is
 * therefore *dominated* by authorization rather than by serialisation, which is
 * as close to "authorize in isolation" as an end-to-end harness gets without
 * instrumenting the function itself.
 *
 * Two arms, because the interesting question is the *delta*:
 *
 *   granted    a package that has a package-tier grant row. The `grants` lookup
 *              finds something and resolution unions it in.
 *   ungranted  a package with none. The lookup is an index probe that returns
 *              nothing — the common case on any real estate, and the one a
 *              corpus with no grants at all would mistake for the whole story.
 *
 * `corpus-seed --granted-fraction` decides which is which: every tenth package
 * carries a grant, so `perf-gem-0000000` is granted and `perf-gem-0000001` is
 * not.
 *
 * Run:
 *   task perf:authz:corpus SIZE=m
 *   task perf:authz:upstream SIZE=m      # separate shell
 *   task perf:authz:server               # separate shell
 *   task perf:authz:resolution SIZE=m
 */
import http from "k6/http";
import { check } from "k6";
import { Trend } from "k6/metrics";
import { BASE_URL } from "../config.js";

const TOKENS = [
  __ENV.BATLEHUB_TOKEN || "perf-admin-token",
  "perf-user-token",
  "perf-user2-token",
];

const DURATION = __ENV.BATLEHUB_DURATION || "60s";
const VUS = Number(__ENV.BATLEHUB_VUS || 8);
const REGISTRY = __ENV.BATLEHUB_REGISTRY || "perf-gems";

/** How many packages the corpus holds, so the arms address real ones. */
const PACKAGES = Number(__ENV.BATLEHUB_PACKAGES || 25000);
/** Every Nth package carries a package-tier grant; mirrors `--granted-fraction`. */
const GRANTED_EVERY = Number(__ENV.BATLEHUB_GRANTED_EVERY || 10);

export const options = {
  vus: VUS,
  duration: DURATION,
  thresholds: {},
  summaryTrendStats: ["min", "med", "avg", "p(95)", "p(99)", "max"],
};

const granted = new Trend("resolve_granted_ms", true);
const ungranted = new Trend("resolve_ungranted_ms", true);

function pkg(n) {
  return `perf-gem-${String(n).padStart(7, "0")}`;
}

export default function authorizeResolution() {
  const headers = { Authorization: `Bearer ${TOKENS[__VU % TOKENS.length]}` };

  // A different coordinate each iteration, so the measurement is of resolution
  // rather than of whatever the last request left warm. Packages are picked
  // deterministically from the VU and iteration rather than at random: k6's
  // `Math.random` would make two runs incomparable, and the whole point of a
  // threshold fixed in advance is that the runs are comparable.
  const base = (__VU * 1_000 + __ITER * 7) % PACKAGES;

  const g = base - (base % GRANTED_EVERY); // …0, …10, …20 — has a grant row
  const u = g + 1; // the neighbour, which has none

  for (const [n, trend, label] of [
    [g, granted, "granted"],
    [u, ungranted, "ungranted"],
  ]) {
    const res = http.get(`${BASE_URL}/proxy/${REGISTRY}/info/${pkg(n)}`, {
      headers,
      tags: { arm: label },
    });
    if (check(res, { [`${label} 200`]: (r) => r.status === 200 })) {
      trend.add(res.timings.duration);
    }
  }
}

export function handleSummary(data) {
  const m = (n) => data.metrics[n] && data.metrics[n].values;
  const g = m("resolve_granted_ms");
  const u = m("resolve_ungranted_ms");
  const fmt = (v) => (v == null ? "—" : v.toFixed(2));

  let out = "\n── RFC 0015 §11.7 — single-coordinate resolution ──────────────\n";
  out += `size=${__ENV.BATLEHUB_SIZE || "?"}  vus=${VUS}  duration=${DURATION}\n\n`;
  out += "arm                p50 ms   p95 ms   p99 ms   max ms\n";
  for (const [label, v] of [
    ["granted", g],
    ["ungranted", u],
  ]) {
    if (!v) continue;
    out += `${label.padEnd(16)} ${fmt(v.med).padStart(8)} ${fmt(v["p(95)"]).padStart(8)} ${fmt(v["p(99)"]).padStart(8)} ${fmt(v.max).padStart(8)}\n`;
  }
  if (g && u) {
    out += `\ndelta p99 (granted − ungranted): ${fmt(g["p(99)"] - u["p(99)"])} ms\n`;
    out += "\nThe threshold is on what `authorize` *adds*, and neither arm is a\n";
    out += "zero: both include the read itself. The delta is what finding a grant\n";
    out += "row costs over probing and finding none; the absolute p99 bounds the\n";
    out += "whole request, so authorization's share cannot exceed it.\n";
  }

  return {
    stdout: out,
    [`perf/results/resolution-${__ENV.BATLEHUB_SIZE || "unknown"}.json`]: JSON.stringify(
      {
        size: __ENV.BATLEHUB_SIZE,
        vus: VUS,
        duration: DURATION,
        granted: g && { p50: g.med, p95: g["p(95)"], p99: g["p(99)"], max: g.max },
        ungranted: u && { p50: u.med, p95: u["p(95)"], p99: u["p(99)"], max: u.max },
      },
      null,
      2,
    ),
  };
}
