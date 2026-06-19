import { defineConfig } from "vitest/config";
import vue from "@vitejs/plugin-vue";
import tailwindcss from "@tailwindcss/vite";
import path from "node:path";
import { readdirSync, existsSync } from "node:fs";

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
function coverageIncludeFromTests(): string[] {
  const srcDir = path.resolve(__dirname, "src");
  const rel = (p: string) => path.relative(__dirname, p).split(path.sep).join("/");
  const included = new Set<string>();

  const testFiles = readdirSync(srcDir, { recursive: true, encoding: "utf8" }).filter((f) =>
    /\.(test|spec)\.ts$/.test(f),
  );

  for (const testFile of testFiles) {
    const testPath = path.join(srcDir, testFile);
    const dir = path.dirname(testPath);

    if (rel(dir).startsWith("src/components/")) {
      for (const entry of readdirSync(dir)) {
        const isSource = /\.(vue|ts)$/.test(entry) && !/\.(test|spec)\.ts$/.test(entry);
        if (isSource && entry !== "index.ts") included.add(rel(path.join(dir, entry)));
      }
    } else {
      const base = path.basename(testPath).replace(/\.(test|spec)\.ts$/, "");
      for (const ext of [".vue", ".ts"]) {
        const candidate = path.join(dir, base + ext);
        if (existsSync(candidate)) {
          included.add(rel(candidate));
          break;
        }
      }
    }
  }

  return [...included].sort();
}

export default defineConfig({
  plugins: [vue(), tailwindcss()],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
  build: {
    outDir: "dist",
  },
  test: {
    environment: "jsdom",
    setupFiles: ["./src/test/setup.ts"],
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
});
