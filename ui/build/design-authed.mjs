#!/usr/bin/env node
/**
 * axe *and* a type-ramp check over the routes that need a session (RFC 0003
 * §4.7, §13).
 *
 * The `impeccable detect` / `@axe-core/cli` gates scan by URL and have no way
 * to carry a session, so `/me/*` and every `/admin/*` page — 18 routes, the
 * largest surface in the console — were simply never measured: they redirect to
 * `/login`, and scanning them would have graded the login page eighteen times.
 *
 * This seeds the token into `localStorage` before the app boots, then runs
 * axe-core in the page. It talks to an already-running Chrome over CDP
 * (`puppeteer-core`, so no second browser is downloaded) — in this workspace
 * that is the `che-browser` sidecar.
 *
 *   BATLEHUB_ADMIN_TOKEN=… BATLEHUB_USER_TOKEN=… node build/a11y-authed.mjs
 *
 * Tokens come from the environment and are never written to disk: they are
 * credentials for a running instance, not fixtures.
 */
import puppeteer from "puppeteer-core";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const AXE_SOURCE = readFileSync(require.resolve("axe-core/axe.min.js"), "utf8");

const BASE = process.env.BASE ?? "http://localhost:5174";
const CDP = process.env.CDP_URL ?? "http://localhost:9222";
const TOKEN_KEY = "batlehub_access_token";

/** WCAG 2.2 AA, the same tag set the unauthenticated gate uses. */
const TAGS = ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"];

/**
 * DESIGN.md's two ramps: JetBrains Mono at 12/13/15/16/20 and Silkscreen on its
 * 8px em at 16/24/56/72/88/104. A size outside this set is a step nobody
 * declared, and "the admin pages are not redrawn in the specimen grammar" stops
 * being an assertion the moment it is measured.
 */
const RAMP = new Set([12, 13, 15, 16, 20, 24, 56, 72, 88, 104]);
const DISPLAY_FACE = "Silkscreen";

const ADMIN_ROUTES = [
  "/admin/dashboard",
  "/admin/packages/all",
  "/admin/packages/bulk",
  "/admin/security/users",
  "/admin/security/ip-blocks",
  "/admin/security/access-check",
  "/admin/namespaces/team-namespaces",
  "/admin/namespaces/beta-channel",
  "/admin/operations/config-reload",
  "/admin/operations/warming",
  "/admin/operations/explore-cache",
  "/admin/observability/health",
  "/admin/observability/sbom",
  "/admin/observability/audit-log",
  "/admin/notifications",
];

// `/me/tokens` needs `auth_provider` set, which a static token satisfies.
//
// `/` is here as well as in the unauthenticated scan, and both are needed: the
// quota and advisory widgets (RFC 0004 §4.2) render *only* for a signed-in
// viewer, so the anonymous pass measures a home page that is missing them —
// including the one meter in the system, whose whole point is an accessibility
// contract.
const USER_ROUTES = ["/", "/me/profile", "/me/tokens", "/me/namespace", "/me/cli"];

const plans = [
  { role: "admin", token: process.env.BATLEHUB_ADMIN_TOKEN, routes: [...ADMIN_ROUTES, ...USER_ROUTES] },
  { role: "user", token: process.env.BATLEHUB_USER_TOKEN, routes: USER_ROUTES },
];

/**
 * Coverage is asserted, not merely regenerated (RFC 0004 §10).
 *
 * This list growing is expected — every route a future pass adds or splits
 * extends it. It *shrinking* is the failure this catches: a route quietly
 * dropped from the arrays above would make the gate pass by measuring less,
 * which reads identically to measuring clean.
 */
const EXPECTED_COMBINATIONS = 25;
const planned = plans.reduce((n, p) => n + p.routes.length, 0);
if (planned < EXPECTED_COMBINATIONS) {
  console.error(
    `coverage shrank: ${planned} route/role combinations, expected at least ${EXPECTED_COMBINATIONS}. ` +
      `If a route was deliberately removed, lower EXPECTED_COMBINATIONS in the same commit.`,
  );
  process.exit(2);
}

