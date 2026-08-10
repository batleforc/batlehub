import { describe, expect, it } from "vitest";

import { buildCsp } from "./csp.ts";

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
    const csp = buildCsp("https://api.example.com");
    expect(csp.match(/https:\/\/api\.example\.com/g)).toHaveLength(1);
  });

  /** A relative base is same-origin, which `'self'` already covers. */
  it("ignores a relative base URL", () => {
    expect(buildCsp("/api")).toContain("connect-src 'self';");
  });

  it("ignores surrounding whitespace", () => {
    expect(buildCsp("   ")).toContain("connect-src 'self';");
  });

  it("never allows inline or remote script", () => {
    const csp = buildCsp("https://api.example.com");
    expect(csp).toContain("script-src 'self'");
    expect(csp).not.toContain("'unsafe-inline'; script-src");
    expect(csp).toContain("object-src 'none'");
    expect(csp).toContain("base-uri 'self'");
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
