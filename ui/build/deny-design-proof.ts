/**
 * Takes the design proof out of what the dev server will serve.
 *
 * `ui/design-proof/index.html` is a standalone artefact — RFC 0003 Phase 1's
 * proving surface, checked in because it is runnable source rather than a
 * screenshot. It only *happens* to sit inside Vite's root (`ui/`), which made
 * `GET /design-proof/` answer 200 on every front, including 5173 — the one
 * reached through the workspace's public endpoint. A frozen, unauthenticated
 * second copy of the catalog, showing stub data under the product's own
 * masthead, on a public URL that no route or nav entry in the app knows about,
 * is not something the artefact was ever meant to be.
 *
 * A middleware, and deliberately **not** `server.fs.deny`, which is the obvious
 * home for it: a `deny` array in user config *replaces* Vite's defaults rather
 * than extending them, so declaring one pattern there served `ui/.env` with a
 * 200 (measured, not assumed). Copying the default list alongside it works and
 * then silently narrows the day Vite adds an entry — that list has already grown
 * from four patterns to six. Appending to the resolved list from
 * `configResolved` does nothing: the deny globs are compiled into a matcher
 * before the server starts. A middleware needs none of that and touches no
 * security default of Vite's.
 *
 * 404 rather than 403: this is not a permission boundary that some credential
 * could open, it is a file that is no longer part of the site.
 *
 * The artefact stays where `DESIGN.md`, the RFCs and the component comments
 * point at it, and stays runnable: open the file, or serve that one directory
 * (`npx serve ui/design-proof`). Only the console's own dev server stops handing
 * it out. The production build never contained it — `build` has a single entry
 * (`ui/index.html`) and the only directory copied verbatim is `ui/public/`.
 */
import type { Plugin } from "vite";

/** The path segment the artefact lives under, matched anywhere in the URL. */
const SEGMENT = /(^|\/)design-proof(\/|$)/;

/**
 * Whether a request URL addresses the design proof.
 *
 * Matched as a whole path *segment*, so it cannot be widened by a neighbour
 * (`/design-proofs/` is a different directory and stays servable) and cannot be
 * dodged by depth — `/design-proof/fonts/…` and the `/@fs/<abs path>/…`
 * absolute form Vite also accepts are the same rule.
 */
export function isDesignProofPath(url: string): boolean {
  const raw = url.split("?")[0].split("#")[0];
  /* The decoded form too, because `%2Fdesign-proof%2F` arrives at the static
     middleware decoded. `decodeURIComponent` throws on a malformed escape,
     which is not a reason to serve the file — the raw form still decides. */
  let decoded = raw;
  try {
    decoded = decodeURIComponent(raw);
  } catch {
    /* keep the raw form */
  }
  return SEGMENT.test(raw) || SEGMENT.test(decoded);
}

/**
 * Installed from `configureServer` without deferring, so it runs ahead of Vite's
 * static, HTML-transform and SPA-fallback middlewares. Deferred (by returning a
 * function) it would run after them, which is after the file has been served.
 */
export function denyDesignProofPlugin(): Plugin {
  return {
    name: "batlehub-deny-design-proof",
    configureServer(server) {
      server.middlewares.use((req, res, next) => {
        if (!isDesignProofPath(req.url ?? "")) return next();
        res.statusCode = 404;
        res.end("Not found");
      });
    },
  };
}
