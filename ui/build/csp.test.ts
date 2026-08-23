import { describe, expect, it } from "vitest";

import { buildCsp, resolveLivePort } from "./csp.ts";

const API_ORIGIN = "https://api.example.com";
const SOCKET_BADGE = "https://badge.socket.dev";

/**
 * Split a policy into `{ directive -> source tokens }`.
 *
 * Assertions go through this rather than substring-matching the whole policy
 * string. Matching `csp.includes(API_ORIGIN)` would also accept
 * `https://api.example.com.evil.test` — a substring check on a URL is exactly
 * the pattern CodeQL's `js/incomplete-url-substring-sanitization` flags, and it
 * is the wrong assertion here regardless: a CSP source is a whole token, so the
 * test should compare whole tokens.
 *
 * Comparisons against a token use an explicit `=== API_ORIGIN` rather than
 * `.includes(API_ORIGIN)`. The two are equivalent on an array of strings, but
 * that query matches any `.includes(<url>)` call without checking whether the
 * receiver is an array or a string, so the explicit equality is both what the
 * test means and the form that reads unambiguously.
 */
function directives(csp: string): Record<string, string[]> {
  return Object.fromEntries(
    csp.split("; ").map((directive) => {
      const [name, ...sources] = directive.split(" ");
      return [name, sources];
    }),
  );
}

/**
 * `buildCsp` decides whether the shipped SPA can talk to its API at all, and a
 * mistake here fails at runtime in the browser console rather than at build
 * time — so it is worth pinning even though it is build tooling.
 */
describe("buildCsp", () => {
  it("restricts connect-src to 'self' when no API base URL is configured", () => {
    expect(buildCsp("")).toContain("connect-src 'self';");
  });

  /**
   * The deployment `cors_allowed_origins` exists to support: UI on one origin,
   * API on another. A bare `connect-src 'self'` blocks every fetch, and the
   * symptom looks like a dead backend rather than a refused connection.
   */
  it("allows the API origin when VITE_API_BASE_URL names a different origin", () => {
    const csp = buildCsp("https://api.example.com");
    expect(csp).toContain("connect-src 'self' https://api.example.com;");
  });

  /** A CSP source is an origin; a path would invalidate the whole directive. */
  it("reduces a full URL to its origin", () => {
    expect(buildCsp("https://api.example.com/v1/base")).toContain(
      "connect-src 'self' https://api.example.com;",
    );
  });

  it("keeps a non-default port, which is part of the origin", () => {
    expect(buildCsp("http://localhost:8080")).toContain(
      "connect-src 'self' http://localhost:8080;",
    );
  });

  it("does not duplicate the origin when it is already same-origin-ish", () => {
    const sources = directives(buildCsp(API_ORIGIN))["connect-src"];
    expect(sources.filter((source) => source === API_ORIGIN)).toHaveLength(1);
  });

  /** A relative base is same-origin, which `'self'` already covers. */
  it("ignores a relative base URL", () => {
    expect(buildCsp("/api")).toContain("connect-src 'self';");
  });

  it("ignores surrounding whitespace", () => {
    expect(buildCsp("   ")).toContain("connect-src 'self';");
  });

  /**
   * Asserted with the trailing `;` on purpose, so the directive is pinned
   * *whole*. A bare `toContain("script-src 'self'")` is a substring match and
   * stays green if someone later widens it to `script-src 'self' 'unsafe-inline'`
   * — which is precisely the regression this test exists to block, and the one
   * an earlier version of it failed to catch.
   */
  it("never allows inline or remote script", () => {
    const csp = buildCsp("https://api.example.com");
    expect(csp).toContain("script-src 'self';");
    expect(csp).toContain("object-src 'none';");
    expect(csp).toContain("base-uri 'self';");
    expect(csp).toContain("default-src 'self';");
  });

  /**
   * Under `[registries.readme] remote_images = "proxy"` the server rewrites a
   * README's `<img>` tags to absolute URLs on **its own** origin — not the
   * third party's, and not necessarily the SPA's. On a split-origin deployment
   * that is not `'self'`, so without this every proxied README image is blocked
   * and the console reports a CSP violation naming this project's own backend.
   *
   * This test replaced one asserting the API origin widened `connect-src` and
   * nothing else. That assertion was the bug, pinned: it was written when the
   * API origin only ever served `fetch` responses, and it stayed green while
   * README images — added later, served from the same origin — were refused.
   */
  it("allows the API origin on img-src, for server-proxied README images", () => {
    expect(directives(buildCsp(API_ORIGIN))["img-src"]).toEqual([
      "'self'",
      "data:",
      SOCKET_BADGE,
      API_ORIGIN,
    ]);
  });

  /**
   * The API origin widens those two directives and nothing else. Without this,
   * a future change that appended it to every directive — or to `script-src` —
   * would still satisfy the two tests above.
   */
  it("widens only connect-src and img-src for the API origin", () => {
    const parsed = directives(buildCsp(API_ORIGIN));
    const widened = Object.entries(parsed)
      .filter(([, sources]) => sources.some((source) => source === API_ORIGIN))
      .map(([name]) => name);
    expect(widened).toEqual(["img-src", "connect-src"]);
  });

  /**
   * A same-origin deployment is the common case and must not grow a redundant
   * token: `'self'` already covers it, and a duplicated source in a policy is a
   * sign the two code paths disagree about what the origin is.
   */
  it("adds nothing when the API base is relative", () => {
    expect(directives(buildCsp("/api"))["img-src"]).toEqual(["'self'", "data:", SOCKET_BADGE]);
  });

  /**
   * `frame-ancestors` is ignored in meta form; clickjacking is covered by the
   * server's `X-Frame-Options: DENY`. Asserting its absence keeps someone from
   * "fixing" the omission and believing the page is framed-protected by the CSP.
   */
  it("omits frame-ancestors, which meta CSP cannot enforce", () => {
    expect(buildCsp("")).not.toContain("frame-ancestors");
  });
});