const missing = plans.filter((p) => !p.token).map((p) => p.role);
if (missing.length) {
  console.error(`missing token(s) for: ${missing.join(", ")} — set BATLEHUB_{ADMIN,USER}_TOKEN`);
  process.exit(2);
}

const browser = await puppeteer.connect({ browserURL: CDP });
let failures = 0;
let scanned = 0;

for (const { role, token, routes } of plans) {
  for (const route of routes) {
    const page = await browser.newPage();
    await page.setViewport({ width: 1440, height: 900 });
    const pageErrors = [];
    page.on("pageerror", (e) => pageErrors.push(String(e.message).split("\n")[0].slice(0, 120)));

    try {
      // The token has to exist before the app's first script runs: `initAuth`
      // reads it synchronously at import time, and the router then resolves
      // identity once. Seeding after `goto` would race that.
      await page.evaluateOnNewDocument(
        (key, value) => localStorage.setItem(key, value),
        TOKEN_KEY,
        token,
      );
      await page.goto(BASE + route, { waitUntil: "networkidle2" });
      await new Promise((r) => setTimeout(r, 900));

      const landed = new URL(page.url()).pathname;
      if (landed !== route) {
        // A redirect means the session did not take, so whatever axe measured
        // would be the login page. Reported rather than counted as a pass.
        console.log(`✗ ${role} ${route} → redirected to ${landed} (session not applied)`);
        failures++;
        continue;
      }

      await page.evaluate(AXE_SOURCE);
      const results = await page.evaluate(
        async (tags) => await window.axe.run(document, { runOnly: { type: "tag", values: tags } }),
        TAGS,
      );
      scanned++;

      const type = await page.evaluate((face) => {
        const sizes = new Set();
        for (const el of document.querySelectorAll("body *")) {
          if (el.children.length || !el.innerText?.trim() || !el.getClientRects().length) continue;
          sizes.add(Math.round(parseFloat(getComputedStyle(el).fontSize)));
        }
        const h1 = document.querySelector("main h1") ?? document.querySelector("h1");
        return {
          sizes: [...sizes].sort((a, b) => a - b),
          h1: h1 && {
            size: Math.round(parseFloat(getComputedStyle(h1).fontSize)),
            display: getComputedStyle(h1).fontFamily.includes(face),
          },
        };
      }, DISPLAY_FACE);

      const offRamp = type.sizes.filter((s) => !RAMP.has(s));
      const typeProblems = [];
      if (offRamp.length) typeProblems.push(`sizes off the ramp: ${offRamp.join(", ")}px`);
      if (!type.h1) typeProblems.push("no h1");
      else if (!type.h1.display) typeProblems.push(`h1 not in the display face (${type.h1.size}px)`);

      if (typeProblems.length) {
        failures++;
        console.log(`✗ ${role} ${route}`);
        for (const p of typeProblems) console.log(`    [type] ${p}`);
      }

      if (results.violations.length) {
        failures++;
        if (!typeProblems.length) console.log(`✗ ${role} ${route}`);
        for (const v of results.violations) {
          console.log(`    [${v.id}] ${v.help} — ${v.nodes.length} node(s)`);
          for (const n of v.nodes.slice(0, 3)) {
            console.log(`      ${n.target.join(" ")}`);
            // The summary carries the measured ratio and the colours axe used,
            // which is the difference between fixing the contrast and guessing.
            const why = (n.failureSummary ?? "").split("\n").filter(Boolean).slice(1, 3);
            for (const line of why) console.log(`        ${line.trim()}`);
          }
        }
      } else if (!typeProblems.length) {
        console.log(`✓ ${role} ${route}`);
      }
      if (pageErrors.length) {
        failures++;
        console.log(`  ! page error: ${[...new Set(pageErrors)].join(" | ")}`);
      }
    } finally {
      await page.close();
    }
  }
}

await browser.disconnect();
console.log(`\n${scanned} authenticated route(s) scanned, ${failures} with findings`);
process.exit(failures ? 1 : 0);
