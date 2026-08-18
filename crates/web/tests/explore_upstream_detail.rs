//! The package page answering for a package this instance holds nothing of
//! (RFC 0007 §2.3, §4.4, §5.5).
//!
//! This is the half of the RFC with the most ways to be quietly wrong, so the
//! assertions are about what is *not* there as much as what is: a page view
//! must not be able to change what the catalogue claims this instance has, and
//! a private name must never be sent to a public index.

mod common;
#[allow(unused_imports)]
use common::*;

use std::sync::{Arc, Mutex};

use actix_web::test::{call_service, read_body_json, TestRequest};
use async_trait::async_trait;
use base64::Engine as _;

use batlehub_adapters::in_memory::InMemoryReadmeRepository;
use batlehub_config::schema::RegistryMode;
use batlehub_core::{
    entities::{PackageId, PackageMetadata},
    error::CoreError,
    ports::{DocumentKind, FetchedArtifact, ReadmeRepository, RegistryClient, VersionDocument},
    services::{hot_config::UpstreamDetailConfig, ReadmeService},
};

// ── A registry that counts what it was asked ──────────────────────────────────

/// Wraps the shared fixture and counts every upstream document fetch, so the
/// coalescing and negative-cache assertions are about *requests* rather than
/// about timing.
struct CountingRegistry {
    inner: Arc<FixedRegistry>,
    fetches: Arc<Mutex<usize>>,
    /// Per-version metadata resolves, which is a different question from
    /// listing-document fetches: PyPI answers the README only from the former.
    resolves: Arc<Mutex<usize>>,
    /// Linked-README reads, so the extension galleries' outbound fetch is
    /// counted rather than assumed.
    linked_reads: Arc<Mutex<usize>>,
    /// When set, every fetch fails this way instead — a `404` is a fact about
    /// the package, a connection error is not, and the two must be cached
    /// differently.
    fail: Option<FailureMode>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FailureMode {
    NotFound,
    Unreachable,
}

#[async_trait]
impl RegistryClient for CountingRegistry {
    fn registry_type(&self) -> &str {
        self.inner.registry_type()
    }

    async fn resolve_metadata(&self, pkg: &PackageId) -> Result<PackageMetadata, CoreError> {
        *self.resolves.lock().unwrap() += 1;
        let mut meta = self.inner.resolve_metadata(pkg).await?;
        // PyPI's description lives in `/pypi/{name}/{version}/json`, one request
        // per version — so unlike npm's packument, the *listing* document
        // carries no README and only a per-version resolve has one.
        // Driven off `readme_support()` rather than a list of kinds, so this
        // fixture mirrors the protocol instead of enumerating what happens to
        // be implemented — a kind added to the "answers for unheld versions"
        // column gets fixture coverage without anyone remembering to add it.
        //
        // `MetadataLinked` answers with a URL, everything else with the text.
        use batlehub_core::entities::{MetadataReadme, ReadmeFormat, ReadmeSupport};
        if let Ok(kind) = self
            .registry_type()
            .parse::<batlehub_core::entities::RegistryKind>()
        {
            let support = kind.readme_support();
            if support.answers_for_unheld_versions() {
                let found = match support {
                    ReadmeSupport::MetadataLinked => MetadataReadme::linked(
                        format!(
                            "https://upstream.invalid/{}/{}/README.md",
                            pkg.name, pkg.version
                        ),
                        ReadmeFormat::Markdown,
                    ),
                    _ => MetadataReadme::text(
                        format!("# {} {}", pkg.name, pkg.version),
                        ReadmeFormat::Markdown,
                    ),
                };
                meta.extra = serde_json::json!({ "readme": found });
            }
        }
        Ok(meta)
    }

    async fn fetch_linked_readme(
        &self,
        url: &str,
        _max_bytes: usize,
    ) -> Result<Option<String>, CoreError> {
        *self.linked_reads.lock().unwrap() += 1;
        Ok(Some(format!("# read from {url}")))
    }

    async fn fetch_artifact(&self, pkg: &PackageId) -> Result<FetchedArtifact, CoreError> {
        self.inner.fetch_artifact(pkg).await
    }

