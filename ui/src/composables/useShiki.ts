import { ref } from "vue";
import type { HighlighterCore } from "shiki/core";
import { createHighlighterCore } from "shiki/core";
import { createJavaScriptRegexEngine } from "shiki/engine/javascript";

let _highlighter: HighlighterCore | null = null;
let _promise: Promise<HighlighterCore> | null = null;
const _ready = ref(false);

/**
 * The two themes, held to the console's own contrast floor.
 *
 * GitHub's default pair paints comments `#6a737d` in **both** renditions — the
 * one token colour that does not swap — and a code block sits on
 * `--ground-raised`, where that measures 3.95:1 in light and 4.00:1 in dark
 * against a 4.5:1 requirement. `const` and `=` failed too, at 3.77:1 in light.
 * Measured in a browser over a real README, not computed from the theme file.
 *
 * The high-contrast pair fixes every token but one: light's comment is `#66707b`
 * at 4.13:1, still short. `colorReplacements` is Shiki's own mechanism for
 * exactly this — the theme keeps its identity and one value is re-derived
 * against the ground it is actually painted on, which is the same rule DESIGN.md
 * applies when a token crosses renditions.
 *
 * Applied here rather than in `ReadmePanel`, because Shiki writes the colour
 * inline per token: there is no scope left in the DOM for CSS to target, and a
 * README is not the only surface — the Setup Guide's snippets are lit by the
 * same pair, and the console has one theme rather than two.
 */
const LIGHT_CONTRAST_FLOOR = {
  // 4.13:1 → 5.24:1 on `--ground-raised`. GitHub's own `fg.muted`, so the theme
  // still reads as itself.
  "#66707b": "#57606a",
};

async function themeWithFloor(
  mod: Promise<{ default: unknown }>,
  colorReplacements: Record<string, string>,
) {
  const theme = (await mod).default as Record<string, unknown>;
  return { ...theme, colorReplacements };
}

// Import only the languages/themes we actually use, via the fine-grained
// core API. The full `shiki` bundle pulls in every language and theme as
// separate chunks (~6 MB), even when only a handful are requested at runtime.
function init(): Promise<HighlighterCore> {
  _promise ??= createHighlighterCore({
    themes: [
      themeWithFloor(import("@shikijs/themes/github-light-high-contrast"), LIGHT_CONTRAST_FLOOR),
      import("@shikijs/themes/github-dark-high-contrast"),
    ],
    langs: [
      import("@shikijs/langs/toml"),
      import("@shikijs/langs/yaml"),
      import("@shikijs/langs/bash"),
      import("@shikijs/langs/ini"),
      import("@shikijs/langs/json"),
      import("@shikijs/langs/jsonc"),
      import("@shikijs/langs/xml"),
      import("@shikijs/langs/terraform"),
    ],
    engine: createJavaScriptRegexEngine(),
  })
    .then((h) => {
      _highlighter = h;
      _ready.value = true;
      return h;
    })
    .catch((e) => {
      // Clear the cached promise so a later call can try again. `??=` memoises
      // it forever otherwise, and a single failure — the themes and langs are
      // dynamic `import()`s, so a stale chunk hash 404s after a redeploy while a
      // tab is open — makes every subsequent call reject with the same settled
      // rejection, for the life of the page.
      _promise = null;
      throw e;
    });
  return _promise;
}

function highlight(code: string, lang: string): string {
  if (!_highlighter) return "";
  try {
    return _highlighter.codeToHtml(code, {
      lang,
      themes: { light: "github-light-high-contrast", dark: "github-dark-high-contrast" },
      defaultColor: false,
    });
  } catch {
    return "";
  }
}
export function useShiki() {
  // `catch`, not `void`: `void` on a promise does not handle its rejection, so
  // a failed startup import surfaced as an unhandled rejection at app start.
  // The console degrades to unhighlighted code blocks, which is the right
  // outcome and not one worth a banner.
  init().catch(() => {});
  return { highlight, ready: _ready };
}

// ── Languages a README may name ──────────────────────────────────────────────
//
// The eight above are the console's own snippets, loaded at startup because the
// Setup Guide shows them immediately. A README is somebody else's document and
// can fence anything, so these are loaded **on demand**: the first `bash` block
// in a README costs one chunk, and a package that fences nothing costs none.
//
// A static map rather than `import(`@shikijs/langs/${lang}`)`, because Vite has
// to see the specifier to emit a chunk for it — a template literal would either
// fail to resolve or pull all 361 languages into the graph.
const LAZY_LANGS: Record<string, () => Promise<unknown>> = {
  c: () => import("@shikijs/langs/c"),
  cpp: () => import("@shikijs/langs/cpp"),
  csharp: () => import("@shikijs/langs/csharp"),
  css: () => import("@shikijs/langs/css"),
  diff: () => import("@shikijs/langs/diff"),
  dockerfile: () => import("@shikijs/langs/dockerfile"),
  go: () => import("@shikijs/langs/go"),
  graphql: () => import("@shikijs/langs/graphql"),
  html: () => import("@shikijs/langs/html"),
  java: () => import("@shikijs/langs/java"),
  javascript: () => import("@shikijs/langs/javascript"),
  jsx: () => import("@shikijs/langs/jsx"),
  kotlin: () => import("@shikijs/langs/kotlin"),
  lua: () => import("@shikijs/langs/lua"),
  markdown: () => import("@shikijs/langs/markdown"),
  php: () => import("@shikijs/langs/php"),
  python: () => import("@shikijs/langs/python"),
  ruby: () => import("@shikijs/langs/ruby"),
  rust: () => import("@shikijs/langs/rust"),
  scala: () => import("@shikijs/langs/scala"),
  shellscript: () => import("@shikijs/langs/shellscript"),
  sql: () => import("@shikijs/langs/sql"),
  swift: () => import("@shikijs/langs/swift"),
  tsx: () => import("@shikijs/langs/tsx"),
  typescript: () => import("@shikijs/langs/typescript"),
};

