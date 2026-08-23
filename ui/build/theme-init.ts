import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { minify, type Plugin } from "vite";

/** Where the browser fetches it, and the name the document hardcodes. */
export const THEME_INIT_URL = "/theme-init.js";

/**
 * The unprocessed source, as `theme-init.test.ts` also reads it.
 *
 * Resolved from the working directory rather than from `import.meta.url`, for
 * the reason `tokens.test.ts` records at the top of its own file: under Vite's
 * pipeline `import.meta.url` is an *http* URL, so `.pathname` yields
 * `/build/theme-init.js` and the read fails from the repo root. Both callers —
 * the plugin, via `vite.config.ts`, and the test, via vitest — run with `ui/`
 * as the working directory.
 */
export const themeInitSource = (): string => resolve(process.cwd(), "build/theme-init.js");

export const readThemeInit = (): string => readFileSync(themeInitSource(), "utf8");

/**
 * Ship `theme-init.js`, minified, at a stable URL.
 *
 * It is a *blocking* script in `<head>` — that is the entire point of it, and
 * why it cannot be part of the bundle — so every byte is a byte before the
 * first paint. The file is mostly comment, and the comment is load-bearing:
 * this script has to agree with `useColorMode` exactly or the flash it removes
 * comes back at mount. Both facts are true at once, which is what this plugin
 * is for: the reader gets the reasoning, the browser gets 268 bytes.
 *
 * `public/` was the obvious home and is the wrong one — Vite copies that
 * directory verbatim, so the shipped file would be the commented source and
 * minifying it would mean overwriting what the copy had just written. Emitting
 * it as an asset produces one file, once.
 *
 * `configureServer` is not a nicety: without it `/theme-init.js` 404s under
 * `vite dev`, and the dev console — which is what the rendered design gates
 * drive — would flash while production did not. Dev serves the source
 * unminified, as dev serves everything; `theme-init.test.ts` runs both forms
 * through the same cases, so the two cannot diverge in behaviour.
 */
export function themeInitPlugin(): Plugin {
  return {
    name: "batlehub-theme-init",

    configureServer(server) {
      server.middlewares.use((req, res, next) => {
        if (req.url?.split("?")[0] !== THEME_INIT_URL) return next();
        res.setHeader("Content-Type", "text/javascript");
        // Re-read per request: this file is edited rarely, and a stale copy
        // held across a dev session is a confusing thing to debug.
        res.end(readThemeInit());
      });
    },

    async generateBundle() {
      const { code } = await minify("theme-init.js", readThemeInit());
      this.emitFile({ type: "asset", fileName: THEME_INIT_URL.slice(1), source: code });
    },
  };
}
