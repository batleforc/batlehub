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
 *
 * The second half of this list is RFC 0004-bis §4.1. Four HTML attributes and
 * two props were not enough: `confirm-label="Clear Cache"` sat above a French
 * confirmation sentence and this gate read zero, because a *prop* carrying the
 * button's own text was not among the six names it knew. Adding names is the
 * narrow fix; the rule the audit actually needs is `looksHuman` below, and this
 * list is now only the set of names that are human text *whatever* they hold.
 */
const HUMAN_ATTRS = [
  "title",
  "placeholder",
  "aria-label",
  "alt",
  "label",
  "description",
  "empty-message",
  "confirm-label",
  "loading-label",
  "item-noun",
  "scope",
  "action",
  "value-text",
  "message",
  "hint",
  "heading",
  "summary",
  "legend",
];

/**
 * The rule, rather than the list.
 *
 * `HUMAN_ATTRS` enumerates positions text has been *found* in before, which is
 * why this gate has now read zero twice over live English — once for the whole
 * admin navigation (`adminSections.ts`'s header records it) and once for five
 * strings in two dialogs. A name ending in `-label`, `-message`, `-text`,
 * `-title`, `-hint` or `-description` is human-readable text by construction:
 * that is what those suffixes mean. Anything matching is scanned, and
 * `isTranslatable` is still the one that decides whether the value is prose.
 */
const looksHuman = (attr) =>
  HUMAN_ATTRS.includes(attr) ||
  /(^|-)(label|message|text|title|hint|description|caption|tooltip|placeholder)$/.test(attr);

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
 * The `<script>` region, comments and imports removed.
 *
 * Imports carry module specifiers, JSDoc carries English by design, and neither
 * reaches a screen — leaving them in buries the findings under the prose that
 * explains them.
 */