/**
 * Impeccable's live mode (RFC 0003 §7) loads a helper script from
 * `http://localhost:<port>/live.js`. It is the only thing allowed to widen
 * `script-src`, it is dev-only, and it is opt-in — so these tests pin both
 * halves: that the relaxation does what live mode needs, and that it cannot
 * reach a production build or express anything beyond one localhost port.
 */
describe("buildCsp — live mode", () => {
  const LIVE_ORIGIN = "http://localhost:4849";

  it("allows the live helper on script-src and connect-src", () => {
    const parsed = directives(buildCsp("", 4849));
    expect(parsed["script-src"]).toEqual(["'self'", LIVE_ORIGIN]);
    expect(parsed["connect-src"]).toEqual(["'self'", LIVE_ORIGIN]);
  });

  /** The overlay screenshots the page into a `blob:` URL. */
  it("allows blob: images for the live overlay", () => {
    expect(directives(buildCsp("", 4849))["img-src"]).toEqual([
      "'self'",
      "data:",
      SOCKET_BADGE,
      "blob:",
    ]);
  });

  it("widens nothing else", () => {
    const parsed = directives(buildCsp(API_ORIGIN, 4849));
    const widened = Object.entries(parsed)
      .filter(([, sources]) => sources.some((source) => source === LIVE_ORIGIN))
      .map(([name]) => name);
    expect(widened).toEqual(["script-src", "connect-src"]);
  });

  /**
   * The default call is what every build that has not opted in produces, and it
   * must be byte-identical to the policy from before live mode existed.
   */
  it.each([undefined, null, 0, -1, 65536, Number.NaN, 1.5] as const)(
    "emits the untouched policy for %p",
    (port) => {
      expect(buildCsp(API_ORIGIN, port as number | null | undefined)).toBe(buildCsp(API_ORIGIN));
    },
  );

  it("never widens script-src without an opt-in port", () => {
    expect(buildCsp(API_ORIGIN)).toContain("script-src 'self';");
  });
});

/**
 * The gate itself. `buildCsp` will happily widen for any port it is handed; the
 * guarantee that a shipped build is never handed one lives here.
 */
describe("resolveLivePort", () => {
  it("refuses in a production build even when the variable is set", () => {
    expect(resolveLivePort("production", { VITE_IMPECCABLE_LIVE_PORT: "4849" })).toBeNull();
  });

  it("returns the port for a development build that opted in", () => {
    expect(resolveLivePort("development", { VITE_IMPECCABLE_LIVE_PORT: "4849" })).toBe(4849);
  });

  it("returns null when the variable is absent", () => {
    expect(resolveLivePort("development", {})).toBeNull();
  });

  /**
   * The variable is a port, never a source expression. Anything that is not a
   * plain in-range integer is refused rather than coerced, so a value like
   * `"4849 https://evil.test"` cannot smuggle a second source into the policy.
   */
  it.each([
    "",
    "   ",
    "0",
    "-1",
    "65536",
    "not-a-port",
    "4849 https://evil.test",
    "'unsafe-inline'",
  ])("refuses the value %p", (value) => {
    expect(resolveLivePort("development", { VITE_IMPECCABLE_LIVE_PORT: value })).toBeNull();
  });
});

/**
 * The one third-party image origin, pinned.
 *
 * `socket_badge` is on by default and the version table renders one
 * `<img src="https://badge.socket.dev/…">` per row, so under the previous
 * `img-src 'self' data:` the feature was a broken-image box per row in every
 * deployment. The origin is now admitted — deliberately, and at a cost stated in
 * `csp.ts`: each badge tells socket.dev which package a reader is looking at.
 *
 * Pinned as a whole token and as an exact list, because both halves are the
 * assertion: that the badge can load, and that admitting it widened *nothing
 * else*. A substring check would accept `https://badge.socket.dev.evil.test`,
 * which is the pattern the tests at the top of this file already refuse for
 * `connect-src`.
 */
describe("buildCsp — the socket.dev badge", () => {
  it("admits the badge origin on img-src and nowhere else", () => {
    // Built with no API base URL, so this list is the badge's own contribution
    // and nothing else. The API origin joins `img-src` too — for the proxied
    // README images, asserted above — and building with one here would turn
    // this exact-list assertion into a test of two features at once, failing
    // whenever either changed.
    expect(directives(buildCsp(""))["img-src"]).toEqual(["'self'", "data:", SOCKET_BADGE]);

    const parsed = directives(buildCsp(API_ORIGIN));
    const elsewhere = Object.entries(parsed)
      .filter(
        ([name, sources]) =>
          name !== "img-src" && sources.some((source) => source === SOCKET_BADGE),
      )
      .map(([name]) => name);
    expect(elsewhere).toEqual([]);
  });

  /** It is a constant, not derived from any environment value. */
  it("admits it whatever the API base URL is", () => {
    for (const base of ["", "/api", API_ORIGIN, "https://elsewhere.example"]) {
      expect(directives(buildCsp(base))["img-src"]).toContain(SOCKET_BADGE);
    }
  });
});
