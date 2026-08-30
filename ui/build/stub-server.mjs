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
import { fileURLToPath } from "node:url";

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

createServer(async (req, res) => {
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
  //
  // The request path is resolved and then checked to still be *under* `DIST`
  // before it is allowed to name a file. `new URL` already collapses `..`, so
  // nothing here is known to escape today — but that is a property of the
  // parser rather than of this handler, and it is the containment check, not
  // the parser, that the next person reading this can verify. Anything outside
  // falls through to the SPA shell, which is what an unknown route gets anyway.
  const requested = resolve(DIST, `.${url.pathname}`);
  const inDist = requested === DIST || requested.startsWith(DIST + sep);
  const file =
    inDist && url.pathname !== "/" && extname(requested) && existsSync(requested)
      ? requested
      : join(DIST, "index.html");
  try {
    const data = await readFile(file);
    res.writeHead(200, { "content-type": TYPES[extname(file)] ?? "application/octet-stream" });
    res.end(data);
  } catch {
    res.writeHead(404).end("not found");
  }
}).listen(PORT, () => {
  console.log(`stub server: ${DIST} + ${Object.keys(FIXTURES).length} API fixtures on :${PORT}`);
});