function scriptOf(source) {
  const region = source.includes("<template>") ? source.slice(0, source.indexOf("<template>")) : source;
  return region
    .replace(/\/\*[\s\S]*?\*\//g, "")
    .replace(/(^|[^:"'`\\])\/\/[^\n]*/g, "$1")
    .replace(/^\s*(import|export)\s[^;\n]*(from\s*["'][^"']*["'])?;?/gm, "");
}

/**
 * Text that reaches a human through a variable rather than through a tag.
 *
 * `parseError.value = "Paste some CSV content first."` renders in an alert two
 * lines later and no template scan can see it, which is how §2.1's examples
 * shipped behind a green gate. `isTranslatable` already returned `true` for
 * every one of them — the audit simply never asked it about this position.
 *
 * Two sinks: assignment (to a ref, a property, or a plain local) and a call
 * argument. `SILENT_SINK` is the one concession to enumeration, and it is
 * deliberately about *destinations that are not a screen* rather than about
 * shapes of text: a message handed to `console.error` or `new Error` is for
 * whoever reads the log, and translating it would make a stack trace harder to
 * search. `t(…)`/`te(…)` are excluded for the opposite reason — their argument
 * is a catalogue key, which is the fixed form this gate is driving toward.
 */
const SILENT_SINK =
  /(?:console\.\w+|new\s+Error|Error|URL|URLSearchParams|RegExp|JSON\.\w+|localStorage\.\w+|sessionStorage\.\w+|querySelector(?:All)?|getElementById|createElement|setAttribute|getAttribute|matchMedia|cva|cn|defineModel|defineEmits|\bt|\bte|\btm|\bd|\bn)\s*$/;

/**
 * Identifiers whose assigned string is machine-facing whatever it says: a route
 * path, a storage key, a CSS class list, a sort order, a CLI invocation. Matched
 * on the *target* rather than the value, because "asc" and "grid" are not prose
 * either way but `"Paste some CSV content first."` is.
 *
 * Two forms and no third: the bare word, or the word as a camelCase suffix
 * (`emptyPath`, `storageKey`). Matching a bare suffix would silence `valid` for
 * ending in `id` and `monkey` for ending in `key`, which is how a denylist
 * quietly turns into the same blind spot the list above exists to close.
 */
const SILENT_WORDS = [
  "class", "className", "classes", "style", "path", "href", "src", "url", "key", "id",
  "mode", "sort", "order", "type", "variant", "status", "state", "locale", "lang",
  "theme", "token", "method", "format", "code", "command", "snippet", "example",
  // HTTP header names. `Authorization: \`Bearer ${token}\`` strips to "Bearer",
  // which is prose by shape and a protocol constant by destination.
  "authorization", "accept", "header", "scheme",
];
const capitalise = (w) => w[0].toUpperCase() + w.slice(1);
// The suffix half is case-*sensitive* on purpose: `/id$/i` matches `valid`, and
// only the capital in `packageId` distinguishes a suffix from a coincidence.
const SILENT_SUFFIX = new RegExp(`[a-z0-9](?:${SILENT_WORDS.map(capitalise).join("|")})$`);
const SILENT_EXACT = new RegExp(`^(?:${SILENT_WORDS.join("|")})$`, "i");
const SILENT_TARGET = { test: (name) => SILENT_EXACT.test(name) || SILENT_SUFFIX.test(name) };

function scriptStrings(source) {
  const out = [];
  const script = scriptOf(source);
  // A `${…}` residue is a value, not a sentence — the same rule `boundLiterals`
  // applies. Without it a code snippet inside a backtick template reads as
  // prose the moment it contains a quoted attribute.
  const strip = (text) => text.replace(/\$\{[^}]*\}/g, "").trim();

  // Assignment: `x = "…"`, `x.value = "…"`, `obj.prop = "…"`.
  for (const [, target, single, double, backtick] of script.matchAll(
    /([\w.$[\]"']+)\s*=(?!=)\s*(?:"([^"\\\n]*)"|'([^'\\\n]*)'|`([^`\\]*)`)/g,
  )) {
    const text = strip(single ?? double ?? backtick);
    /* The name being assigned to, however it was reached: `parseError.value`,
       `row.error`, `headers["Authorization"]`. The last identifier in the
       expression is the one that names the destination. */
    const name = target.replace(/\.value$/, "").match(/[A-Za-z_$][\w$]*/g)?.pop() ?? target;
    if (SILENT_TARGET.test(name)) continue;
    if (isTranslatable(text)) out.push(`${target} = "${text}"`);
  }

  // Call argument: `setError("…")`, `announce("…")`, `ref("…")`. The callee is
  // read from the text *before* the paren — `match.index`, not `indexOf`, which
  // would resolve every repeat of a common call to its first occurrence.
  for (const match of script.matchAll(/\(\s*(?:"([^"\\\n]*)"|'([^'\\\n]*)'|`([^`\\]*)`)/g)) {
    const text = strip(match[1] ?? match[2] ?? match[3]);
    if (SILENT_SINK.test(script.slice(Math.max(0, match.index - 40), match.index))) continue;
    if (isTranslatable(text)) out.push(`(… "${text}")`);
  }

  /* A fallback expression: `x ?? "Unknown"`, `ok ? "Yes" : "No"`, `a || "—"`.
     This is where default human text hides — `errorMsg.value = e instanceof
     Error ? e.message : "Export failed"` is an assignment whose right-hand side
     is an expression, so the assignment sink above cannot see it, and it is the
     shape every "…or a default message" line takes. */
  for (const match of script.matchAll(
    /(?:\?\?|\?|\|\|)\s*(?:"([^"\\\n]*)"|'([^'\\\n]*)'|`([^`\\]*)`)/g,
  )) {
    const text = strip(match[1] ?? match[2] ?? match[3]);
    if (isTranslatable(text)) out.push(`… ?? "${text}"`);
  }

  /* Object-literal value: `dark: "Theme: dark"` in a lookup table. This is the
     §2.2 defect in its purest form — `theme.dark` was translated, correct, and
     referenced zero times while `ThemeToggle` held the English in a `LABELS`
     map. This replaces a pass that scanned `label:` and `title:` only, because
     those are the two keys text had been found under before; the key's *name*
     is not what makes the value text. */
  for (const [, name, single, double, backtick] of script.matchAll(
    /(?<![\w.$])([A-Za-z_$][\w$]*)\s*:\s*(?:"([^"\\\n]*)"|'([^'\\\n]*)'|`([^`\\]*)`)/g,
  )) {
    if (SILENT_TARGET.test(name)) continue;
    const text = strip(single ?? double ?? backtick);
    if (isTranslatable(text)) out.push(`${name}: "${text}"`);
  }

  return [...new Set(out)];
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
  /* `*`, not `{3,}`: a minimum length here makes the engine mis-pair quotes.
     Against `? "…" : "Save"` it skips the one-character `"…"`, resumes at that
     literal's *closing* quote, and matches `" : "` — swallowing `"Save"`
     entirely. Length is `isProse`'s job; this regex's only job is finding where
     a literal starts and ends. */
  const literal = /'([^']*)'|`([^`]*)`/g;
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

  /* Inside an interpolation all three quote styles are fair game — it is not an
     attribute value, so a double quote does not terminate anything, and a
     backtick is the one an interpolated sentence is most likely to use.
     Omitting the backtick is how `` `Sign in with ${providerLabel(p.name)}` ``
     rendered English on the login page with this gate reading zero. */
  for (const [, expr] of template.matchAll(/\{\{([\s\S]*?)\}\}/g)) {
    for (const [, single, double, backtick] of expr.matchAll(
      // See the note on `literal` above: a length floor here mis-pairs quotes.
      /'([^']*)'|"([^"]*)"|`([^`]*)`/g,
    )) {
      const text = strip(single ?? double ?? backtick);
      if (isTranslatable(text)) out.push(`{{ … '${text}' }}`);
    }
  }
  return out;
}

