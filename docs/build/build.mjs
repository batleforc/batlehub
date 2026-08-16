#!/usr/bin/env node
/**
 * `vitepress build`, with server-render failures treated as failures.
 *
 * VitePress catches an exception thrown while server-rendering a page, prints
 * the stack, and carries on to exit 0. The page still ships — Vue re-renders it
 * in the browser — so nothing looks wrong to anyone with JavaScript running.
 * What actually happened is that the page's HTML no longer contains its
 * content: a search engine, a text browser, and the site's own local search
 * index all see the empty shell.
 *
 * Two pages were doing this the moment RFC 0005 published them, both for the
 * same reason: VitePress runs markdown through Vue, so a `{{ … }}` in prose is
 * an interpolation even inside a code span. `guide/security-scanning.md` quotes
 * a GitHub Actions expression and `rfc/0003-ui-rework.md` quotes two Vue
 * templates. The fix is `<code v-pre>`; this is what makes the next one
 * impossible to miss.
 */
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";

const ROOT = fileURLToPath(new URL("..", import.meta.url));
const SSR_FAILURE = /^(?:\w*Error|Uncaught)\b/;

const child = spawn("vitepress", ["build", ...process.argv.slice(2)], {
  cwd: ROOT,
  shell: true,
  stdio: ["inherit", "pipe", "pipe"],
});

const failures = [];
for (const stream of [child.stdout, child.stderr]) {
  let carry = "";
  stream.on("data", (chunk) => {
    process.stdout.write(chunk);
    const lines = (carry + chunk).split("\n");
    carry = lines.pop() ?? "";
    for (const line of lines) if (SSR_FAILURE.test(line)) failures.push(line);
  });
}

child.on("close", (code) => {
  if (code !== 0) process.exit(code ?? 1);
  if (failures.length) {
    console.error(
      `\n${failures.length} page(s) failed to server-render. VitePress exits 0 on ` +
        `these and ships the page as an empty shell — see docs/build/build.mjs.\n`,
    );
    for (const f of new Set(failures)) console.error(`  ${f}`);
    process.exit(1);
  }
});
