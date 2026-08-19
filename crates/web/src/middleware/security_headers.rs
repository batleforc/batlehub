//! Baseline security response headers, applied to every response.
//!
//! BatleHub serves three very different things from one origin: the admin SPA
//! (which holds bearer tokens in `localStorage`), the JSON API, and artifact
//! bytes that outsiders control — raw repository files, npm tarballs, `.vsix`
//! bundles. Same-origin is what makes that combination worth hardening: anything
//! the browser can be talked into *rendering* from an artifact URL runs with the
//! SPA's origin, and therefore its storage.
//!
//! Three headers, chosen because they are unconditionally safe for all three
//! surfaces:
//!
//! - `X-Content-Type-Options: nosniff` — the important one. Stops the browser
//!   second-guessing the declared `Content-Type`, which is what turns an artifact
//!   containing HTML into a document. Pairs with `proxy_stream`'s
//!   `application/octet-stream` default; either alone leaves a gap.
//! - `X-Frame-Options: DENY` — no part of this app is meant to be framed, so
//!   clickjacking against the admin UI has no legitimate use case to preserve.
//! - `Referrer-Policy: no-referrer` — package coordinates are in the path, and
//!   registry URLs are often internal. Nothing here should leak to a third party
//!   through a `Referer`.
//!
//! **Deliberately not set here:**
//!
//! - `Strict-Transport-Security` — TLS terminates at the ingress; the server
//!   itself usually speaks plaintext HTTP, and emitting HSTS from behind a
//!   proxy is how you pin a browser to `https://` for a host that cannot serve
//!   it. Set it on the ingress instead (the chart's `ingress.annotations`
//!   carries a documented example).
//! - `Content-Security-Policy` — it cannot be global: the Scalar API-docs page at
//!   `/scalar` loads its bundle from a CDN and would break under a
//!   `script-src 'self'` policy. It also cannot be attached to the static-file
//!   service alone, because `actix_files::Files` is not a `ServiceFactory` and
//!   takes no middleware. The SPA therefore declares its own policy in a
//!   `<meta http-equiv="Content-Security-Policy">` tag, built at build time by
//!   `ui/build/csp.ts` so `connect-src` can track the configured API origin —
//!   and *narrowed to the running config* when the document is served, by
//!   `crate::spa`, which can subtract sources but never add one.
//!   `frame-ancestors` is ignored in meta form, which is precisely why
//!   `X-Frame-Options` is sent here for every response.
//!
//! [`DefaultHeaders`] only inserts a header the response does not already carry,
//! so a handler that sets its own value (the OIDC callback's stricter
//! `Referrer-Policy`, for instance) keeps it.

use actix_web::http::header::{HeaderName, HeaderValue};
use actix_web::middleware::DefaultHeaders;

/// Baseline headers for every response. Register near-outermost so it also
/// covers responses produced by inner middleware (the IP-block `403`, the rate
/// limiter's `429`) and by the static-file service, not just handler output.
pub fn security_headers() -> DefaultHeaders {
    DefaultHeaders::new()
        .add((
            HeaderName::from_static("x-content-type-options"),
            HeaderValue::from_static("nosniff"),
        ))
        .add((
            HeaderName::from_static("x-frame-options"),
            HeaderValue::from_static("DENY"),
        ))
        .add((
            HeaderName::from_static("referrer-policy"),
            HeaderValue::from_static("no-referrer"),
        ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::{http::header, test, web, App, HttpResponse};

    #[actix_web::test]
    async fn baseline_headers_are_present_on_a_normal_response() {
        let app = test::init_service(App::new().wrap(security_headers()).route(
            "/",
            web::get().to(|| async { HttpResponse::Ok().body("x") }),
        ))
        .await;

        let resp = test::call_service(&app, test::TestRequest::get().uri("/").to_request()).await;
        let h = resp.headers();
        assert_eq!(h.get("x-content-type-options").unwrap(), "nosniff");
        assert_eq!(h.get("x-frame-options").unwrap(), "DENY");
        assert_eq!(h.get("referrer-policy").unwrap(), "no-referrer");
    }

    /// The headers have to reach error responses too — a `403` body is still a
    /// body the browser may be talked into rendering.
    #[actix_web::test]
    async fn baseline_headers_are_present_on_an_error_response() {
        let app = test::init_service(App::new().wrap(security_headers()).route(
            "/",
            web::get().to(|| async { HttpResponse::Forbidden().body("nope") }),
        ))
        .await;

        let resp = test::call_service(&app, test::TestRequest::get().uri("/").to_request()).await;
        assert_eq!(resp.status(), actix_web::http::StatusCode::FORBIDDEN);
        assert_eq!(
            resp.headers().get("x-content-type-options").unwrap(),
            "nosniff"
        );
    }

    /// `DefaultHeaders` must not clobber a handler that set a stricter value of
    /// its own — the OIDC callback relies on this.
    #[actix_web::test]
    async fn handler_supplied_value_wins() {
        let app = test::init_service(App::new().wrap(security_headers()).route(
            "/",
            web::get().to(|| async {
                HttpResponse::Ok()
                    .insert_header((header::REFERRER_POLICY, "same-origin"))
                    .body("x")
            }),
        ))
        .await;

        let resp = test::call_service(&app, test::TestRequest::get().uri("/").to_request()).await;
        assert_eq!(
            resp.headers().get("referrer-policy").unwrap(),
            "same-origin"
        );
    }
}