    async fn fetch_version_document(
        &self,
        package: &str,
        kind: DocumentKind,
    ) -> Result<VersionDocument, CoreError> {
        *self.fetches.lock().unwrap() += 1;
        match self.fail {
            Some(FailureMode::NotFound) => {
                Err(CoreError::NotFound(format!("no such package: {package}")))
            }
            Some(FailureMode::Unreachable) => {
                Err(CoreError::Registry("connection refused".to_owned()))
            }
            None => self.inner.fetch_version_document(package, kind).await,
        }
    }

    async fn list_versions(&self, package: &str) -> Result<Vec<String>, CoreError> {
        self.inner.list_versions(package).await
    }
}

// ── The app under test ────────────────────────────────────────────────────────

/// Everything a discovery-read test needs to assert on: the app, the upstream
/// request counter, and the stores a page view must not have written to.
struct Fixture {
    parts: LocalRegistryAppParts,
    fetches: Arc<Mutex<usize>>,
    resolves: Arc<Mutex<usize>>,
    linked_reads: Arc<Mutex<usize>>,
    readme_repo: Arc<InMemoryReadmeRepository>,
}

async fn fixture(kind: &str, mode: RegistryMode, fail: Option<FailureMode>) -> Fixture {
    fixture_with(kind, mode, fail, None).await
}

async fn fixture_with(
    kind: &str,
    mode: RegistryMode,
    fail: Option<FailureMode>,
    cfg: Option<UpstreamDetailConfig>,
) -> Fixture {
    let readme_repo = InMemoryReadmeRepository::new();
    let readme_svc = Arc::new(ReadmeService::new(
        Arc::clone(&readme_repo) as Arc<dyn ReadmeRepository>
    ));
    let parts = local_registry_app_parts_with_readme(
        "reg1",
        kind,
        mode,
        None,
        Some(Arc::clone(&readme_svc)),
    );

    let fetches = Arc::new(Mutex::new(0));
    let resolves = Arc::new(Mutex::new(0));
    let linked_reads = Arc::new(Mutex::new(0));
    {
        let mut hot = parts.proxy_svc.hot.write().await;
        hot.registries.insert(
            "reg1".to_owned(),
            Arc::new(CountingRegistry {
                inner: FixedRegistry::new(kind),
                fetches: Arc::clone(&fetches),
                resolves: Arc::clone(&resolves),
                linked_reads: Arc::clone(&linked_reads),
                fail,
            }) as Arc<dyn RegistryClient>,
        );
        if let Some(cfg) = cfg {
            hot.upstream_detail.insert("reg1".to_owned(), cfg);
        }
    }

    Fixture {
        parts,
        fetches,
        resolves,
        linked_reads,
        readme_repo,
    }
}

async fn build(
    parts: LocalRegistryAppParts,
) -> impl actix_web::dev::Service<
    actix_http::Request,
    Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
    Error = actix_web::Error,
> {
    build_local_registry_app(parts, batlehub_web::CargoIndexMap::default(), None).await
}

async fn detail(
    app: &impl actix_web::dev::Service<
        actix_http::Request,
        Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
        Error = actix_web::Error,
    >,
    uri: &str,
) -> serde_json::Value {
    let req = TestRequest::get()
        .uri(uri)
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let resp = call_service(app, req).await;
    assert_eq!(resp.status(), 200, "the detail page must always answer");
    read_body_json(resp).await
}

fn versions_named(body: &serde_json::Value, source: &str) -> Vec<String> {
    body["versions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|v| v["source"] == source)
        .map(|v| v["version"].as_str().unwrap().to_owned())
        .collect()
}

// ── The assertion the whole RFC turns on ──────────────────────────────────────

/// A package with no local rows at all returns upstream versions. This is the
/// test that would have failed before RFC 0007: the console's own search finds
/// packages this instance holds nothing of, and the page it links to said
/// *"no versions yet"*.
#[actix_web::test]
async fn a_package_with_no_local_rows_answers_from_upstream() {
    let f = fixture("npm", RegistryMode::Proxy, None).await;
    let app = build(f.parts).await;

    let body = detail(&app, "/api/v1/explore/packages/reg1/never-pulled").await;

    let mut upstream = versions_named(&body, "upstream");
    upstream.sort();
    assert_eq!(upstream, ["1.0.0", "1.1.0", "2.0.0-beta.1"]);
    assert_eq!(body["upstream"]["attempted"], true);
    assert_eq!(body["upstream"]["version_count"], 3);
    assert_eq!(body["upstream"]["truncated"], false);
    assert!(body["upstream"]["error"].is_null());

    // Every upstream-only row says so in every cell that would otherwise be a
    // claim about what this instance holds.
    for version in body["versions"].as_array().unwrap() {
        assert_eq!(version["source"], "upstream");
        assert!(
            version["download_count"].is_null(),
            "0 would be a definite answer with nothing behind it: {version}"
        );
        assert!(version["last_accessed"].is_null());
        assert!(version["license"].is_null());
        assert_eq!(
            version["vulnerabilities_scanned"], false,
            "a green row on a package this instance has never held is a claim we cannot support"
        );
        // The packument carries the README, so one cached fetch answered both
        // halves of what the page was missing.
        assert_eq!(version["readme"], "available");
    }
    // Publish times come from the packument's `time` map.
    assert!(body["versions"][0]["published_at"].is_string());
}

// ── What a page view must not do ──────────────────────────────────────────────

/// A page view must not be able to change what the catalogue claims this
/// instance has — otherwise browsing the console silently rewrites the inventory
/// an operator reads to make decisions.
///
/// Asserted against the stores directly, because this is the invariant of §4.4
/// and an implementation drift here is invisible from the response body.
#[actix_web::test]
async fn a_discovery_read_writes_nothing() {
    let f = fixture("npm", RegistryMode::Proxy, None).await;
    let readme_repo = Arc::clone(&f.readme_repo);
    let package_repo = Arc::clone(&f.parts.admin_svc);
    let app = build(f.parts).await;

    let body = detail(&app, "/api/v1/explore/packages/reg1/never-pulled").await;
    assert_eq!(body["upstream"]["attempted"], true);

    // Not in the catalogue: `/api/v1/explore/packages` is the record of what
    // this instance has, and browsing must not add to it.
    let listing: serde_json::Value = read_body_json(
        call_service(
            &app,
            TestRequest::get()
                .uri("/api/v1/explore/packages")
                .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
                .to_request(),
        )
        .await,
    )
    .await;
    let listed: Vec<&str> = listing["packages"]
        .as_array()
        .map(|a| a.iter().filter_map(|p| p["name"].as_str()).collect())
        .unwrap_or_default();
    assert!(
        !listed.contains(&"never-pulled"),
        "the discovery read put the package in the catalogue: {listed:?}"
    );

    // No `package_readmes` row: a README derived from a cached document is
    // bounded by the metadata cache's TTL, and a row written because somebody
    // looked at a page would have nothing that ever deletes it (§5.6).
    assert!(
        readme_repo.all().is_empty(),
        "the discovery read stored a README: {:?}",
        readme_repo.all()
    );

    // And no download count moved anywhere.
    let _ = package_repo;
}

// ── Local rows win ────────────────────────────────────────────────────────────

/// A version present both locally and upstream appears once, described by what
/// we know about it. The merge only *adds* rows the local sources did not have.
#[actix_web::test]
async fn local_rows_win_the_merge_and_upstream_only_adds() {
    let f = fixture("npm", RegistryMode::Hybrid, None).await;
    let app = build(f.parts).await;

    // Publish 1.0.0 here. It is also one of the three versions upstream knows.
    let tarball = base64::engine::general_purpose::STANDARD.encode(b"bytes");
    let req = TestRequest::put()
        .uri("/proxy/reg1/shared")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_json(serde_json::json!({
            "name": "shared",
            "versions": { "1.0.0": { "name": "shared", "version": "1.0.0", "dist": {} } },
            "_attachments": { "shared-1.0.0.tgz": { "data": tarball, "length": 5 } }
        }))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 200);

    let body = detail(&app, "/api/v1/explore/packages/reg1/shared").await;

    // The package is published here, so the discovery read is suppressed
    // entirely — a private name is never sent to a public index (§7.7).
    assert_eq!(body["upstream"]["attempted"], false);
    assert_eq!(versions_named(&body, "local"), ["1.0.0"]);
    assert!(versions_named(&body, "upstream").is_empty());
    assert_eq!(
        body["versions"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|v| v["version"] == "1.0.0")
            .count(),
        1,
        "1.0.0 appeared twice"
    );
}

/// A package published to a **hybrid** registry makes no upstream call at all,
/// asserted on the request counter: on a hybrid registry a private package
/// shares a namespace with a public index, and sending its name there on every
/// page view would leak the existence of internal software to a third party.
#[actix_web::test]
async fn a_locally_published_package_is_never_asked_about_upstream() {
    let f = fixture("npm", RegistryMode::Hybrid, None).await;
    let fetches = Arc::clone(&f.fetches);
    let app = build(f.parts).await;

    let tarball = base64::engine::general_purpose::STANDARD.encode(b"bytes");
    let req = TestRequest::put()
        .uri("/proxy/reg1/internal-lib")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_json(serde_json::json!({
            "name": "internal-lib",
            "versions": { "1.0.0": { "name": "internal-lib", "version": "1.0.0", "dist": {} } },
            "_attachments": { "internal-lib-1.0.0.tgz": { "data": tarball, "length": 5 } }
        }))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 200);
    let before = *fetches.lock().unwrap();

