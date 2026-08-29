/**
 * Scenario 08 — RFC 0015 §11.7: what a per-identity whole-registry document costs
 *
 * The question this answers is phase 0b's, and it has a decision attached
 * (§11.7, "What the number decides"): if the filtered arm is close to the
 * cached one at size M, a grant-set cache key is an optimisation phase 3 can
 * add later; if it is an order of magnitude worse, the key is load-bearing and
 * phase 3 has to be designed around it from the first commit.
 *
 * Two arms, both shipping today, over the same corpus:
 *
 *   arm 1  perf-gems-proxy  unfiltered, shared cache. `multi_package_document`
 *                           caches the upstream document under an
 *                           identity-blind key; only the block set is applied
 *                           per request. The floor.
 *
 *   arm 2  perf-gems        filtered, uncached — what this registry *was*
 *                           before RFC 0015 phase 3. The local compact-index
 *                           build looped over every package name calling
 *                           `load_visible_versions(…, identity)`: the naive
 *                           correct implementation, and not a prototype of one.
 *
 *   arm 3  perf-gems        filtered, keyed by resolved grant set — what it is
 *                           now. §4.4's filter plus `DocumentCache`. The metric
 *                           names still say `arm2` because they are the same
 *                           registry and renaming them would break the
 *                           comparison with `perf/results/authz-m.json`, which
 *                           holds the arm-2 numbers this build can no longer
 *                           reproduce.
 *
 * Two documents per arm, because they differ in a way that matters: `/versions`
 * carries every live version of every gem and `/names` carries only the names,
 * so they bracket how much of the cost is the *walk* over packages and how much
 * is rendering what the walk found.
 *
 * Three identities, round-robined by VU. Arm 2's cost is per-identity by
 * construction; a single caller would let a per-identity cache that does not
 * exist look like one that does.
 *
 * Run:
 *   task perf:authz:run                    # after perf:authz:corpus SIZE=…
 *   BATLEHUB_SIZE=m task perf:authz:run
 *
 * Deliberately *not* a threshold-gated scenario. §11.7 fixes its thresholds
 * against arm 3, which does not exist until phase 3; this run produces the
 * baseline those thresholds are expressed relative to, so a failure here would
 * be a threshold invented for the occasion. The numbers are printed and
 * recorded instead.
 */
import http from "k6/http";
import { check } from "k6";
import { Trend, Counter } from "k6/metrics";
import { BASE_URL } from "../config.js";

const TOKENS = [
  __ENV.BATLEHUB_TOKEN || "perf-admin-token",
  "perf-user-token",
  "perf-user2-token",
];

/**
 * Duration for the small corpus, a fixed iteration count for the large ones.
 *
 * The two modes are not a convenience. Arm 2 on the L corpus is a single
 * request measured in *minutes*, so a duration-based run finishes with zero
 * completed iterations and reports nothing; and a duration long enough to fix
 * that would be an hour of wall clock to learn an order of magnitude §11.7
 * already calls "informational". Setting BATLEHUB_ITERATIONS switches to
 * per-VU iterations, which bounds the run by work rather than by time.
 */
const DURATION = __ENV.BATLEHUB_DURATION || "60s";
const VUS = Number(__ENV.BATLEHUB_VUS || 4);
const ITERATIONS = Number(__ENV.BATLEHUB_ITERATIONS || 0);

export const options = {
  scenarios: {
    documents: ITERATIONS
      ? {
          executor: "per-vu-iterations",
          vus: VUS,
          iterations: ITERATIONS,
          // Generous: the executor must not cut a request short, or the slow
          // arm's tail — the only part anyone is interested in — is truncated
          // into the fast arm's percentiles.
          maxDuration: __ENV.BATLEHUB_MAX_DURATION || "3h",
        }
      : {
          executor: "constant-vus",
          vus: VUS,
          duration: DURATION,
        },
  },
  // A filtered whole-registry build on the L corpus is measured in seconds, not
  // milliseconds, and k6's default 60s request timeout would turn the number
  // this scenario exists to produce into an error rate.
  noConnectionReuse: false,
  thresholds: {},
  // k6's default trend summary stops at p(95). §11.7 states its thresholds in
  // p99, so a run that cannot report one measures the wrong statistic.
  summaryTrendStats: ["min", "med", "avg", "p(95)", "p(99)", "max"],
};

/** One Trend per arm × document, so the summary separates them. */
const trends = {};
const bytes = {};
const errors = new Counter("doc_errors");

