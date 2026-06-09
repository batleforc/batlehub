import { ref } from "vue";
import type { HighlighterCore } from "shiki/core";
import { createHighlighterCore } from "shiki/core";
import { createJavaScriptRegexEngine } from "shiki/engine/javascript";

let _highlighter: HighlighterCore | null = null;
let _promise: Promise<HighlighterCore> | null = null;
const _ready = ref(false);

// Import only the languages/themes we actually use, via the fine-grained
// core API. The full `shiki` bundle pulls in every language and theme as
// separate chunks (~6 MB), even when only a handful are requested at runtime.
function init(): Promise<HighlighterCore> {
  _promise ??= createHighlighterCore({
    themes: [import("@shikijs/themes/github-light"), import("@shikijs/themes/github-dark")],
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
  }).then((h) => {
    _highlighter = h;
    _ready.value = true;
    return h;
  });
  return _promise;
}

void init();
function highlight(code: string, lang: string): string {
  if (!_highlighter) return "";
  try {
    return _highlighter.codeToHtml(code, {
      lang,
      themes: { light: "github-light", dark: "github-dark" },
      defaultColor: false,
    });
  } catch {
    return "";
  }
}
export function useShiki() {
  return { highlight, ready: _ready };
}