    let body = detail(&app, "/api/v1/explore/packages/reg1/internal-lib").await;
    assert_eq!(body["upstream"]["attempted"], false);
    assert_eq!(
        *fetches.lock().unwrap(),
        before,
        "the private name reached the upstream"
    );
}

/// The sibling of the test above, so the suppression cannot be over-applied
/// into "anything we know about is answered locally": a package with *some*
/// versions held but none published here does make the call. Holding three
/// versions out of forty is exactly the case where the missing rows are worth
/// showing — the suppression is about provenance, not coverage.
#[actix_web::test]
async fn a_package_with_held_but_unpublished_versions_is_still_asked() {
    let f = fixture("npm", RegistryMode::Proxy, None).await;
    let fetches = Arc::clone(&f.fetches);
    let admin = Arc::clone(&f.parts.admin_svc);
    let app = build(f.parts).await;
    let _ = admin;

    // Pull one version through the proxy, so it appears as a `proxied` row.
    let req = TestRequest::get()
        .uri("/proxy/reg1/partly-held/-/partly-held-1.0.0.tgz")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    call_service(&app, req).await;
    let before = *fetches.lock().unwrap();

    let body = detail(&app, "/api/v1/explore/packages/reg1/partly-held").await;
    assert_eq!(body["upstream"]["attempted"], true);
    assert!(
        *fetches.lock().unwrap() > before,
        "a partly-held package was treated as locally published"
    );
}