for (const arm of ["arm1_cached", "arm2_filtered"]) {
  for (const doc of ["versions", "names"]) {
    trends[`${arm}_${doc}`] = new Trend(`doc_${arm}_${doc}_ms`, true);
    bytes[`${arm}_${doc}`] = new Trend(`doc_${arm}_${doc}_bytes`);
  }
}

const ARMS = [
  { arm: "arm1_cached", registry: "perf-gems-proxy" },
  { arm: "arm2_filtered", registry: "perf-gems" },
];

export default function wholeRegistryDocuments() {
  const token = TOKENS[__VU % TOKENS.length];
  const headers = { Authorization: `Bearer ${token}` };

  for (const { arm, registry } of ARMS) {
    for (const doc of ["versions", "names"]) {
      const res = http.get(`${BASE_URL}/proxy/${registry}/${doc}`, {
        headers,
        timeout: "600s",
        tags: { arm, doc },
      });
      const ok = check(res, {
        [`${arm}/${doc} 200`]: (r) => r.status === 200,
      });
      if (!ok) {
        errors.add(1, { arm, doc, status: String(res.status) });
        continue;
      }
      trends[`${arm}_${doc}`].add(res.timings.duration);
      bytes[`${arm}_${doc}`].add(res.body ? res.body.length : 0);
    }
  }
}

/**
 * A compact table rather than k6's default wall of metrics.
 *
 * The comparison is the point, so the ratio is computed here rather than left
 * for someone to divide two numbers out of a log — that is the step where a
 * measurement turns into "roughly the same, I think".
 */
export function handleSummary(data) {
  const m = (name) => data.metrics[name] && data.metrics[name].values;
  const rows = [];

  for (const doc of ["versions", "names"]) {
    const a1 = m(`doc_arm1_cached_${doc}_ms`);
    const a2 = m(`doc_arm2_filtered_${doc}_ms`);
    const b1 = m(`doc_arm1_cached_${doc}_bytes`);
    const b2 = m(`doc_arm2_filtered_${doc}_bytes`);
    if (!a1 || !a2) continue;
    rows.push({
      doc,
      arm1_p50: a1.med,
      arm1_p95: a1["p(95)"],
      arm1_p99: a1["p(99)"],
      arm1_bytes: b1 ? b1.avg : 0,
      arm2_p50: a2.med,
      arm2_p95: a2["p(95)"],
      arm2_p99: a2["p(99)"],
      arm2_bytes: b2 ? b2.avg : 0,
      ratio_p99: a1["p(99)"] > 0 ? a2["p(99)"] / a1["p(99)"] : null,
    });
  }

  const fmt = (v) => (v == null ? "—" : v.toFixed(1));
  let out = "\n── RFC 0015 §11.7 — whole-registry documents ──────────────────\n";
  const mode = ITERATIONS ? `${ITERATIONS} iterations/VU` : DURATION;
  out += `size=${__ENV.BATLEHUB_SIZE || "?"}  vus=${VUS}  ${mode}\n\n`;
  out += "document   arm            p50 ms    p95 ms    p99 ms      KB\n";
  for (const r of rows) {
    out += `${r.doc.padEnd(10)} arm1 cached  ${fmt(r.arm1_p50).padStart(8)}  ${fmt(r.arm1_p95).padStart(8)}  ${fmt(r.arm1_p99).padStart(8)}  ${(r.arm1_bytes / 1024).toFixed(0).padStart(6)}\n`;
    out += `${"".padEnd(10)} arm2 filtered${fmt(r.arm2_p50).padStart(8)}  ${fmt(r.arm2_p95).padStart(8)}  ${fmt(r.arm2_p99).padStart(8)}  ${(r.arm2_bytes / 1024).toFixed(0).padStart(6)}\n`;
    out += `${"".padEnd(10)} ratio p99    ${(r.ratio_p99 == null ? "—" : r.ratio_p99.toFixed(1) + "×").padStart(8)}\n\n`;
  }

  const errs = m("doc_errors");
  if (errs && errs.count > 0) {
    out += `!! ${errs.count} request(s) failed — the numbers above are over the rest\n`;
  }

  return {
    stdout: out,
    [`perf/results/authz-${__ENV.BATLEHUB_SIZE || "unknown"}.json`]:
      JSON.stringify(
        { size: __ENV.BATLEHUB_SIZE, vus: VUS, duration: DURATION, iterations: ITERATIONS, rows },
        null,
        2,
      ),
  };
}
