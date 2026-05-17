import { ref } from "vue";
import type { Highlighter } from "shiki";
import { createHighlighter } from "shiki";

let _highlighter: Highlighter | null = null;
let _promise: Promise<Highlighter> | null = null;
const _ready = ref(false);

function init(): Promise<Highlighter> {
  if (!_promise) {
    _promise = createHighlighter({
      themes: ["github-light", "github-dark"],
      langs: ["toml", "yaml", "bash", "ini", "json", "jsonc", "text"],
    }).then((h) => {
      _highlighter = h;
      _ready.value = true;
      return h;
    });
  }
  return _promise;
}

init();

export function useShiki() {
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

  return { highlight, ready: _ready };
}