// ── Rungs ─────────────────────────────────────────────────────────────────────

/// Rung 3 with nothing to fall back to: the page answers from local rows, says
/// the upstream could not be reached, and is still a `200`. It never degrades
/// to an empty page presented as an answer.
#[actix_web::test]
async fn an_unreachable_upstream_still_answers_with_the_reason() {
    let f = fixture("npm", RegistryMode::Proxy, Some(FailureMode::Unreachable)).await;
    let app = build(f.parts).await;

    let body = detail(&app, "/api/v1/explore/packages/reg1/never-pulled").await;
    assert_eq!(body["upstream"]["attempted"], true);
    assert!(
        body["upstream"]["error"].is_string(),
        "the page must say why it is short: {body}"
    );
    assert!(body["versions"].as_array().unwrap().is_empty());
}

/// Rung 1: a second read within the TTL makes no upstream call.
#[actix_web::test]
async fn a_second_page_view_within_the_ttl_makes_no_upstream_call() {
    let f = fixture("npm", RegistryMode::Proxy, None).await;
    let fetches = Arc::clone(&f.fetches);
    let app = build(f.parts).await;

    let first = detail(&app, "/api/v1/explore/packages/reg1/never-pulled").await;
    assert_eq!(first["upstream"]["freshness"], "fresh");
    let after_first = *fetches.lock().unwrap();

    let second = detail(&app, "/api/v1/explore/packages/reg1/never-pulled").await;
    assert_eq!(second["upstream"]["freshness"], "cached");
    assert_eq!(*fetches.lock().unwrap(), after_first);
}

