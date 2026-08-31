import { spawn, type ChildProcess } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
// @ts-expect-error — a build script, not part of the typed src tree.
import { staticFileFor } from "../../build/stub-server.mjs";

/**
 * `build/stub-server.mjs` serves files named by the request path, so the one
 * thing it must never do is hand out a file from outside the dist it was
 * pointed at.
 *
 * Two levels, on purpose. `staticFileFor` is called with the raw strings a
 * URL parser would never produce, because going through the server only ever
 * proves what `new URL` normalises — those cases pass with the containment
 * check deleted, which makes them worthless as a guard on it. The HTTP cases
 * below are the end-to-end shape (assets serve, unknown routes get the shell),
 * not the security assertion.
 */

// The `fetch` calls below carry a `nosemgrep` for react-insecure-request: the
// server is spawned on loopback by this file and there is no TLS to speak.
const SERVER = resolve(process.cwd(), "build/stub-server.mjs");
const PORT = 47_312;
const base = `http://127.0.0.1:${PORT}`;

let sandbox: string;
let dist: string;
let child: ChildProcess;

beforeAll(async () => {
  sandbox = mkdtempSync(join(tmpdir(), "stub-server-"));
  dist = join(sandbox, "dist");
  mkdirSync(join(dist, "assets"), { recursive: true });
  writeFileSync(join(dist, "index.html"), "<!doctype html><title>shell</title>");
  writeFileSync(join(dist, "assets", "app.js"), "export const ok = true;\n");
  // The prize: a real file beside the dist, reachable only by escaping it.
  writeFileSync(join(sandbox, "secret.js"), "export const leaked = true;\n");

  child = spawn(process.execPath, [SERVER, "--port", String(PORT), "--dist", dist], {
    stdio: "ignore",
  });
  for (let i = 0; i < 100; i++) {
    try {
      // nosemgrep: typescript.react.security.react-insecure-request.react-insecure-request
      await fetch(base + "/");
      return;
    } catch {
      await new Promise((r) => setTimeout(r, 50));
    }
  }
  throw new Error("stub server did not start");
});

afterAll(() => {
  child?.kill();
  rmSync(sandbox, { recursive: true, force: true });
});

describe("staticFileFor", () => {
  const shell = () => join(dist, "index.html");

  it("names a file inside the dist", () => {
    expect(staticFileFor(dist, "/assets/app.js")).toBe(join(dist, "assets", "app.js"));
  });

  it.each([
    ["traversal to a real neighbour", "/../secret.js"],
    ["traversal through a real dir", "/assets/../../secret.js"],
    ["deep traversal", "/../../../../../../etc/passwd"],
    ["dist-prefix sibling", "/../dist-backup/secret.js"],
  ])("refuses to leave the dist: %s", (_name, pathname) => {
    expect(staticFileFor(dist, pathname)).toBe(shell());
  });

  it.each([
    ["the root", "/"],
    ["a client-side route", "/registries/npm"],
    ["a file that is not there", "/assets/missing.js"],
  ])("falls back to the shell for %s", (_name, pathname) => {
    expect(staticFileFor(dist, pathname)).toBe(shell());
  });
});

describe("stub server over HTTP", () => {
  const body = async (path: string) => (await fetch(base + path)).text();

  it("serves files from the dist", async () => {
    // nosemgrep: typescript.react.security.react-insecure-request.react-insecure-request
    const res = await fetch(base + "/assets/app.js");
    expect(res.status).toBe(200);
    expect(res.headers.get("content-type")).toContain("text/javascript");
    expect(await res.text()).toContain("ok = true");
  });

  it("falls back to the SPA shell for client-side routes", async () => {
    expect(await body("/")).toContain("shell");
    expect(await body("/registries/npm")).toContain("shell");
  });

  it("answers API paths from the fixtures", async () => {
    // nosemgrep: typescript.react.security.react-insecure-request.react-insecure-request
    const res = await fetch(base + "/api/v1/registries");
    expect(res.status).toBe(200);
    expect(Array.isArray(await res.json())).toBe(true);
  });

  it.each(["/../secret.js", "/..%2fsecret.js", "/%2e%2e/secret.js"])(
    "never leaks a neighbour file: %s",
    async (path) => {
      expect(await body(path)).not.toContain("leaked");
    },
  );
});
