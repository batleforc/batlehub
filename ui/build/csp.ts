/**
 * Content-Security-Policy for the SPA document.
 *
 * Lives in the document (a `<meta http-equiv>` substituted at build time) rather
 * than in a response header for two reasons that only show up on contact:
 *
 *  - It must NOT apply to `/scalar`. The API-docs page loads its bundle from a
 *    CDN and would break under `script-src 'self'`.
 *  - `actix_files::Files` is not a `ServiceFactory`, so it takes no per-service
 *    middleware — the static-file service cannot carry a header of its own.
 *
 * `connect-src` is derived from `VITE_API_BASE_URL`, the same build-time value
 * `src/config.ts` hands to the generated SDK. When that names a different origin
 * — the deployment `cors_allowed_origins` exists to support — a bare `'self'`
 * blocks every `fetch`, and the symptom reads as "the backend is down" rather
 * than "the CSP refused it". Deriving both from one variable keeps the policy
 * and the client from disagreeing.
 *
 * `style-src` needs `'unsafe-inline'`: the Vue/Tailwind build emits inline style
 * attributes and shiki injects inline styles for highlighting. Scripts get no
 * such exemption — the bundle is entirely external files, and that is the half
 * of this policy that stops an artifact served from the same origin from
 * executing.
 *
 * `frame-ancestors` is deliberately absent: it is ignored in meta form. The
 * server sends `X-Frame-Options: DENY` on every response instead — see
 * `crates/web/src/middleware/security_headers.rs`.
 *
 * `livePort` is the one thing that may widen `script-src`, and it exists for
 * Impeccable's live mode (RFC 0003 §7), which serves a helper script from
 * `http://localhost:<port>/live.js`. It is deliberately a *port number*, not an
 * origin or a source list: the only policy this argument can express is "one
 * localhost port", so no environment value can turn it into an arbitrary source.
 * `resolveLivePort` is what decides whether it may be non-null at all, and it
 * refuses in a production build.
 */
export function buildCsp(apiBaseUrl: string, livePort?: number | null): string {
  const connectSrc = ["'self'"];
  const trimmed = apiBaseUrl.trim();
  if (trimmed) {
    try {
      // Only an origin is a valid CSP source expression; leaving a path on it
      // would invalidate the directive rather than narrow it.
      connectSrc.push(new URL(trimmed).origin);
    } catch {
      // Relative base (e.g. "/api") — same-origin, already covered by 'self'.
    }
  }

  const scriptSrc = ["'self'"];
  const imgSrc = ["'self'", "data:"];
  if (isUsablePort(livePort)) {
    const liveOrigin = `http://localhost:${livePort}`;
    scriptSrc.push(liveOrigin);
    connectSrc.push(liveOrigin);
    // The live overlay screenshots the page through `URL.createObjectURL`,
    // which produces a `blob:` URL that `img-src 'self' data:` rejects.
    imgSrc.push("blob:");
  }

  return [
    "default-src 'self'",
    `script-src ${scriptSrc.join(" ")}`,
    "style-src 'self' 'unsafe-inline'",
    `img-src ${imgSrc.join(" ")}`,
    "font-src 'self' data:",
    `connect-src ${[...new Set(connectSrc)].join(" ")}`,
    "object-src 'none'",
    "base-uri 'self'",
    "form-action 'self'",
  ].join("; ");
}

function isUsablePort(port: number | null | undefined): port is number {
  return typeof port === "number" && Number.isInteger(port) && port >= 1 && port <= 65535;
}

/**
 * Decide whether the live-mode relaxation above applies to this build.
 *
 * Two independent conditions, both required, so neither a stray `.env` on a
 * build machine nor a wrong `--mode` alone can ship a widened policy:
 *
 *  1. the build is not a production build, and
 *  2. `VITE_IMPECCABLE_LIVE_PORT` names a valid port.
 *
 * Anything else — absent, empty, non-numeric, out of range, or a production
 * build regardless of the variable — yields `null`, and `buildCsp` then emits
 * exactly the policy it emitted before live mode existed.
 *
 * The digits-only test is not redundant with `parseInt`: `parseInt` stops at the
 * first non-digit, so `"4849 https://evil.test"` would otherwise parse to a
 * valid port and silently discard the rest. It cannot widen the policy either
 * way — `buildCsp` builds the origin from the number — but a value that is not
 * a port should be refused, not quietly truncated.
 */
export function resolveLivePort(
  mode: string,
  env: Record<string, string | undefined>,
): number | null {
  if (mode === "production") return null;
  const raw = (env.VITE_IMPECCABLE_LIVE_PORT ?? "").trim();
  if (!/^\d+$/.test(raw)) return null;
  const port = Number.parseInt(raw, 10);
  return isUsablePort(port) ? port : null;
}
