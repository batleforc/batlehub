#!/usr/bin/env node
/**
 * Report user-visible text that is still hardcoded in templates.
 *
 * The RFC calls for a lint gate that "fails on literal user-visible text in
 * templates, so the catalogue cannot silently rot as pages are added". That gate
 * is only honest once the extraction is finished — turning it on mid-migration
 * would mean a permanently red build that everyone learns to ignore, which is
 * worse than no gate at all.
 *
 * So this reports by default and fails only with `--max <n>`, letting the
 * remaining count be ratcheted down as surfaces are converted, and pinned at 0
 * when Phase 8 closes.
 *
 *   node build/i18n-audit.mjs           # report
 *   node build/i18n-audit.mjs --max 0   # gate
 *
 * It is deliberately conservative: it looks only at text nodes and at the
 * attributes that are always read by a human, and skips anything that is plainly
 * not a sentence (identifiers, numbers, code, single symbols).
 */
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";

import { isTranslatable } from "./i18n-shared.mjs";

const ROOT = new URL("../src", import.meta.url).pathname;

/** Attributes whose value a person reads aloud or sees. */
const HUMAN_ATTRS = ["title", "placeholder", "aria-label", "alt"];

function walk(dir) {
  return readdirSync(dir).flatMap((entry) => {
    const path = join(dir, entry);
    if (statSync(path).isDirectory()) return entry === "client" ? [] : walk(path);
    return path.endsWith(".vue") ? [path] : [];
  });
}

function templateOf(source) {
  const open = source.indexOf("<template>");
  const close = source.lastIndexOf("</template>");
  return open === -1 || close === -1 ? "" : source.slice(open + 10, close);
}

function findings(path) {
  const source = readFileSync(path, "utf8");
  const template = templateOf(source)
    .replace(/<!--[\s\S]*?-->/g, "") // comments are not user-visible
    .replace(/<(pre|code)[\s\S]*?<\/\1\s*>/g, ""); // `</code\n>` is still a close tag // code samples are not prose

  /* Attribute *values* are stripped before splitting on tags: an expression like
     `v-if="count > 0"` contains a `>` that would otherwise end a tag early and
     leak the rest of the expression in as if it were prose. Human-facing
     attributes are matched separately, below, against the original template. */
  const textOnly = template.replace(/=\s*"[^"]*"/g, '=""').replace(/=\s*'[^']*'/g, "=''");

  const out = [];

  // Text nodes: everything between tags that is not an interpolation.
  for (const chunk of textOnly.split(/<[^>]*>/)) {
    const text = chunk.replace(/\{\{[\s\S]*?\}\}/g, "").trim();
    if (!text || text.length < 3) continue;
    if (!isTranslatable(text)) continue;
    out.push(text.length > 60 ? `${text.slice(0, 57)}…` : text);
  }

  // Human-facing attributes with a literal (non-bound) value.
  for (const attr of HUMAN_ATTRS) {
    const re = new RegExp(`(?<![:\\w-])${attr}="([^"{]+)"`, "g");
    for (const [, value] of template.matchAll(re)) {
      const text = value.trim();
      if (isTranslatable(text)) out.push(`${attr}="${text}"`);
    }
  }
  return out;
}

const maxIndex = process.argv.indexOf("--max");
const max = maxIndex === -1 ? null : Number(process.argv[maxIndex + 1]);

const report = walk(ROOT)
  .map((path) => ({ path: relative(ROOT, path), items: findings(path) }))
  .filter((entry) => entry.items.length > 0)
  .sort((a, b) => b.items.length - a.items.length);

const total = report.reduce((sum, entry) => sum + entry.items.length, 0);

for (const { path, items } of report) {
  console.log(`\n${path}  (${items.length})`);
  for (const item of items.slice(0, 6)) console.log(`  ${item}`);
  if (items.length > 6) console.log(`  … ${items.length - 6} more`);
}

console.log(`\n${total} untranslated strings across ${report.length} files`);

if (max !== null && total > max) {
  console.error(`\ni18n audit: ${total} untranslated strings exceeds the budget of ${max}`);
  process.exit(1);
}
