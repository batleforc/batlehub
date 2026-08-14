#!/usr/bin/env node
/**
 * Every cross-reference in the documentation resolves, and no published page is
 * unreachable (RFC 0005 phase 8).
 *
 * Nothing has ever checked this, and two references were already dead when the
 * RFC was written: `incident-response.md` pointed at `docs/post-mortem-template.md`
 * and `soc2-checklist.md` at `docs/monitoring.md`, neither of which existed.
 * VitePress's own dead-link check would not have found either, and that is the
 * gap this fills rather than duplicates:
 *
 *   - VitePress checks markdown links in *published* pages only. This also
 *     checks `internal/` — excluded from the build, still read by people — and
 *     the repository's own markdown (README, CLAUDE.md, CONTRIBUTING.md, …),
 *     which is where most links into the documentation actually live.
 *
 *   - VitePress checks `[text](target)`. Both of the already-dead references
 *     above were written as inline code, `docs/monitoring.md`, which is a link
 *     in every sense except the syntactic one. Any code span that contains a
 *     slash and ends in `.md` is treated as a path and checked.
 *
 *   - VitePress cannot know what an orphan is. A page that no sidebar, nav
 *     entry or other page links to is published and unreachable, which is the
 *     same defect as a dead link seen from the other end.
 *
 *   node build/check-links.mjs
 */
import { readFileSync, readdirSync, statSync, existsSync } from "node:fs";
import { join, dirname, resolve, relative, sep } from "node:path";
import { fileURLToPath } from "node:url";

const DOCS = fileURLToPath(new URL("..", import.meta.url));
const REPO = resolve(DOCS, "..");

/** Directories that hold no documentation, or hold generated copies of it. */
const SKIP = new Set([
  "node_modules",
  "target",
  ".git",
  "dist",
  "coverage",
  ".vitepress",
  ".desloppify",
  ".claude",
  ".impeccable",
  "fuzz",
]);

function markdownFiles(dir, acc = []) {
  for (const entry of readdirSync(dir)) {
    if (SKIP.has(entry)) continue;
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) markdownFiles(full, acc);
    else if (entry.endsWith(".md")) acc.push(full);
  }
  return acc;
}

/** Published pages: every markdown file under docs/ that VitePress builds. */
function publishedPages() {
  return markdownFiles(DOCS)
    .filter((f) => !relative(DOCS, f).startsWith("internal" + sep))
    .filter((f) => !relative(DOCS, f).startsWith("build" + sep));
}

/**
 * Turn a link target into the file it names, or null if it names nothing.
 *
 * VitePress accepts four spellings of the same page — `./foo.md`, `./foo`,
 * `/guide/foo`, and a directory whose `index.md` is implied — so a checker that
 * only understood one of them would report as broken exactly the links that
 * work.
 */
function resolveTarget(fromFile, target) {
  const clean = target.split("#")[0].split("?")[0];
  if (!clean) return { ok: true };

  const inDocs = !relative(DOCS, fromFile).startsWith("..");
  const base = clean.startsWith("/")
    ? // Root-absolute inside the site means the site root; outside it, the repo.
      join(inDocs ? DOCS : REPO, clean)
    : resolve(dirname(fromFile), clean);

  const candidates = [
    base,
    base + ".md",
    join(base, "index.md"),
    // A repo-root reference to a page by its published path, e.g. a README
    // pointing at `docs/guide/installation.md`.
    base.replace(/\.html$/, ".md"),
  ];
  return { ok: candidates.some((c) => existsSync(c)) };
}

/**
 * Documents that record a state of the world rather than describe the current
 * one, and the *narrow* thing they are excused from.
 *
 * An RFC quoting `docs/configuration.md` is quoting the tree as it stood when
 * the argument was made; RFC 0005 quotes `docs/monitoring.md` and
 * `docs/post-mortem-template.md` precisely because they did not exist, which is
 * the defect it is reporting. "Fixing" any of those would falsify the record,
 * and a checker that demanded it would be asking for the wrong thing.
 *
 * So the exemption is by *kind of reference*, not by file. These documents are
 * excused from the code-span check — a quotation — and are still held to the
 * markdown-link check, because a link is something a reader clicks and a stale
 * one is broken whatever the document is. `rfc-0005-merge-conflicts.md` is the
 * exception to the exception: it is a generated `diff` of deleted files, so
 * even its `[text](target)` links are quotations of text that no longer exists.
 */
const QUOTES_HISTORY = [/^CHANGELOG\.md$/, /^todo\.md$/, /^docs\/rfc\//];
const GENERATED = [/^docs\/internal\/rfc-0005-merge-conflicts\.md$/];

const findings = [];
const linkedFromPages = new Set();

/** Nav and sidebar entries. Parsed rather than imported: config.ts is
 *  TypeScript with a Vite plugin in it, and this check must not need a build. */
const configSrc = readFileSync(join(DOCS, ".vitepress", "config.ts"), "utf8");
const reachable = new Set(
  [...configSrc.matchAll(/link:\s*"([^"]+)"/g)].map((m) => m[1]),
);

for (const file of markdownFiles(REPO)) {
  const src = readFileSync(file, "utf8");
  const rel = relative(REPO, file).split(sep).join("/");
  if (GENERATED.some((p) => p.test(rel))) continue;

  // Markdown links, minus the ones that do not name a file in this repository.
  for (const [, , target] of src.matchAll(/\[([^\]]*)\]\(([^)\s]+)\)/g)) {
    if (/^(https?:|mailto:|#|<)/.test(target)) continue;
    if (!resolveTarget(file, target).ok) {
      findings.push({ rel, kind: "dead link", target });
    }
    if (target.startsWith("/")) linkedFromPages.add(target.split("#")[0]);
  }

  // Inline code that is a path to a markdown file. Both of the references this
  // gate was written for were written this way, so the syntactic definition of
  // "link" is the one thing it must not adopt.
  if (QUOTES_HISTORY.some((p) => p.test(rel))) continue;
  for (const [, span] of src.matchAll(/`([^`\n]+)`/g)) {
    if (!span.includes("/") || !span.endsWith(".md")) continue;
    if (/[\s*<>|]/.test(span)) continue;
    // A code span is written for a reader of the repository, so it is a
    // repo-relative path far more often than a page-relative one — but both
    // spellings occur, and neither is wrong. Either resolving is a pass.
    const fromRepo = existsSync(join(REPO, span));
    if (!fromRepo && !resolveTarget(file, span).ok) {
      findings.push({ rel, kind: "dead path in code span", target: span });
    }
  }
}

// Orphans. A published page nothing points at is unreachable, which is a dead
// link seen from the other end.
for (const page of publishedPages()) {
  const rel = relative(DOCS, page).split(sep).join("/");
  if (rel === "index.md") continue; // the home page is the entry point
  const url = "/" + rel.replace(/(index)?\.md$/, "").replace(/\/$/, "/");
  const bare = "/" + rel.replace(/\.md$/, "");
  if (
    !reachable.has(url) &&
    !reachable.has(bare) &&
    !linkedFromPages.has(url) &&
    !linkedFromPages.has(bare)
  ) {
    findings.push({ rel: "docs/" + rel, kind: "orphan page", target: bare });
  }
}

if (findings.length) {
  console.error(`${findings.length} finding(s):\n`);
  for (const f of findings) console.error(`  ${f.kind}: ${f.rel} → ${f.target}`);
  process.exit(1);
}
console.log("every documentation cross-reference resolves, and no page is orphaned");