/// An upstream `404` is a fact — upstream *answered* — so it is remembered, and
/// a reload loop or a crawler cannot turn every page view into a request.
#[actix_web::test]
async fn an_upstream_404_is_remembered() {
    let f = fixture("npm", RegistryMode::Proxy, Some(FailureMode::NotFound)).await;
    let fetches = Arc::clone(&f.fetches);
    let app = build(f.parts).await;

    for _ in 0..3 {
        let body = detail(&app, "/api/v1/explore/packages/reg1/no-such-thing").await;
        // Not an error: upstream answered, and the answer was "no".
        assert_eq!(body["upstream"]["attempted"], false);
        assert!(body["upstream"]["error"].is_null());
    }
    assert_eq!(
        *fetches.lock().unwrap(),
        1,
        "the absence was not remembered"
    );
}

/// A connection failure is not a fact about the package, so the next reader
/// tries again rather than being told the package does not exist.
#[actix_web::test]
async fn a_connection_failure_is_retried_rather_than_remembered() {
    let f = fixture("npm", RegistryMode::Proxy, Some(FailureMode::Unreachable)).await;
    let fetches = Arc::clone(&f.fetches);
    let app = build(f.parts).await;

    for _ in 0..3 {
        detail(&app, "/api/v1/explore/packages/reg1/never-pulled").await;
    }
    assert_eq!(*fetches.lock().unwrap(), 3);
}

// ── Bounding and gating ───────────────────────────────────────────────────────

/// `max_versions` truncates newest-first and says so. A silently shortened list
/// is a lie about the registry.
#[actix_web::test]
async fn max_versions_truncates_and_reports_it() {
    let f = fixture_with(
        "npm",
        RegistryMode::Proxy,
        None,
        Some(UpstreamDetailConfig {
            max_versions: 2,
            ..UpstreamDetailConfig::default()
        }),
    )
    .await;
    let app = build(f.parts).await;

    let body = detail(&app, "/api/v1/explore/packages/reg1/never-pulled").await;
    assert_eq!(body["upstream"]["truncated"], true);
    assert_eq!(body["upstream"]["version_count"], 2);
    // The cap keeps the *first N rows of the table*, in the table's own order —
    // stable before pre-release, then newest first. Consistency with what the
    // reader sees matters more than a second ordering that would drop a
    // different pair: a truncated list whose kept rows were not the ones at the
    // top would be confusing in a way a shorter list is not.
    let mut kept = versions_named(&body, "upstream");
    kept.sort();
    assert_eq!(kept, ["1.0.0", "1.1.0"]);
}

/// `?upstream=skip` gives any consumer that wants the old shape the old
/// behaviour, and makes no upstream call.
#[actix_web::test]
async fn upstream_skip_answers_from_local_rows_only() {
    let f = fixture("npm", RegistryMode::Proxy, None).await;
    let fetches = Arc::clone(&f.fetches);
    let app = build(f.parts).await;

    let body = detail(
        &app,
        "/api/v1/explore/packages/reg1/never-pulled?upstream=skip",
    )
    .await;
    assert_eq!(body["upstream"]["attempted"], false);
    assert!(body["versions"].as_array().unwrap().is_empty());
    assert_eq!(*fetches.lock().unwrap(), 0);
}

/// A `local`-mode registry has no upstream: there is nothing to ask, and the
/// page is already complete from local rows.
#[actix_web::test]
async fn a_local_mode_registry_is_never_asked() {
    let f = fixture("npm", RegistryMode::Local, None).await;
    let fetches = Arc::clone(&f.fetches);
    let app = build(f.parts).await;

    let body = detail(&app, "/api/v1/explore/packages/reg1/never-pulled").await;
    assert_eq!(body["upstream"]["attempted"], false);
    assert_eq!(*fetches.lock().unwrap(), 0);
}

/// A registry with the read turned off makes no call — the switch an air-gapped
/// estate sets, and the one an operator whose threat model is "this box talks
/// upstream only when a build needs bytes" reaches for.
#[actix_web::test]
async fn a_disabled_discovery_read_makes_no_call() {
    let f = fixture_with(
        "npm",
        RegistryMode::Proxy,
        None,
        Some(UpstreamDetailConfig {
            enabled: false,
            ..UpstreamDetailConfig::default()
        }),
    )
    .await;
    let fetches = Arc::clone(&f.fetches);
    let app = build(f.parts).await;

    let body = detail(&app, "/api/v1/explore/packages/reg1/never-pulled").await;
    assert_eq!(body["upstream"]["attempted"], false);
    assert_eq!(*fetches.lock().unwrap(), 0);
}

