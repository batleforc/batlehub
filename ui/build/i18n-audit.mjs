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
 * Four places text hides, because it reached zero while three of them were
 * unscanned and most of the console was still English:
 *
 *   1. text nodes                  `<th>Registry</th>`
 *   2. human-facing attributes     `title="Dashboard"`, `label="Registries"`
 *   3. literals in expressions     `{{ busy ? 'Loading…' : 'Refresh' }}`
 *   4. literals in `<script>`      `{ label: "All Packages" }`
 *
 * (1) missed every single capitalised word, because the "is this an identifier"
 * test was case-insensitive and unanchored to spaces, so `Registries` and `You`
 * read as identifiers. (2) covered four HTML attributes but no component prop.
 * (3) could not see anything, because attribute values are blanked before the
 * tag split and interpolations are stripped. (4) was out of scope entirely,
 * which is how the whole admin navigation stayed English behind a green gate.
 *
 * It stays conservative about what counts as prose: identifiers, numbers, code,
 * single symbols, Tailwind class lists and the domain terms §4.6 keeps verbatim
 * are all skipped. A gate nobody can drive to zero is a gate nobody reads.
 */
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";

import { isTranslatable } from "./i18n-shared.mjs";

const ROOT = new URL("../src", import.meta.url).pathname;

/**
 * Attributes whose value a person reads aloud or sees. `label` and
 * `description` are here because a component prop is just as visible as an HTML
 * attribute: `<Facet label="Registries">` rendered the catalog's facet heading
 * in English in both locales.
 */
const HUMAN_ATTRS = ["title", "placeholder", "aria-label", "alt", "label", "description"];

/**
 * Object keys whose string value is rendered as a label somewhere. Templates
 * are not the only place user-visible text hides: a `label:` in a `<script>`
 * array reaches the screen through `{{ link.label }}` just as directly, and
 * scanning only templates is what let the whole admin navigation sit in English
 * while this gate reported zero.
 */
const LABEL_KEYS = ["label", "title"];

/**
 * Files whose string data the RFC keeps as data (§6.7): registry setup snippets
 * are tool names, config keys and CLI invocations, which §4.6 keeps verbatim in
 * both locales. Scanning them would bury a real finding under 200 that must not
 * be touched — and a gate nobody can get to zero is a gate nobody reads.
 */
const DATA_FILES = /(registryTypes|registryPathFields)\.ts$/;

function walk(dir) {
  return readdirSync(dir).flatMap((entry) => {
    const path = join(dir, entry);
    if (statSync(path).isDirectory()) return entry === "client" ? [] : walk(path);
    if (DATA_FILES.test(path) || /\.(test|spec)\.ts$/.test(path)) return [];
    return path.endsWith(".vue") || path.endsWith(".ts") ? [path] : [];
  });
}

function templateOf(source) {
  const open = source.indexOf("<template>");
  const close = source.lastIndexOf("</template>");
  return open === -1 || close === -1 ? "" : source.slice(open + 10, close);
}

/**
 * `label: "All Packages"` in a script. A value that is already a catalogue key
 * (`adminNav.users`) is the fixed form, and `isTranslatable` rejects it for
 * free — a dotted identifier is not prose.
 */
function scriptLabels(source) {
  const out = [];
  const script = source.includes("<script") ? source.slice(0, source.indexOf("<template>") + 1) : source;
  for (const key of LABEL_KEYS) {
    const re = new RegExp(`(?<![\\w.])${key}\\s*:\\s*"([^"\\\\]*)"`, "g");
    for (const [, value] of script.matchAll(re)) {
      const text = value.trim();
      if (isTranslatable(text)) out.push(`${key}: "${text}"`);
    }
  }
  return out;
}

/**
 * String literals inside a *bound* expression — `:title="hit ? 'A' : 'B'"`, or
 * an interpolation like `{{ ok ? 'Yes' : 'No' }}`.
 *
 * The text-node pass has to blank every attribute value before it splits on
 * tags (a `>` inside an expression would end the tag early), and it strips
 * interpolations, so literals in either place were invisible. That is where the
 * catalog's four empty states and the whole fresh-instance path were hiding.
 *
 * `:class` and `:style` are skipped: their literals are Tailwind class lists,
 * not prose, and they outnumber the real findings roughly twenty to one — which
 * would bury this gate rather than sharpen it.
 */
function boundLiterals(template) {
  const out = [];
  // A backtick literal may interpolate; its `${…}` parts are values, and what
  // is left around them is the sentence — `` `All registries (${n})` `` is
  // still English prose that needs a key.
  const literal = /'([^']{3,})'|`([^`]{3,})`/g;
  const strip = (text) => text.replace(/\$\{[^}]*\}/g, "").trim();

  for (const [, colonAttr, directive, value] of template.matchAll(
    /(?::([\w.-]+)|v-([\w:.-]+))="([^"]*)"/g,
  )) {
    const attr = colonAttr ?? directive ?? "";
    if (/^(class|style)$/.test(attr) || /^bind:(class|style)$/.test(attr)) continue;
    for (const [, single, backtick] of value.matchAll(literal)) {
      const text = strip(single ?? backtick);
      if (isTranslatable(text)) out.push(`:${attr}="… '${text}'"`);
    }
  }

  // Inside an interpolation both quote styles are fair game — it is not an
  // attribute value, so a double quote does not terminate anything.
  for (const [, expr] of template.matchAll(/\{\{([\s\S]*?)\}\}/g)) {
    for (const [, single, double] of expr.matchAll(/'([^']{3,})'|"([^"]{3,})"/g)) {
      const text = strip(single ?? double);
      if (isTranslatable(text)) out.push(`{{ … '${text}' }}`);
    }
  }
  return out;
}

function findings(path) {
  const source = readFileSync(path, "utf8");
  if (!path.endsWith(".vue")) return scriptLabels(source);
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
  out.push(...boundLiterals(template));
  out.push(...scriptLabels(source));
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
