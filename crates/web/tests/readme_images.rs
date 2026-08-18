//! The image endpoint: what it serves, what it refuses, and what it never
//! fetches (RFC 0007-bis §4.2, §5.1, §7.1).
//!
//! The fetcher is a **fake**, and deliberately so. The real one's first guard is
//! `ssrf::ensure_public_url`, which refuses loopback — and every in-process mock
//! server is on loopback, so a test using the real client would be testing the
//! guard rather than the endpoint. The guard has its own test where it lives
//! (`adapters::registry::readme_image`); this file is about everything the
//! endpoint decides *before* and *after* the fetch.
//!
//! The fake also counts calls, which is what lets the interesting assertions be
//! about requests that were **not** made: the negative cache, the `strip`
//! policy, and the visibility gate.

mod common;
#[allow(unused_imports)]
use common::*;

use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};

use actix_web::test::{call_service, TestRequest};
use async_trait::async_trait;

use batlehub_adapters::cache::InMemoryCacheStore;
use batlehub_adapters::in_memory::InMemoryReadmeRepository;
use batlehub_config::schema::RegistryMode;
use batlehub_core::{
    entities::{PackageReadme, ReadmeFormat, ReadmeSource},
    error::CoreError,
    ports::{CacheStore, ReadmeImageFetcher, ReadmeRepository},
    services::{
        hot_config::{ReadmeConfig, RemoteImagePolicy},
        readme::image::FetchedImage,
        ReadmeService,
    },
};

// ── The fake fetcher ─────────────────────────────────────────────────────────

/// Answers from a script, and records every URL it was asked for.
struct ScriptedFetcher {
    /// `url → (content_type, bytes)`. Anything not listed answers `Ok(None)`,
    /// which is how the real one reports a `404` or a non-image type.
    answers: Vec<(String, &'static str, Vec<u8>)>,
    calls: AtomicUsize,
    seen: Mutex<Vec<String>>,
}

impl ScriptedFetcher {
    fn new(answers: Vec<(String, &'static str, Vec<u8>)>) -> Arc<Self> {
        Arc::new(Self {
            answers,
            calls: AtomicUsize::new(0),
            seen: Mutex::new(Vec::new()),
        })
    }
    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
    fn seen(&self) -> Vec<String> {
        self.seen.lock().unwrap().clone()
    }
}

#[async_trait]
impl ReadmeImageFetcher for ScriptedFetcher {
    async fn fetch(&self, url: &str, _max_bytes: usize) -> Result<Option<FetchedImage>, CoreError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.seen.lock().unwrap().push(url.to_owned());
        Ok(self
            .answers
            .iter()
            .find(|(u, _, _)| u == url)
            .map(|(_, content_type, bytes)| FetchedImage {
                content_type: batlehub_core::services::readme::image::image_content_type(
                    content_type,
                )
                .expect("test answers use allow-listed types"),
                bytes: bytes.clone(),
            }))
    }
}

// ── The app ──────────────────────────────────────────────────────────────────

const REG: &str = "local-npm";
const PKG: &str = "widget";
const VER: &str = "1.0.0";

/// An app holding one README, with the given image policy and fetcher.
async fn app_with(
    readme_markdown: &str,
    policy: RemoteImagePolicy,
    fetcher: Arc<ScriptedFetcher>,
) -> impl actix_web::dev::Service<
    actix_http::Request,
    Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
    Error = actix_web::Error,
> {
    let repo = InMemoryReadmeRepository::new();
    repo.upsert(PackageReadme {
        registry: REG.to_owned(),
        name: PKG.to_owned(),
        version: VER.to_owned(),
        digest: batlehub_core::entities::readme_digest(readme_markdown),
        content: readme_markdown.to_owned(),
        format: ReadmeFormat::Markdown,
        source: ReadmeSource::LocalPublish,
        truncated: false,
        package_level: false,
        extracted_at: chrono::Utc::now(),
    })
    .await
    .expect("seed");

    let cache: Arc<dyn CacheStore> = Arc::new(InMemoryCacheStore::new());
    let readme_svc = Arc::new(
        ReadmeService::new(Arc::clone(&repo) as Arc<dyn ReadmeRepository>)
            .with_cache(cache)
            .with_image_fetcher(fetcher as Arc<dyn ReadmeImageFetcher>),
    );

    let parts = local_registry_app_parts_with_readme(
        REG,
        "npm",
        RegistryMode::Local,
        None,
        Some(readme_svc),
    );
    // The endpoint reads the policy out of `LocalRegistryService`'s hot config,
    // which the shared factory leaves empty — an absent entry means the README
    // block's default, and its default is `strip`.
    {
        let mut hot = parts.local_svc.hot.write().await;
        hot.readme.insert(
            REG.to_owned(),
            ReadmeConfig {
                remote_images: policy,
                registry_type: "npm".to_owned(),
                ..Default::default()
            },
        );
    }
    build_local_registry_app(parts, batlehub_web::CargoIndexMap::default(), None).await
}

fn image_uri(index: usize) -> String {
    format!("/api/v1/explore/packages/{REG}/{PKG}/{VER}/readme-image/{index}")
}

const BADGE: &str =
    "![build](https://img.shields.io/build.svg) and ![logo](https://cdn.example/logo.png)";

const CLEAN_SVG: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg"><text>passing</text></svg>"#;
const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];