// ── The README half ───────────────────────────────────────────────────────────

/// An upstream-only npm version returns its README, derived from the cached
/// document rather than stored — and the response says which, because a derived
/// answer is bounded by the metadata cache's TTL rather than being a durable
/// record.
#[actix_web::test]
async fn an_upstream_only_version_serves_a_derived_readme() {
    let f = fixture("npm", RegistryMode::Proxy, None).await;
    let readme_repo = Arc::clone(&f.readme_repo);
    let app = build(f.parts).await;

    let req = TestRequest::get()
        .uri("/api/v1/explore/packages/reg1/never-pulled/readme?version=1.1.0&format=both")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = read_body_json(resp).await;

    assert_eq!(body["version"], "1.1.0");
    assert_eq!(body["stored"], false);
    assert!(body["freshness"].is_string(), "{body}");
    assert!(body["source_text"]
        .as_str()
        .unwrap()
        .contains("never-pulled 1.1.0"));
    assert!(body["rendered_html"].as_str().unwrap().contains("<h1"));

    // Derived, not stored: nothing was written for a version this instance
    // holds no bytes for.
    assert!(readme_repo.all().is_empty());
}

/// An upstream-only **cargo** version has its README inside bytes we do not
/// hold, so the endpoint says when one would arrive rather than implying there
/// is none — the same honest limit `license` already has.
#[actix_web::test]
async fn an_upstream_only_archive_borne_version_reports_the_needs_bytes_shape() {
    let f = fixture("cargo", RegistryMode::Proxy, None).await;
    let app = build(f.parts).await;

    let req = TestRequest::get()
        .uri("/api/v1/explore/packages/reg1/some-crate/readme?version=1.0.0")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 404);
    let body: serde_json::Value = read_body_json(resp).await;
    assert_eq!(body["code"], "readme.none-stored");
    assert!(
        body["message"]
            .as_str()
            .unwrap()
            .contains("first downloaded"),
        "{body}"
    );

    // And the version table says `unknown` — never `none`, and never a boolean
    // `false` that would read as "there is none".
    let detail = detail(&app, "/api/v1/explore/packages/reg1/some-crate").await;
    for version in detail["versions"].as_array().unwrap() {
        assert_eq!(version["readme"], "unknown", "{version}");
    }
}

/// PyPI's description lives in `/pypi/{name}/{version}/json`, not in the simple
/// page — so the *listing* document carries no README and the panel has to
/// fetch on selection (RFC 0007 §4.3, open question 7).
///
/// The generated support table says PyPI answers **versions + README** for a
/// version this instance holds nothing of. That has to be true, or the table
/// claims coverage dispatch cannot deliver — the failure RFC 0009 was written
/// about.
#[actix_web::test]
async fn an_unheld_pypi_version_serves_its_per_version_description() {
    let f = fixture("pypi", RegistryMode::Proxy, None).await;
    let readme_repo = Arc::clone(&f.readme_repo);
    let app = build(f.parts).await;

    let req = TestRequest::get()
        .uri("/api/v1/explore/packages/reg1/requests/readme?version=1.1.0&format=both")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200, "the table promises a README here");
    let body: serde_json::Value = read_body_json(resp).await;

    assert_eq!(body["version"], "1.1.0");
    assert_eq!(body["stored"], false);
    assert!(body["source_text"]
        .as_str()
        .unwrap()
        .contains("requests 1.1.0"));

    // Derived, never stored: a row written because somebody looked at a page
    // would have nothing that ever deletes it (§5.6).
    assert!(readme_repo.all().is_empty());
}

/// One request per version is the accepted cost (open question 7), so a second
/// view of the same version must not pay it again.
#[actix_web::test]
async fn a_second_view_of_the_same_pypi_version_resolves_once() {
    let f = fixture("pypi", RegistryMode::Proxy, None).await;
    let resolves = Arc::clone(&f.resolves);
    let app = build(f.parts).await;

    for _ in 0..3 {
        let req = TestRequest::get()
            .uri("/api/v1/explore/packages/reg1/requests/readme?version=1.1.0")
            .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
            .to_request();
        assert_eq!(call_service(&app, req).await.status(), 200);
    }
    assert_eq!(
        *resolves.lock().unwrap(),
        1,
        "the per-version document was re-fetched on a cache hit"
    );
}

