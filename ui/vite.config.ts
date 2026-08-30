import { defineConfig } from "vitest/config";
import { loadEnv, type Plugin } from "vite";
import vue from "@vitejs/plugin-vue";
import tailwindcss from "@tailwindcss/vite";
import path from "node:path";
import { readdirSync, existsSync } from "node:fs";

import { buildCsp, resolveLivePort } from "./build/csp.ts";
import { denyDesignProofPlugin } from "./build/deny-design-proof.ts";
import { themeInitPlugin } from "./build/theme-init.ts";

/** Injects the derived CSP into the `%VITE_CSP%` placeholder in index.html. */
function cspPlugin(apiBaseUrl: string, livePort: number | null): Plugin {
  return {
    name: "batlehub-csp",
    transformIndexHtml(html) {
      return html.replaceAll("%VITE_CSP%", buildCsp(apiBaseUrl, livePort));
    },
  };
}

/**
 * Derive the coverage allow-list from co-located test files, so it stays in sync
 * automatically (no hand-maintained list to forget to update). The 80% threshold
 * below applies to exactly this set — the source files that actually have tests.
 *
 * Mapping rule (matches the repo's two conventions):
 *  - Under `src/components/`, one test exercises a whole component directory, so
 *    every sibling source file (`.vue`/`.ts`, excluding the `index.ts` barrel) is
 *    included.
 *  - Everywhere else (composables, pages, lib, router) tests are 1:1 with source,
 *    so each `Foo.test.ts` maps to its exact sibling `Foo.vue` / `Foo.ts`.
 */
const rel = (p: string) => path.relative(__dirname, p).split(path.sep).join("/");

function collectSourcesForTest(testPath: string, included: Set<string>): void {
  const dir = path.dirname(testPath);

  if (rel(dir).startsWith("src/components/")) {
    for (const entry of readdirSync(dir)) {
      const isSource = /\.(vue|ts)$/.test(entry) && !/\.(test|spec)\.ts$/.test(entry);
      if (isSource && entry !== "index.ts") included.add(rel(path.join(dir, entry)));
    }
    return;
  }

  const base = path.basename(testPath).replace(/\.(test|spec)\.ts$/, "");
  for (const ext of [".vue", ".ts"]) {
    const candidate = path.join(dir, base + ext);
    if (existsSync(candidate)) {
      included.add(rel(candidate));
      break;
    }
  }
}

function coverageIncludeFromTests(): string[] {
  const srcDir = path.resolve(__dirname, "src");
  const included = new Set<string>();

  const testFiles = readdirSync(srcDir, { recursive: true, encoding: "utf8" }).filter((f) =>
    /\.(test|spec)\.ts$/.test(f),
  );

  for (const testFile of testFiles) {
    collectSourcesForTest(path.join(srcDir, testFile), included);
  }

  return [...included].sort((a, b) => a.localeCompare(b));
}

export default defineConfig(({ mode }) => {
  // `loadEnv` reads .env files the same way Vite does for `import.meta.env`, so
  // the CSP sees exactly the value the SDK will be built with.
  const env = loadEnv(mode, __dirname, "VITE_");

  return {
    plugins: [
      vue(),
      tailwindcss(),
      // The second argument is the only thing that can widen `script-src`, and
      // `resolveLivePort` returns null for every production build — see
      // `build/csp.ts` and RFC 0003 §7.
      cspPlugin(env.VITE_API_BASE_URL ?? "", resolveLivePort(mode, env)),
      // `ui/design-proof/` sits inside Vite's root without being part of the
      // console, so the dev server published it — see `build/deny-design-proof.ts`.
      denyDesignProofPlugin(),
      // The blocking <head> script that decides the rendition before the first
      // paint. Emitted rather than copied from `public/` so it ships minified
      // — every byte here is a byte before anything is drawn.
      themeInitPlugin(),
    ],
    resolve: {
      alias: {
        "@": path.resolve(__dirname, "./src"),
      },
    },
    server: {
      allowedHosts: [".cde.batleforc.fr", "localhost"],
      host: true,
      // No `fs.deny` here on purpose: a user-declared `deny` *replaces* Vite's
      // defaults rather than extending them, and `ui/.env` came back with a 200
      // the moment one was set. What this server refuses is a middleware
      // instead — `build/deny-design-proof.ts`.
    },
    build: {
      outDir: "dist",
    },
    test: {
      environment: "jsdom",
      setupFiles: ["./src/test/setup.ts"],
      // Mounting a radix Dialog through DialogPortal costs ~600 ms in isolation
      // and was measured past 8 s once the page suites run in parallel — CI
      // parallelises harder than any local run, so the 5 s default turns real
      // passes into flakes that look like product failures.
      testTimeout: 20_000,
      coverage: {
        provider: "v8",
        reporter: ["text", "lcov", "html"],
        // Auto-derived from co-located test files (see `coverageIncludeFromTests`);
        // the threshold below applies to this set, not the whole src/ tree. Adding a
        // co-located `*.test.ts` enrolls its source automatically.
        include: coverageIncludeFromTests(),
        thresholds: {
          lines: 80,
          branches: 80,
          functions: 80,
          statements: 80,
        },
      },
    },
  };
});
