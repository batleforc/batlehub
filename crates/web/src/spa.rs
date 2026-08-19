//! Serving the console's own document, and narrowing its policy to the config.
//!
//! # Why the server touches the CSP at all
//!
//! The policy is built by `ui/build/csp.ts` and substituted into a
//! `<meta http-equiv="Content-Security-Policy">` at **build time**, for two
//! reasons that have not changed: it must not apply to `/scalar`, whose bundle
//! comes from a CDN, and `actix_files::Files` is not a `ServiceFactory`, so it
//! can carry no header of its own.
//!
//! What that leaves is a policy frozen at `pnpm build`, describing a
//! configuration nobody has chosen yet. One source in it is config-dependent:
//! `https://badge.socket.dev`, admitted so the socket.dev badge can load. It is
//! a per-registry feature flag, so an instance that turns it off everywhere
//! still shipped a document announcing a third-party origin it will never call
//! — a wider policy than the deployment needs, which is the one direction a CSP
//! must never drift (RFC 0013 §7, §11 O5).
//!
//! # The rule: this can only subtract
//!
//! The built policy is the **maximum**. This module reads the document as built
//! and removes sources the running config makes unnecessary; it has no vocabulary
//! for adding one. That is deliberate and is the whole reason the narrowing lives
//! here rather than in a second policy builder written in Rust:
//!
//!  - there is exactly one definition of what the console may load, and it is the
//!    TypeScript one the browser already gets in dev;
//!  - no config value, however wrong, can open something the build did not allow;
//!  - a deployment whose `index.html` predates this code serves its policy
//!    unchanged rather than failing.
//!
//! [`narrow_csp`] is a pure function over the document text, and its tests assert
//! the subset property directly rather than the resulting string.

use std::path::PathBuf;
use std::sync::Arc;

use actix_web::{get, http::header, web, HttpResponse, Responder};
use batlehub_core::services::LocalRegistryService;

use crate::error::AppError;

/// The directory the built SPA is served from — `[server].static_dir`.
#[derive(Clone)]
pub struct SpaDir(pub PathBuf);

/// The one third-party origin the built policy admits, and the only one this
/// server currently knows how to decide about.
///
/// It is spelled here as well as in `ui/build/csp.ts`, which is duplication of a
/// *constant* rather than of the policy: if the two ever disagree the effect is
/// that this narrowing stops finding anything, which leaves the built policy
/// intact. Failing towards "no change" is the safe direction, and the `BUILT`
/// fixture in this module's tests — copied from a real build — is what says so
/// if the generator changes shape.
const SOCKET_BADGE_ORIGIN: &str = "https://badge.socket.dev";

const META_MARKER: &str = "http-equiv=\"Content-Security-Policy\"";

/// Remove from the document's CSP every source the config says is unused.
///
/// Returns the document unchanged when there is no policy to narrow, when it is
/// not in the shape this understands, or when there is nothing to remove — so
/// applying it twice is applying it once.
pub fn narrow_csp(html: &str, allow_socket_badge: bool) -> String {
    // Nothing to decide: the built policy is already the answer.
    if allow_socket_badge {
        return html.to_owned();
    }

    let Some((start, end)) = content_attribute_span(html) else {
        return html.to_owned();
    };
    let narrowed = drop_source(&html[start..end], SOCKET_BADGE_ORIGIN);
    let mut out = String::with_capacity(html.len());
    out.push_str(&html[..start]);
    out.push_str(&narrowed);
    out.push_str(&html[end..]);
    out
}

/// Byte range of the `content="…"` value on the CSP meta tag.
///
/// Deliberately a search rather than an HTML parse: the input is a document this
/// repository builds from its own template, the tag is written by us, and a
/// parser dependency would be a larger surface than the thing it protects. Every
/// failure to recognise the shape returns `None`, which serves the document as
/// built.
fn content_attribute_span(html: &str) -> Option<(usize, usize)> {
    let marker = html.find(META_MARKER)?;
    // Only look inside this tag: `content=` appears on most of the other meta
    // tags in the document, and matching one of those would rewrite the page
    // description instead of the policy.
    let tag_end = html[marker..].find('>')? + marker;
    let attr = html[marker..tag_end].find("content=\"")? + marker + "content=\"".len();
    let close = html[attr..tag_end].find('"')? + attr;
    Some((attr, close))
}