fn scripted() -> Arc<ScriptedFetcher> {
    ScriptedFetcher::new(vec![
        (
            "https://img.shields.io/build.svg".to_owned(),
            "image/svg+xml",
            CLEAN_SVG.to_vec(),
        ),
        (
            "https://cdn.example/logo.png".to_owned(),
            "image/png",
            PNG.to_vec(),
        ),
    ])
}

// ── What it serves ───────────────────────────────────────────────────────────

/// The whole contract in one test: index `n` resolves to the `n`th image of the
/// stored README, and the caller never named a URL.
#[actix_web::test]
async fn an_index_resolves_to_that_images_url_and_serves_its_bytes() {
    let fetcher = scripted();
    let app = app_with(BADGE, RemoteImagePolicy::Proxy, Arc::clone(&fetcher)).await;

    let resp = call_service(
        &app,
        TestRequest::get()
            .uri(&image_uri(1))
            .insert_header(("Authorization", bearer(USER_TOKEN)))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.headers().get("content-type").unwrap(),
        "image/png",
        "the type is echoed from the allow-list"
    );
    // Index 1 is the *second* image, and the fetcher was asked for exactly that
    // URL — which appears nowhere in the request the browser made.
    assert_eq!(fetcher.seen(), vec!["https://cdn.example/logo.png"]);
    let body = actix_web::test::read_body(resp).await;
    assert_eq!(&body[..], PNG);
}

/// §7.2's first control, and the one that does not depend on the SVG sanitiser
/// being right. On every type, not only on SVG.
#[actix_web::test]
async fn every_image_carries_the_sandbox_csp_and_is_privately_cacheable() {
    let app = app_with(BADGE, RemoteImagePolicy::Proxy, scripted()).await;

    for index in [0usize, 1] {
        let resp = call_service(
            &app,
            TestRequest::get()
                .uri(&image_uri(index))
                .insert_header(("Authorization", bearer(USER_TOKEN)))
                .to_request(),
        )
        .await;
        assert_eq!(resp.status(), 200, "index {index}");
        let csp = resp
            .headers()
            .get("content-security-policy")
            .expect("CSP present")
            .to_str()
            .unwrap();
        assert!(csp.contains("default-src 'none'"), "{index}: {csp}");
        assert!(csp.contains("sandbox"), "{index}: {csp}");
        assert_eq!(
            resp.headers().get("content-disposition").unwrap(),
            "inline",
            "index {index}"
        );
        // `private`: the response is behind the visibility gate, so a shared
        // cache must not hold an internal package's badge.
        let cache_control = resp
            .headers()
            .get("cache-control")
            .unwrap()
            .to_str()
            .unwrap();
        assert!(
            cache_control.starts_with("private"),
            "{index}: {cache_control}"
        );
    }
}

