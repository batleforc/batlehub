#!/usr/bin/env node
/**
 * The built SPA plus a stubbed read-only API, on one origin (RFC 0003 §13).
 *
 * The rendered gates need pages that *render*. The CI job served the build with
 * no API at all, so every page was an empty shell — six pages reported clean
 * while none of them existed. Running the real server instead would fix that
 * only halfway: a fresh backend is an **empty** backend, every admin list falls
 * to its empty state, and an empty page is exactly what we already know
 * measures nothing. Eight of the twenty-one endpoints returned `[]` even
 * against a populated development instance.
 *
 * So the fixtures are populated on purpose: rows are what expose contrast,
 * reflow, overflow and truncation. They were captured from a real backend and
 * their shapes are checked against `openapi.json` by `fixtures.test.ts`, so the
 * usual spec-sync gate catches drift.
 *
 * What this deliberately does not test: that the server returns these shapes.
 * That is the Rust integration tests' job, and the spec gate's. This serves
 * rendering, and only rendering.
 *
 *   node build/stub-server.mjs [--port 4173] [--dist ../dist]
 */
import { createServer } from "node:http";
import { readFile } from "node:fs/promises";
import { existsSync } from "node:fs";
import { extname, join, resolve, dirname, sep } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const arg = (name, fallback) => {
  const i = process.argv.indexOf(`--${name}`);
  return i === -1 ? fallback : process.argv[i + 1];
};

const PORT = Number(arg("port", 4173));
const DIST = resolve(arg("dist", join(HERE, "..", "dist")));
const FIXTURES = JSON.parse(await readFile(join(HERE, "fixtures", "captured.json"), "utf8"));

const TYPES = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".json": "application/json",
  ".svg": "image/svg+xml",
  ".png": "image/png",
  ".woff2": "font/woff2",
};

/**
 * Fixtures are keyed by the concrete path they were captured on, so a request
 * for a registry the fixtures do not name still has to answer. Falling back to
 * the first fixture whose shape matches the same route keeps every registry
 * rendering rows rather than an empty state.
 */
function lookup(pathname) {
  if (pathname in FIXTURES) return FIXTURES[pathname];
  const segments = pathname.split("/");
  for (const [key, value] of Object.entries(FIXTURES)) {
    const candidate = key.split("/");
    if (candidate.length !== segments.length) continue;
    // One differing segment is the path parameter (a registry name).
    if (candidate.filter((s, i) => s !== segments[i]).length === 1) return value;
  }
  return undefined;
}

/**
 * The file a request path is allowed to name, or the SPA shell.
 *
 * The path is canonicalised and then checked to still be *under* `dist` before
 * it can name a file. `new URL` already collapses `..`, so no request the
 * server can receive today is known to escape — but that is a property of the
 * parser, not of this function, and a caller that ever hands over a less
 * normalised string should not be the moment the containment starts mattering.
 * So the test sits directly on the canonicalised path, in the one expression
 * that lets caller-derived data become the return value: not stored off in a
 * boolean and re-combined with unrelated terms further down, where neither a
 * reader nor a taint analyser can tie it to the read it guards.
 *
 * Anything outside falls through to the shell, which is what an unknown route
 * gets anyway. `resolved === dist` needs no case of its own — that is the `/`
 * request, and `dist + sep` already excludes it.
 *
 * Exported for `src/test/stub-server.test.ts`: this is the security-relevant
 * line in the file, and it is only honestly testable apart from the parser.
 */
export function staticFileFor(dist, pathname) {
  const shell = join(dist, "index.html");
  const resolved = resolve(dist, `.${pathname}`);
  if (!resolved.startsWith(dist + sep)) return shell;
  return extname(resolved) && existsSync(resolved) ? resolved : shell;
}

const server = createServer(async (req, res) => {
  const url = new URL(req.url, `http://localhost:${PORT}`);

  if (url.pathname.startsWith("/api/")) {
    if (req.method !== "GET") {
      // Read-only: a gate that mutates state is a gate with an order dependency.
      res.writeHead(405, { "content-type": "application/json" });
      return res.end(JSON.stringify({ error: "stub is read-only" }));
    }
    const body = lookup(url.pathname);
    if (body === undefined) {
      res.writeHead(404, { "content-type": "application/json" });
      return res.end(JSON.stringify({ error: `no fixture for ${url.pathname}` }));
    }
    res.writeHead(200, { "content-type": "application/json" });
    return res.end(JSON.stringify(body));
  }

  // Static files, with the SPA fallback every client-side route needs.
  const file = staticFileFor(DIST, url.pathname);
  try {
    const data = await readFile(file);
    res.writeHead(200, { "content-type": TYPES[extname(file)] ?? "application/octet-stream" });
    res.end(data);
  } catch {
    res.writeHead(404).end("not found");
  }
});

// Only when run as the entry point: importing this module (the test does, for
// `staticFileFor`) must not bind a port.
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  server.listen(PORT, () => {
    console.log(`stub server: ${DIST} + ${Object.keys(FIXTURES).length} API fixtures on :${PORT}`);
  });
}