/// Drop one source expression from every directive of a policy.
///
/// Whitespace-separated tokens, compared whole: a source is removed only when it
/// *is* the token, never when it contains it, so `https://badge.socket.dev.evil`
/// is left alone rather than half-edited. A directive left with a name and no
/// sources is dropped entirely — `img-src` with nothing after it forbids
/// everything, which would be a narrowing this function has no mandate to make
/// on its own.
fn drop_source(policy: &str, source: &str) -> String {
    policy
        .split(';')
        .filter_map(|directive| {
            let mut tokens = directive.split_whitespace();
            let name = tokens.next()?;
            let kept: Vec<&str> = tokens.filter(|t| *t != source).collect();
            if kept.is_empty() {
                // The directive named nothing but the source we removed. Leaving
                // `img-src` empty would block every image including `'self'`;
                // dropping the directive falls back to `default-src`, which the
                // built policy sets to `'self'`. Both are narrower than what was
                // there; this is the one that does not break the page.
                return None;
            }
            Some(format!("{name} {}", kept.join(" ")))
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// Whether any registry would render a socket.dev badge.
///
/// An absent entry means *on* — `FeatureFlags::default()` — which is the same
/// reading `explore/detail.rs` uses when it decides whether to emit the URL. The
/// two must agree: a policy narrower than the page's own behaviour is a broken
/// image, and this is the side that would be wrong.
async fn socket_badge_anywhere(local_svc: &LocalRegistryService) -> bool {
    let hot = local_svc.hot.read().await;
    if hot.feature_flags.is_empty() {
        // No registry configured, so no row can carry a badge. Serving the
        // wider policy here would be admitting an origin for a page that
        // cannot ask for it.
        return false;
    }
    hot.feature_flags.values().any(|f| f.socket_badge)
}

/// The SPA document, with its policy narrowed to what this instance can use.
///
/// Read per request rather than cached: it is one small file, requested once per
/// navigation rather than per asset, and the alternative is a cache that has to
/// be invalidated by config reload — a second piece of state to keep true, for a
/// saving nobody would measure.
async fn serve_index(
    dir: web::Data<SpaDir>,
    local_svc: web::Data<Arc<LocalRegistryService>>,
) -> Result<impl Responder, AppError> {
    let path = dir.0.join("index.html");
    let html = tokio::fs::read_to_string(&path).await.map_err(|e| {
        tracing::error!(path = %path.display(), error = %e, "spa: index.html unreadable");
        AppError::not_found("index.html not found")
    })?;

    let html = narrow_csp(&html, socket_badge_anywhere(&local_svc).await);

    Ok(HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        // The document describes an instance whose config can change under it,
        // so it is the one file here that must not be held by a cache.
        .insert_header((header::CACHE_CONTROL, "no-cache"))
        .body(html))
}

#[get("/")]
async fn spa_root(
    dir: web::Data<SpaDir>,
    local_svc: web::Data<Arc<LocalRegistryService>>,
) -> Result<impl Responder, AppError> {
    serve_index(dir, local_svc).await
}

/// The same document by its own name.
///
/// `actix_files::Files` would otherwise serve `/index.html` straight off disk,
/// with the built policy — the narrowing would then depend on which URL the
/// reader happened to arrive by.
#[get("/index.html")]
async fn spa_index_html(
    dir: web::Data<SpaDir>,
    local_svc: web::Data<Arc<LocalRegistryService>>,
) -> Result<impl Responder, AppError> {
    serve_index(dir, local_svc).await
}

/// Register the document routes. Must be configured **before** the static-file
/// service, which would otherwise answer both of them first.
pub fn configure_spa(cfg: &mut web::ServiceConfig) {
    cfg.service(spa_root).service(spa_index_html);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The policy as `buildCsp()` emits it, copied from a real build. If the
    /// generator changes shape, this is the test that says so rather than a
    /// deployment quietly serving an un-narrowed document.
    const BUILT: &str = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
         img-src 'self' data: https://badge.socket.dev; font-src 'self' data:; \
         connect-src 'self'; object-src 'none'; base-uri 'self'; form-action 'self'";

    fn document(policy: &str) -> String {
        format!(
            "<!doctype html>\n<html>\n<head>\n\
             <meta charset=\"UTF-8\" />\n\
             <meta http-equiv=\"Content-Security-Policy\" content=\"{policy}\" />\n\
             <meta name=\"description\" content=\"Your package hub.\" />\n\
             </head>\n<body></body>\n</html>\n"
        )
    }

    /// Every directive's source list must be a subset of what was built. This is
    /// the property the module exists to guarantee, so it is asserted directly
    /// rather than through an expected string.
    fn sources(policy: &str) -> Vec<(String, Vec<String>)> {
        policy
            .split(';')
            .filter(|d| !d.trim().is_empty())
            .map(|d| {
                let mut t = d.split_whitespace();
                let name = t.next().unwrap().to_owned();
                (name, t.map(str::to_owned).collect())
            })
            .collect()
    }

    fn policy_of(html: &str) -> String {
        let (s, e) = content_attribute_span(html).expect("a policy");
        html[s..e].to_owned()
    }

    #[test]
    fn the_badge_origin_goes_when_no_registry_would_draw_one() {
        let out = narrow_csp(&document(BUILT), false);
        let policy = policy_of(&out);

        assert!(!policy.contains("badge.socket.dev"), "{policy}");
        assert!(policy.contains("img-src 'self' data:"), "{policy}");
    }

    #[test]
    fn the_badge_origin_stays_when_one_registry_would() {
        let out = narrow_csp(&document(BUILT), true);
        assert_eq!(out, document(BUILT));
    }

    /// The invariant: narrowing can only ever remove.
    #[test]
    fn no_directive_gains_a_source() {
        let before = sources(BUILT);
        let after = sources(&policy_of(&narrow_csp(&document(BUILT), false)));

        for (name, srcs) in &after {
            let original = before
                .iter()
                .find(|(n, _)| n == name)
                .unwrap_or_else(|| panic!("narrowing invented the directive {name}"));
            for s in srcs {
                assert!(
                    original.1.contains(s),
                    "narrowing added {s} to {name}: {srcs:?}"
                );
            }
        }
    }

    /// Applying it twice is applying it once — the property that catches a
    /// rewrite which is not actually a filter.
    #[test]
    fn narrowing_is_idempotent() {
        let once = narrow_csp(&document(BUILT), false);
        assert_eq!(narrow_csp(&once, false), once);
    }

    /// A document from a build that predates this, or a custom one: serve it as
    /// it is rather than fail, and never rewrite something else that has a
    /// `content=` attribute.
    #[test]
    fn a_document_with_no_policy_is_untouched() {
        let html = "<!doctype html><html><head>\
                    <meta name=\"description\" content=\"img-src https://badge.socket.dev\" />\
                    </head></html>";
        assert_eq!(narrow_csp(html, false), html);
    }

    #[test]
    fn only_the_policy_tag_is_rewritten() {
        let out = narrow_csp(&document(BUILT), false);
        assert!(
            out.contains("<meta name=\"description\" content=\"Your package hub.\" />"),
            "{out}"
        );
    }

    /// Whole tokens only: a host that merely starts with the origin is a
    /// different host, and half-editing it would produce a policy nobody wrote.
    #[test]
    fn a_lookalike_origin_is_not_touched() {
        let policy = "img-src 'self' https://badge.socket.dev.evil.test";
        assert_eq!(drop_source(policy, SOCKET_BADGE_ORIGIN), policy);
    }

    /// A directive whose only source was the one removed falls back to
    /// `default-src` rather than being left empty, which would block everything
    /// the directive covers.
    #[test]
    fn a_directive_emptied_by_the_removal_is_dropped_not_left_bare() {
        let out = drop_source(
            "default-src 'self'; img-src https://badge.socket.dev",
            SOCKET_BADGE_ORIGIN,
        );
        assert_eq!(out, "default-src 'self'");
    }

    #[test]
    fn a_policy_without_the_origin_survives_verbatim() {
        let policy = "default-src 'self'; img-src 'self' data:";
        assert_eq!(drop_source(policy, SOCKET_BADGE_ORIGIN), policy);
    }
}