/// An SVG is served through the allow-list, not verbatim: two-thirds of README
/// images are SVG (§13.2), so refusing them was not an option — sanitising them
/// was (§7.2).
#[actix_web::test]
async fn an_svg_is_served_sanitised() {
    let hostile = br#"<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script><text>passing</text></svg>"#;
    let fetcher = ScriptedFetcher::new(vec![(
        "https://img.shields.io/build.svg".to_owned(),
        "image/svg+xml",
        hostile.to_vec(),
    )]);
    let app = app_with(BADGE, RemoteImagePolicy::Proxy, fetcher).await;

    let resp = call_service(
        &app,
        TestRequest::get()
            .uri(&image_uri(0))
            .insert_header(("Authorization", bearer(USER_TOKEN)))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.headers().get("content-type").unwrap(), "image/svg+xml");
    let body = String::from_utf8(actix_web::test::read_body(resp).await.to_vec()).unwrap();
    assert!(!body.contains("script"), "{body}");
    assert!(!body.contains("alert"), "{body}");
    // And the badge still reads.
    assert!(body.contains("passing"), "{body}");
}

// ── What it refuses ──────────────────────────────────────────────────────────

/// Flipping to `strip` stops the egress immediately, which is what an operator
/// setting it expects. Asserted on the fetcher, not on the status: a `404` that
/// still made the request would have failed the operator while passing the test.
#[actix_web::test]
async fn under_strip_nothing_is_fetched_at_all() {
    let fetcher = scripted();
    let app = app_with(BADGE, RemoteImagePolicy::Strip, Arc::clone(&fetcher)).await;

    let resp = call_service(
        &app,
        TestRequest::get()
            .uri(&image_uri(0))
            .insert_header(("Authorization", bearer(USER_TOKEN)))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 404);
    assert_eq!(fetcher.calls(), 0, "no upstream request under strip");
}

#[actix_web::test]
async fn an_index_past_the_end_is_a_404_and_fetches_nothing() {
    let fetcher = scripted();
    let app = app_with(BADGE, RemoteImagePolicy::Proxy, Arc::clone(&fetcher)).await;

    let resp = call_service(
        &app,
        TestRequest::get()
            .uri(&image_uri(99))
            .insert_header(("Authorization", bearer(USER_TOKEN)))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 404);
    assert_eq!(fetcher.calls(), 0, "there was no URL to fetch");
}

/// A README with no images has no image endpoint, whatever index is asked for.
#[actix_web::test]
async fn a_readme_with_no_images_serves_none() {
    let fetcher = scripted();
    let app = app_with(
        "# Just prose\n\nNo pictures.",
        RemoteImagePolicy::Proxy,
        Arc::clone(&fetcher),
    )
    .await;

    for index in [0usize, 1, 5] {
        let resp = call_service(
            &app,
            TestRequest::get()
                .uri(&image_uri(index))
                .insert_header(("Authorization", bearer(USER_TOKEN)))
                .to_request(),
        )
        .await;
        assert_eq!(resp.status(), 404, "index {index}");
    }
    assert_eq!(fetcher.calls(), 0);
}

/// An SVG the sanitiser will not vouch for is not served: `proxy` is not an
/// undertaking to render whatever a badge host returns.
#[actix_web::test]
async fn an_svg_that_is_not_svg_is_refused_rather_than_served() {
    let fetcher = ScriptedFetcher::new(vec![(
        "https://img.shields.io/build.svg".to_owned(),
        "image/svg+xml",
        b"<html><body>gotcha</body></html>".to_vec(),
    )]);
    let app = app_with(BADGE, RemoteImagePolicy::Proxy, fetcher).await;

    let resp = call_service(
        &app,
        TestRequest::get()
            .uri(&image_uri(0))
            .insert_header(("Authorization", bearer(USER_TOKEN)))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 404);
}

// ── The caching, positive and negative ───────────────────────────────────────

/// A second read of the same image makes no second request. Two packages that
/// share a badge URL — which is most of them — are one entry, because the key is
/// the URL and not the coordinate.
#[actix_web::test]
async fn a_served_image_is_not_fetched_twice() {
    let fetcher = scripted();
    let app = app_with(BADGE, RemoteImagePolicy::Proxy, Arc::clone(&fetcher)).await;

    for _ in 0..3 {
        let resp = call_service(
            &app,
            TestRequest::get()
                .uri(&image_uri(0))
                .insert_header(("Authorization", bearer(USER_TOKEN)))
                .to_request(),
        )
        .await;
        assert_eq!(resp.status(), 200);
    }
    assert_eq!(fetcher.calls(), 1, "cached after the first read");
}

