#!/usr/bin/env node
/**
 * One space, one reader — enforced (RFC 0005-bis §4.4, §5.1).
 *
 * RFC 0005 sorted the documentation by who reads it, at the top level, and put
 * nothing below that to hold the line. `guide/` re-acquired both audiences
 * inside a year: 25 links in one sidebar mixing "deploy this behind Postgres"
 * with "point npm at it". A rule with no call site is a rule nobody adopted,
 * which is the finding RFC 0005 kept making about itself.
 *
 * Four assertions. `check-links.mjs` already owns the orphan half — a page in
 * *no* sidebar — and this owns its mirror image and the counts.
 *
 *   one sidebar     A page listed in two sidebars will be edited for one reader
 *                   and read by the other.
 *
 *   no loops        A "See also" must not point at an index that lists the page
 *                   it is on. Twenty-one registry pages ended with
 *                   "User Guide → npm", which led to a table of links back to
 *                   the registry pages. A link that returns you to where you
 *                   started is not a link.
 *
 *   sidebar size    A sidebar is one person's list, and a list nobody can hold
 *                   in their head is a list they scroll past.
 *
 *   no typed TOC    The theme draws the outline from the headings. Sixteen
 *                   pages carried a second one, and the nine that were typed by
 *                   hand had every numbered entry dead, because VitePress
 *                   prefixes a leading digit with `_`.
 *
 *   node build/check-audience.mjs
 */
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, relative, sep } from "node:path";
import { fileURLToPath } from "node:url";

const DOCS = fileURLToPath(new URL("..", import.meta.url));
/**
 * A ratchet, not a target. The measured defect was 25 links in one sidebar
 * mixing two audiences; the split left `/guide/` at 20 and created `/use/` at
 * 10. Twenty may only go down — the number exists so the next page has to
 * displace one rather than join the pile.
 */
const MAX_SIDEBAR = 20;

/**
 * A catalogue is one entry per thing catalogued, and its length is a property
 * of the domain rather than of anyone's editing. `/registries/` has 21 registry
 * types because BatleHub supports 21, and `/rfc/` has one entry per RFC ever
 * written — that list only grows, and the way to shorten it would be to stop
 * publishing design history. The rule this file enforces is one audience per
 * sidebar, and the size cap is a proxy for it that does not apply where the
 * list is an index.
 */
const CATALOGUE = new Set(["/registries/", "/rfc/"]);
const HOME_CARDS = 3;
/** A page with this many links into one directory is an index of it. */
const INDEX_THRESHOLD = 10;

const findings = [];
const sizes = [];
const add = (kind, detail) => findings.push({ kind, detail });

/* ── One sidebar per page ─────────────────────────────────────────────────── */

const config = readFileSync(join(DOCS, ".vitepress", "config.ts"), "utf8");
const sidebars = [...config.matchAll(/^ {6}"(\/[^"]*)": \[$/gm)].map((m) => m[1]);

/** Every `link:` under each sidebar key, keyed by the sidebar it belongs to. */
const listedIn = new Map(); // link → [sidebar, …]
for (const key of sidebars) {
  const start = config.indexOf(`      "${key}": [`);
  // The last sidebar has no successor to stop at, so it runs to the close of the
  // object literal instead.
  const later = sidebars
    .map((k) => config.indexOf(`      "${k}": [`))
    .filter((i) => i > start)
    .sort((a, b) => a - b);
  const end = later.length > 0 ? later[0] : config.indexOf("\n    },", start);
  const body = config.slice(start, end);
  const links = [...body.matchAll(/link:\s*"([^"]+)"/g)].map((m) => m[1]);

  sizes.push(`${key} ${links.length}`);
  if (links.length > MAX_SIDEBAR && !CATALOGUE.has(key)) {
    add("sidebar too long", `${key} — ${links.length} links, over ${MAX_SIDEBAR}`);
  }
  for (const l of links) {
    if (!listedIn.has(l)) listedIn.set(l, []);
    listedIn.get(l).push(key);
  }
}

for (const [link, keys] of listedIn) {
  if (keys.length > 1) {
    add("two sidebars", `${link} — listed in ${keys.join(" and ")}`);
  }
}

/* ── No "See also" that points at an index of this page ───────────────────── */

const SKIP_DIRS = new Set(["node_modules", ".vitepress", "build", "public", "internal"]);
function pages(dir = DOCS, acc = []) {
  for (const entry of readdirSync(dir)) {
    if (SKIP_DIRS.has(entry)) continue;
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) pages(full, acc);
    else if (entry.endsWith(".md")) acc.push(full);
  }
  return acc;
}

const all = pages();
const urlOf = (file) =>
  "/" + relative(DOCS, file).split(sep).join("/").replace(/(index)?\.md$/, "");
const textOf = new Map(all.map((f) => [f, readFileSync(f, "utf8")]));

for (const file of all) {
  const src = textOf.get(file);
  // Split rather than a lazy `[\s\S]*?` with a `(?=^## |\Z)` lookahead: JS has no
  // `\Z`, so that alternative was matching a literal "Z" and the section could
  // only ever be ended by a following `## ` — a "See also" that ran to the end of
  // the page was skipped entirely.
  const afterHeading = src.split(/^## See also\n/m)[1];
  if (afterHeading === undefined) continue;
  const seeAlso = afterHeading.split(/^## /m)[0];
  const here = urlOf(file);
  const dir = here.slice(0, here.lastIndexOf("/") + 1);

  // The whole `(…)` destination in one linear capture, then trimmed at the
  // fragment. Two adjacent greedy classes that both accept `#` backtrack.
  for (const [, dest] of seeAlso.matchAll(/\]\(([^)]*)\)/g)) {
    const target = dest.split(/[#\s]/)[0];
    if (!target.startsWith("/")) continue;
    const targetFile = all.find((f) => urlOf(f) === target || urlOf(f) === target + "/");
    if (!targetFile) continue;
    const back = [...textOf.get(targetFile).matchAll(/\]\((\/[^)#\s]*)/g)].filter((m) =>
      m[1].startsWith(dir),
    );
    if (back.length >= INDEX_THRESHOLD) {
      add(
        "see-also loop",
        `${relative(DOCS, file)} → ${target}, which links to ${back.length} pages under ${dir}`,
      );
    }
  }
}

/* ── The counts, because "we reduced it" is not a test ────────────────────── */

const home = readFileSync(join(DOCS, "index.md"), "utf8");
const cards = (home.match(/^ {2}- icon:/gm) ?? []).length;
if (cards !== HOME_CARDS) {
  add("home page", `${cards} feature cards, expected ${HOME_CARDS}`);
}

for (const file of all) {
  const src = textOf.get(file);
  if (/^## Table of [Cc]ontents$/m.test(src) || /^\[\[toc\]\]$/m.test(src)) {
    add("typed table of contents", relative(DOCS, file));
  }
}

/* ── Report ───────────────────────────────────────────────────────────────── */

if (findings.length) {
  console.error(`${findings.length} finding(s):\n`);
  for (const f of findings) console.error(`  ${f.kind}: ${f.detail}`);
  process.exit(1);
}
console.log(
  `each page is in one sidebar and no "see also" loops back — ` + sizes.join(" · "),
);