/**
 * What people actually write in a fence, mapped to what Shiki calls it.
 *
 * `js` and `sh` are more common in a README than their full names, and a fence
 * Shiki does not recognise is not an error — it is a plain block. Getting the
 * alias right is the difference between a highlighted block and a grey one.
 */
const LANG_ALIASES: Record<string, string> = {
  "c#": "csharp",
  "c++": "cpp",
  cs: "csharp",
  console: "shellscript",
  golang: "go",
  htm: "html",
  js: "javascript",
  jsonl: "json",
  md: "markdown",
  mdx: "markdown",
  node: "javascript",
  py: "python",
  rb: "ruby",
  rs: "rust",
  sh: "shellscript",
  shell: "shellscript",
  text: "",
  plaintext: "",
  ts: "typescript",
  yml: "yaml",
  zsh: "shellscript",
};

/** Languages the startup bundle already carries. */
const EAGER_LANGS = new Set(["bash", "ini", "json", "jsonc", "terraform", "toml", "xml", "yaml"]);

const loaded = new Set<string>();

/**
 * Resolve a fence's language to one this highlighter can use, loading its chunk
 * if that is what it takes. `null` means "render it plain" — an unknown fence,
 * a deliberate `text`, or a chunk that failed to arrive.
 */
export async function ensureLang(raw: string): Promise<string | null> {
  const asked = raw.trim().toLowerCase();
  if (!asked) return null;
  const lang = LANG_ALIASES[asked] ?? asked;
  if (!lang) return null;
  if (EAGER_LANGS.has(lang) || loaded.has(lang)) return lang;

  const load = LAZY_LANGS[lang];
  if (!load) return null;

  // `init()` is inside the `try`, not before it. A highlighter that never
  // arrives is exactly the "chunk that failed to arrive" this function's own
  // doc comment promises to answer `null` for — and leaving the `await` outside
  // rethrew it into `highlightInto`, which rethrew into `ReadmePanel`'s
  // `paintRendered`, aborting at the first fenced block and rejecting the async
  // watcher callback: an unhandled rejection on every README render.
  try {
    const highlighter = await init();
    await highlighter.loadLanguage((await load()) as never);
    loaded.add(lang);
    return lang;
  } catch {
    return null;
  }
}

/**
 * Paint `code` into `el` as highlighted spans — **built as DOM nodes, never as
 * an HTML string.**
 *
 * This exists because the one place that needs it is a README: text written by
 * whoever published the package. `codeToHtml` would be safe (Shiki escapes what
 * it emits) and would still put a second `v-html` into the component that
 * documents why it has exactly one, and `ReadmePanel.test.ts` asserts that
 * count. Tokens sidestep the argument entirely: every character of package text
 * reaches the page through `textContent`, and the only thing this function sets
 * from Shiki is a colour on a custom property.
 *
 * Returns `false` when it painted nothing, so a caller can leave the plain text
 * it already had.
 */
export async function highlightInto(el: HTMLElement, code: string, raw: string): Promise<boolean> {
  const lang = await ensureLang(raw);
  if (!lang || !_highlighter) return false;

  let lines;
  try {
    ({ tokens: lines } = _highlighter.codeToTokens(code, {
      lang,
      themes: { light: "github-light-high-contrast", dark: "github-dark-high-contrast" },
      defaultColor: false,
    }));
  } catch {
    return false;
  }

  const frag = document.createDocumentFragment();
  lines.forEach((line, index) => {
    if (index > 0) frag.append(document.createTextNode("\n"));
    for (const token of line) {
      const span = document.createElement("span");
      span.textContent = token.content;
      // `htmlStyle` under a dual theme is `{"--shiki-light": "#…", "--shiki-dark": "#…"}`,
      // which is what `.shiki-wrapper .shiki span` in `index.css` already reads.
      for (const [prop, value] of Object.entries(token.htmlStyle ?? {})) {
        span.style.setProperty(prop, String(value));
      }
      frag.append(span);
    }
  });

  el.replaceChildren(frag);
  return true;
}