/// 3.3 % of the image URLs in real READMEs are dead (§13.2). Without this, each
/// one is re-fetched on every render-cache miss for as long as the README
/// exists — so the negative cache is a bounded, measured saving rather than a
/// hypothetical one (§11 q15).
#[actix_web::test]
async fn a_failed_image_is_remembered_and_not_re_fetched() {
    // The fetcher knows nothing about this URL, so it answers `Ok(None)` — the
    // shape a `404` takes.
    let fetcher = ScriptedFetcher::new(vec![]);
    let app = app_with(BADGE, RemoteImagePolicy::Proxy, Arc::clone(&fetcher)).await;

    for _ in 0..3 {
        let resp = call_service(
            &app,
            TestRequest::get()
                .uri(&image_uri(0))
                .insert_header(("Authorization", bearer(USER_TOKEN)))
                .to_request(),
        )
        .await;
        assert_eq!(resp.status(), 404);
    }
    assert_eq!(fetcher.calls(), 1, "the miss was remembered");
}

// ── What it inherits ─────────────────────────────────────────────────────────

/// An image is *part of* a README, so it must be reachable exactly when the
/// README is. A caller who cannot see the package gets the same `404` from both,
/// and — the assertion that matters — no request leaves this server on their
/// behalf.
#[actix_web::test]
async fn an_unauthenticated_caller_gets_the_same_answer_from_both_endpoints() {
    let fetcher = scripted();
    let app = app_with(BADGE, RemoteImagePolicy::Proxy, Arc::clone(&fetcher)).await;

    let readme = call_service(
        &app,
        TestRequest::get()
            .uri(&format!(
                "/api/v1/explore/packages/{REG}/does-not-exist/readme"
            ))
            .insert_header(("Authorization", bearer(USER_TOKEN)))
            .to_request(),
    )
    .await;
    let image = call_service(
        &app,
        TestRequest::get()
            .uri(&format!(
                "/api/v1/explore/packages/{REG}/does-not-exist/1.0.0/readme-image/0"
            ))
            .insert_header(("Authorization", bearer(USER_TOKEN)))
            .to_request(),
    )
    .await;
    assert_eq!(readme.status(), image.status());
    assert_eq!(image.status(), 404);
    assert_eq!(fetcher.calls(), 0);
}

/// The `src` the panel receives points at this server and carries an index, and
/// the endpoint that index addresses answers. The two halves are generated by
/// one pass over one document (§5.1), and this is the end-to-end assertion of
/// that.
#[actix_web::test]
async fn the_rendered_src_is_a_url_this_server_answers() {
    let fetcher = scripted();
    let app = app_with(BADGE, RemoteImagePolicy::Proxy, Arc::clone(&fetcher)).await;

    let resp = call_service(
        &app,
        TestRequest::get()
            .uri(&format!(
                "/api/v1/explore/packages/{REG}/{PKG}/readme?version={VER}"
            ))
            .insert_header(("Authorization", bearer(USER_TOKEN)))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = actix_web::test::read_body_json(resp).await;
    let html = body["rendered_html"].as_str().expect("html");

    // No third-party host reaches the reader's browser…
    assert!(!html.contains("shields.io"), "{html}");
    assert!(!html.contains("cdn.example"), "{html}");
    // …and the `src` is a path on this server, which really answers.
    let src = html
        .split("src=\"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .expect("an img src");
    assert!(src.contains("/readme-image/0"), "{src}");
    let path = src.split_once("://").unwrap().1;
    let path = &path[path.find('/').unwrap()..];

    let served = call_service(
        &app,
        TestRequest::get()
            .uri(path)
            .insert_header(("Authorization", bearer(USER_TOKEN)))
            .to_request(),
    )
    .await;
    assert_eq!(
        served.status(),
        200,
        "the rendered src must resolve: {path}"
    );
}
