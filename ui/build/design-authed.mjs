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

/**
 * Both widths, because for fifteen pages neither gate looked at the narrow one.
 *
 * The rendered detector runs 390x844 but only over the *unauthenticated*
 * routes; this gate covered `/admin/*` but at 1440x900 only. So the entire
 * admin surface — the largest in the console — was never measured on a phone,
 * and every one of its pages scrolled sideways: `AdminLayout` held the mobile
 * tab strip and the page content as siblings of one flex row, and documents
 * came out 496-705px wide on a 390px viewport (RFC 0004 Phase 5).
 */
const VIEWPORTS = [1440, 390];
const DISPLAY_FACE = "Silkscreen";

const ADMIN_ROUTES = [
  "/admin/dashboard",
  "/admin/packages/all",
  "/admin/packages/bulk",
  "/admin/security/blocks",
  "/admin/security/access-check",
  "/admin/namespaces/team-namespaces",
  "/admin/namespaces/beta-channel",
  "/admin/operations/config-reload",
  "/admin/operations/warming",
  "/admin/observability/health",
  "/admin/operations/sbom",
  "/admin/observability/audit-log",
  "/admin/notifications/subscriptions",
  "/admin/notifications/inbound",
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
// RFC 0004 Phase 5 moved this twice, both deliberately: -1 for the removed
// `/admin/operations/explore-cache` (its control went to the health page),
// then +2 for the notifications split. The number tracks real coverage, which
// is the point — it may only change in the commit that changes the routes.
const EXPECTED_COMBINATIONS = 24;
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
   for (const width of VIEWPORTS) {
    const page = await browser.newPage();
    await page.setViewport({ width, height: 900 });
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
        console.log(`✗ ${role} ${route} @${width} → redirected to ${landed} (session not applied)`);
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
      // The document must never scroll sideways. This is the assertion the
      // whole admin surface failed silently until RFC 0004 Phase 5, because
      // nothing measured these routes narrow.
      const overflow = await page.evaluate(() => {
        const vw = window.innerWidth;
        const doc = document.documentElement.scrollWidth;
        if (doc <= vw) return null;
        // Name a culprit that is not already inside its own scroll container —
        // a wide table that scrolls in its own wrapper is correct, per
        // DESIGN.md's Own-Container Overflow Rule.
        const clipped = (el) => {
          for (let n = el.parentElement; n; n = n.parentElement) {
            const o = getComputedStyle(n).overflowX;
            if (o === "auto" || o === "hidden" || o === "scroll") return true;
          }
          return false;
        };
        const culprit = [...document.querySelectorAll("body *")]
          .find((el) => el.getBoundingClientRect().right > vw + 1 && !clipped(el));
        return {
          doc,
          vw,
          culprit: culprit
            ? `${culprit.tagName.toLowerCase()}.${(culprit.className || "").toString().split(" ").slice(0, 4).join(".")}`
            : "(inside a scroll container — check the wrapper)",
        };
      });

      const typeProblems = [];
      if (overflow) {
        typeProblems.push(
          `document scrolls sideways: ${overflow.doc}px on a ${overflow.vw}px viewport — ${overflow.culprit}`,
        );
      }
      if (offRamp.length) typeProblems.push(`sizes off the ramp: ${offRamp.join(", ")}px`);
      if (!type.h1) typeProblems.push("no h1");
      else if (!type.h1.display) typeProblems.push(`h1 not in the display face (${type.h1.size}px)`);

      if (typeProblems.length) {
        failures++;
        console.log(`✗ ${role} ${route} @${width}`);
        for (const p of typeProblems) console.log(`    [type] ${p}`);
      }

      if (results.violations.length) {
        failures++;
        if (!typeProblems.length) console.log(`✗ ${role} ${route} @${width}`);
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
        console.log(`✓ ${role} ${route} @${width}`);
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
}

await browser.disconnect();
console.log(`\n${scanned} authenticated route(s) scanned, ${failures} with findings`);
process.exit(failures ? 1 : 0);
