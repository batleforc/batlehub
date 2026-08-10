import { describe, expect, it } from "vitest";

import { buildCsp } from "./csp.ts";

const API_ORIGIN = "https://api.example.com";

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
   * The API origin is allowed to widen `connect-src` and nothing else. Without
   * this, a future change that appended it to every directive — or to
   * `script-src` — would still satisfy the connect-src test above.
   */
  it("widens only connect-src for the API origin", () => {
    const parsed = directives(buildCsp(API_ORIGIN));
    const widened = Object.entries(parsed)
      .filter(([, sources]) => sources.some((source) => source === API_ORIGIN))
      .map(([name]) => name);
    expect(widened).toEqual(["connect-src"]);
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