/**
 * Removes every match of every pattern, until *no* pattern matches any more.
 *
 * One pass is not enough when deleting a match splices its neighbours into a
 * new one — `<!-` + `<!-- -->` + `-` leaves `<!--` behind, and the `>` inside
 * the survivor then splits it, so the comment that was meant to be dropped
 * comes back as prose the scanner grades. Code scanning flagged the single pass
 * as incomplete multi-character sanitisation.
 *
 * The loop has to span the patterns rather than sit inside each one. Stripping
 * comments and then code samples in two separate fixed points leaves the second
 * to re-form the first: `<!-<code>x</code>-- … > … -->` has no comment in it
 * until the code sample is removed, and by then the comment pass is over.
 */
function stripAll(text, ...patterns) {
  let out = text;
  for (let prev; prev !== out; ) {
    prev = out;
    for (const re of patterns) out = out.replace(re, "");
  }
  return out;
}

/**
 * Every untranslated string in one file's source. Exported so the gate can be
 * tested on a fixture rather than on the tree it grades — a scanner that cannot
 * be shown to *fail* is the same green as one that found nothing, which is the
 * failure mode this whole file exists to close (RFC 0004-bis §2).
 *
 * @param source the file contents
 * @param isVue  whether to scan a `<template>` as well as the `<script>`
 */
export function scanSource(source, isVue = true) {
  if (!isVue) return scriptStrings(source);
  // Comments are not user-visible; code samples are not prose. `</code\n>` is
  // still a close tag.
  const template = stripAll(
    templateOf(source),
    /<!--[\s\S]*?-->/g,
    /<(pre|code)[\s\S]*?<\/\1\s*>/g,
  );

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

  /* Human-facing attributes with a literal (non-bound) value. Every attribute
     is matched and `looksHuman` decides which are text — iterating a fixed list
     of names can only ever find text where text has already been found, and
     that is exactly how `confirm-label="Clear Cache"` shipped. */
  for (const [, attr, value] of template.matchAll(/(?<![:\w-])([a-zA-Z][\w-]*)="([^"{]*)"/g)) {
    if (!looksHuman(attr)) continue;
    const text = value.trim();
    if (isTranslatable(text)) out.push(`${attr}="${text}"`);
  }
  out.push(...boundLiterals(template));
  out.push(...scriptStrings(source));
  return out;
}

const findings = (path) => scanSource(readFileSync(path, "utf8"), path.endsWith(".vue"));

// Importing this module must not run the report — `i18n-audit.test.ts` imports
// `scanSource` and would otherwise walk the whole tree on every test run.
if (process.argv[1] && process.argv[1].endsWith("i18n-audit.mjs")) main();

function main() {
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
}
