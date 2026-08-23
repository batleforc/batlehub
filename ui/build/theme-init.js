/**
 * The rendition, decided before the first paint.
 *
 * `tokens.css` makes dark the default: `:root` is the dark palette and light is
 * opt-in through `:root[data-theme="light"]`. `data-theme` is set by
 * `useColorMode` in `ThemeToggle.vue`, which cannot run until the bundle has
 * been fetched, parsed and mounted — so every reader whose preference is light
 * got a full-page flash of near-black first. On a cold cache that is most of a
 * second of the wrong rendition, on the page they asked for.
 *
 * ## Why a file rather than an inline script
 *
 * The document carries its own CSP (`ui/build/csp.ts`), which emits
 * `script-src 'self'` with no nonce and no hash — and `crates/web/src/spa.rs`
 * only ever *narrows* what the build produced, so nothing at runtime can widen
 * it. An inline script would simply not execute.
 *
 * A hash in both places would work and was the other option; this needs no
 * policy change at all, in either the TypeScript that builds it or the Rust
 * that serves it, which is one fewer pair of files that can drift apart.
 *
 * It is loaded **blocking** — a classic `<script src>` in `<head>`, no `defer`,
 * no `async`, and deliberately not `type="module"`, because module scripts are
 * deferred by definition and a deferred theme is the flash this file exists to
 * remove.
 *
 * ## Why it must agree with vueuse exactly
 *
 * This runs first and `useColorMode` runs a moment later, over the same
 * storage key. If the two disagree about any input the reader sees a *second*
 * flash at mount — the same defect moved rather than fixed. So the rules below
 * are vueuse's, not ours:
 *
 *   - the stored value is `auto | light | dark`, under `batlehub.theme`
 *     (`emitAuto: true`, so `auto` is a value that is really written);
 *   - anything that is not `light` or `dark` — absent, `auto`, corrupt —
 *     resolves from the system;
 *   - the system query is `(prefers-color-scheme: dark)`, and `usePreferredDark`
 *     reads *false* where `matchMedia` is unavailable, so light is the fallback
 *     there rather than the palette's own default;
 *   - the attribute is written with the resolved value, `light` or `dark`, not
 *     with the stored one.
 *
 * `theme-init.test.ts` runs this file against a fake document for each of those
 * cases; it is the only thing standing between this and a silent divergence the
 * next time `useColorMode` is configured differently.
 */
(function () {
  var STORAGE_KEY = "batlehub.theme";
  var stored = null;

  try {
    stored = window.localStorage.getItem(STORAGE_KEY);
  } catch (e) {
    // Storage can throw outright — Safari in private mode, a cookie policy that
    // blocks it. The system preference is still a better answer than the
    // palette's default, so this falls through rather than giving up.
  }

  var resolved =
    stored === "light" || stored === "dark"
      ? stored
      : window.matchMedia && window.matchMedia("(prefers-color-scheme: dark)").matches
        ? "dark"
        : "light";

  document.documentElement.setAttribute("data-theme", resolved);
})();