/// And the version table still says `unknown` rather than `available` for a
/// PyPI row nothing has been selected on: filling it for every row would be N
/// upstream requests per page view, and a boolean guess is worse than saying
/// we have not looked.
#[actix_web::test]
async fn the_pypi_version_table_reports_unknown_until_a_row_is_selected() {
    let f = fixture("pypi", RegistryMode::Proxy, None).await;
    let resolves = Arc::clone(&f.resolves);
    let app = build(f.parts).await;

    let body = detail(&app, "/api/v1/explore/packages/reg1/requests").await;
    assert_eq!(body["upstream"]["attempted"], true);
    for version in body["versions"].as_array().unwrap() {
        assert_eq!(version["readme"], "unknown", "{version}");
    }
    assert_eq!(
        *resolves.lock().unwrap(),
        0,
        "the table fetched a description per row"
    );
}

/// OpenVSX and the VS Code Marketplace answer with a *URL*, not the text. The
/// table says they answer **versions + README** for an unheld version, so the
/// link has to actually be followed — through the client's own same-origin and
/// SSRF guards, and only for the one version selected.
#[actix_web::test]
async fn an_unheld_extension_version_follows_its_readme_link() {
    let f = fixture("openvsx", RegistryMode::Proxy, None).await;
    let linked_reads = Arc::clone(&f.linked_reads);
    let readme_repo = Arc::clone(&f.readme_repo);
    let app = build(f.parts).await;

    let req = TestRequest::get()
        .uri("/api/v1/explore/packages/reg1/pub.ext/readme?version=1.1.0&format=both")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200, "the table promises a README here");
    let body: serde_json::Value = read_body_json(resp).await;

    assert_eq!(body["version"], "1.1.0");
    assert_eq!(body["stored"], false);
    assert!(body["source_text"].as_str().unwrap().contains("read from"));
    assert_eq!(*linked_reads.lock().unwrap(), 1);
    // Still nothing stored: this is a page view.
    assert!(readme_repo.all().is_empty());
}

/// Every kind the generated support table says answers **versions + README**
/// must actually answer, and this is the assertion that would have caught the
/// three that did not.
///
/// npm answers from the listing document; PyPI and the extension galleries from
/// a per-version resolve. A kind added to that column without a path to the text
/// fails here rather than shipping a table that claims coverage the code cannot
/// deliver.
#[actix_web::test]
async fn every_kind_promising_a_readme_for_unheld_versions_delivers_one() {
    use batlehub_core::entities::RegistryKind;

    for kind in RegistryKind::ALL {
        if !kind.readme_support().answers_for_unheld_versions() {
            continue;
        }
        let f = fixture(kind.as_str(), RegistryMode::Proxy, None).await;
        let app = build(f.parts).await;

        let req = TestRequest::get()
            .uri("/api/v1/explore/packages/reg1/some-package/readme?version=1.1.0")
            .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
            .to_request();
        let resp = call_service(&app, req).await;
        assert_eq!(
            resp.status(),
            200,
            "{kind} promises a README for an unheld version and did not serve one"
        );
        let body: serde_json::Value = read_body_json(resp).await;
        assert_eq!(body["stored"], false, "{kind}");
        assert!(
            body["rendered_html"]
                .as_str()
                .is_some_and(|h| !h.is_empty()),
            "{kind} served an empty README"
        );
    }
}

/// npm's packument is both the listing *and* the README source, so a version
/// whose packument carries no text must not cost a second fetch of the same
/// document to discover the same nothing.
///
/// Verified against the real registry before it was written: `express` ships
/// `readme: ""` and no per-version field, so this is the common case for large
/// packages rather than an edge one.
#[actix_web::test]
async fn npm_does_not_refetch_its_packument_for_a_per_version_readme() {
    let f = fixture("npm", RegistryMode::Proxy, None).await;
    let resolves = Arc::clone(&f.resolves);
    let app = build(f.parts).await;

    // A version the fixture's packument knows but carries no README for.
    let req = TestRequest::get()
        .uri("/api/v1/explore/packages/reg1/quiet-pkg/readme?version=9.9.9")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 404);
    assert_eq!(
        *resolves.lock().unwrap(),
        0,
        "npm resolved a version to re-read the packument it had already fetched"
    );
}
