/**
 * Every ```mermaid fence in the tree parses.
 *
 * The site renders diagrams through `vitepress-plugin-mermaid`, which runs on
 * the client: a fence with a syntax error builds green, ships, and then draws
 * an error box in the reader's browser. Before the plugin was added the fences
 * were highlighted as source and nothing checked them at all, so this gate is
 * the first thing that has ever read them as diagrams.
 *
 * It parses rather than renders: `mermaid.parse` runs the grammar for the
 * detected diagram type, where rendering would mean a browser for an answer the
 * parser already has. It still needs a DOM, though — mermaid sanitises flowchart
 * labels through DOMPurify, which does nothing useful without one. That is why
 * `jsdom` is a devDependency here. Without it, every `flowchart` in the tree
 * fails with `DOMPurify.addHook is not a function` and every `sequenceDiagram`
 * passes, which looks exactly like 26 broken diagrams and is not.
 *
 * Scope is the whole repository's markdown, `internal/` included: an unpublished
 * page with a broken diagram is still a page someone reads in the editor.
 */
import { readFileSync, readdirSync, statSync } from "node:fs";
import { dirname, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { JSDOM } from "jsdom";

const DOCS = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const ROOT = resolve(DOCS, "..");
const SKIP = new Set(["node_modules", ".git", "dist", "target", "coverage", ".vitepress"]);

function* markdown(dir) {
  for (const entry of readdirSync(dir)) {
    if (SKIP.has(entry)) continue;
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) yield* markdown(full);
    else if (entry.endsWith(".md")) yield full;
  }
}

/** Fences, with the line the opening ``` sits on, so a failure is clickable. */
function fences(src) {
  const out = [];
  let current = null;
  src.split("\n").forEach((line, i) => {
    const trimmed = line.trim();
    if (current === null) {
      if (trimmed === "```mermaid") current = { line: i + 1, body: [] };
      return;
    }
    if (trimmed === "```") {
      out.push({ ...current, body: current.body.join("\n") });
      current = null;
      return;
    }
    current.body.push(line);
  });
  return out;
}

// A DOM, before mermaid is imported: it reads these globals as it loads.
// `navigator` is a getter-only property on Node's global object, hence the
// defineProperty rather than an assignment.
const dom = new JSDOM("<!doctype html><body></body>", { pretendToBeVisual: true });
global.window = dom.window;
global.document = dom.window.document;
global.Element = dom.window.Element;
global.HTMLElement = dom.window.HTMLElement;
global.SVGElement = dom.window.SVGElement;
global.getComputedStyle = dom.window.getComputedStyle;
Object.defineProperty(global, "navigator", {
  value: dom.window.navigator,
  configurable: true,
});

const mermaid = (await import("mermaid")).default;
mermaid.initialize({ startOnLoad: false });

const failures = [];
let total = 0;

for (const file of markdown(ROOT)) {
  const rel = relative(ROOT, file).split(sep).join("/");
  for (const fence of fences(readFileSync(file, "utf8"))) {
    total += 1;
    try {
      await mermaid.parse(fence.body);
    } catch (error) {
      const first = String(error?.message ?? error).split("\n").slice(0, 3).join(" ");
      failures.push(`${rel}:${fence.line} — ${first}`);
    }
  }
}

if (failures.length) {
  console.error("mermaid diagrams that do not parse:\n");
  for (const f of failures) console.error(`  ${f}`);
  console.error(
    `\n${failures.length} of ${total} would render as an error box in the reader's browser.`,
  );
  process.exit(1);
}

console.log(`every mermaid diagram parses — ${total} across the repository`);
