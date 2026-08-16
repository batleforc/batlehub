import { describe, it, expect, vi } from "vitest";

// The router transitively imports `useAuth`, which calls the generated SDK's
// `me()` at module load. Mock the SDK + client so no real `fetch` runs — this
// file only ever asks the router to *resolve* paths, never to navigate.
vi.mock("@/client/sdk.gen", () => ({ me: vi.fn(), oidcRefresh: vi.fn() }));
vi.mock("@/client/client.gen", () => ({ client: { setConfig: vi.fn() } }));

import { router } from "@/router";

/**
 * Every path the console links to must *resolve* to a route.
 *
 * `RouterLink` resolves its `href` statically, at render time. A path handled
 * only by the `beforeEach` guard — a `LEGACY_REDIRECTS` key, a section index —
 * therefore has no match to resolve against: Vue Router logs
 * `[VUE_ROUTER_R0004] No match found for location with path "…"` and emits a
 * wrong `href`. The *click* still works, because the guard catches the
 * navigation, which is precisely why this defect is invisible in use: only
 * middle-click, open-in-new-tab and copy-link-address are broken, and nobody
 * tries those on the page they are building.
 *
 * `HomePage` shipped `<RouterLink to="/explore">` that way. `/explore` is an
 * incoming legacy address for old bookmarks and links in `docs/`; the guard is
 * what keeps it alive, and the home page had no business pointing at it. The
 * six admin section roots avoid the same trap by being registered as real
 * `redirect` routes generated from `SECTION_INDEXES` — this test is what keeps
 * the next one from being added without them.
 *
 * Static targets only. A `:to` bound to an expression or a template literal is
 * built at runtime from data this test cannot see, so it is skipped rather than
 * guessed at.
 */
describe("internal link targets", () => {
  /* `process.cwd()` is `ui/` under vitest — the same anchor
     `src/test/fixtures.test.ts` uses to read `openapi.json`. */
  const SOURCE_ROOT = `${process.cwd()}/src`;

  /** `to="/x"`, `to: "/x"`, `router.push("/x")`, `router.push({ path: "/x" })`. */
  const TARGET = /(?:\bto=["']|\bto:\s*["']|\.push\(\s*["']|\bpath:\s*["'])(\/[^"'`]*)["']/g;

  async function sourceFiles(dir: string): Promise<string[]> {
    const { readdir } = await import("node:fs/promises");
    const entries = await readdir(dir, { withFileTypes: true });
    const out: string[] = [];
    for (const entry of entries) {
      const full = `${dir}/${entry.name}`;
      // `client/` is generated; `router/` declares the routes rather than
      // linking to them, and its own tests name paths that are meant not to
      // resolve.
      if (entry.isDirectory()) {
        if (entry.name === "client" || entry.name === "router") continue;
        out.push(...(await sourceFiles(full)));
      } else if (/\.(vue|ts)$/.test(entry.name) && !entry.name.endsWith(".test.ts")) {
        out.push(full);
      }
    }
    return out;
  }

  it("resolves every static link target to a real route", async () => {
    const { readFile } = await import("node:fs/promises");
    const files = await sourceFiles(SOURCE_ROOT);
    const unresolved: string[] = [];

    for (const file of files) {
      const source = await readFile(file, "utf8");
      for (const [, target] of source.matchAll(TARGET)) {
        // `/` is the home route; `//` would be a protocol-relative URL.
        if (target.startsWith("//")) continue;
        if (router.resolve(target).matched.length === 0) {
          unresolved.push(`${file.replace(`${SOURCE_ROOT}/`, "")} → ${target}`);
        }
      }
    }

    expect(
      [...new Set(unresolved)].sort(),
      "these paths are linked but match no route — RouterLink cannot build an href for them",
    ).toEqual([]);
  });
});
