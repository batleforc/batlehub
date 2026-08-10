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
 */
export function buildCsp(apiBaseUrl: string): string {
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
  return [
    "default-src 'self'",
    "script-src 'self'",
    "style-src 'self' 'unsafe-inline'",
    "img-src 'self' data:",
    "font-src 'self' data:",
    `connect-src ${[...new Set(connectSrc)].join(" ")}`,
    "object-src 'none'",
    "base-uri 'self'",
    "form-action 'self'",
  ].join("; ");
}
