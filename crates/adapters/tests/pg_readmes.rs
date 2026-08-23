//! Integration tests for `PgReadmeRepository` (RFC 0007 §10).
//!
//! Worth a real database for four things an in-memory double agreeing with
//! itself proves nothing about: the `ON CONFLICT` that makes a re-resolve
//! *replace* a version's README rather than duplicate it, the `= ANY($3)`
//! exclusion the fallback rule depends on to skip blocked versions, that
//! deletion is scoped to what it names — the table has no foreign key, so
//! nothing else will clean up after a `DELETE` that is too wide — and the
//! **full-text search**, whose stemming and ranking exist only in Postgres.
//! The in-memory double does a substring match and deliberately does not
//! imitate them, so a test asserting `retry` finds `retrying` can only run
//! here (RFC 0007-bis §5.2, §13.3).
//!
//!   task test:pg-readmes
//!   DATABASE_URL=postgresql://postgres:pass@localhost/postgres \
//!     cargo test -p batlehub-adapters --test pg_readmes

use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{Duration, Utc};
use sqlx::PgPool;

use batlehub_adapters::db::PgReadmeRepository;
use batlehub_core::{
    entities::{readme_digest, PackageReadme, ReadmeFormat, ReadmeSource},
    ports::ReadmeRepository,
};

fn db_url() -> Option<String> {
    std::env::var("DATABASE_URL").ok()
}

static TEST_ID: AtomicU64 = AtomicU64::new(0);

struct TestRepo {
    repo: PgReadmeRepository,
    registry: String,
}

/// A registry name unique per *run*, not just per test: these tests upsert and
/// delete by `(registry, package_name, …)`, so a second run against the same
/// database would read the previous run's rows. The pid supplies that, as it
/// does in `pg_stats_history.rs`. Pids are recycled, so the fixture also clears
/// anything a long-dead namesake left under its own name. It touches nothing
/// else.
async fn make_repo(url: &str) -> TestRepo {
    let id = TEST_ID.fetch_add(1, Ordering::Relaxed);
    let registry = format!("readme-t{}-{id}", std::process::id());
    let pool = PgPool::connect(url).await.expect("connect to postgres");
    batlehub_adapters::migrations::embedded_migrator()
        .run(&pool)
        .await
        .expect("run migrations");
    sqlx::query("DELETE FROM package_readmes WHERE registry = $1")
        .bind(&registry)
        .execute(&pool)
        .await
        .expect("clear rows from a previous run under this registry name");
    TestRepo {
        repo: PgReadmeRepository::new(pool),
        registry,
    }
}

impl TestRepo {
    fn readme(&self, name: &str, version: &str, body: &str) -> PackageReadme {
        PackageReadme {
            registry: self.registry.clone(),
            name: name.into(),
            version: version.into(),
            content: body.into(),
            format: ReadmeFormat::Markdown,
            source: ReadmeSource::UpstreamMetadata,
            digest: readme_digest(body),
            truncated: false,
            package_level: false,
            extracted_at: Utc::now(),
        }
    }
}

/// A re-resolve that read different text replaces the row rather than adding a
/// second one — the coordinate is the primary key, and the page would otherwise
/// have two answers for one version and no rule for picking.
#[tokio::test]
async fn upsert_replaces_the_row_for_a_coordinate() {
    let Some(url) = db_url() else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let t = make_repo(&url).await;

    t.repo
        .upsert(t.readme("express", "4.18.2", "the first read"))
        .await
        .unwrap();
    let mut second = t.readme("express", "4.18.2", "upstream edited it");
    second.format = ReadmeFormat::Plain;
    second.truncated = true;
    second.package_level = true;
    second.source = ReadmeSource::Archive;
    t.repo.upsert(second).await.unwrap();

    let got = t
        .repo
        .get(&t.registry, "express", "4.18.2")
        .await
        .unwrap()
        .expect("row present");
    assert_eq!(got.content, "upstream edited it");
    // Every field moves, not just the body: format and source describe *this*
    // text, and a stale format would send the new bytes through the wrong
    // renderer.
    assert_eq!(got.format, ReadmeFormat::Plain);
    assert_eq!(got.source, ReadmeSource::Archive);
    assert!(got.truncated);
    // npm's root README, attributed to `dist-tags.latest`: the panel says so
    // rather than presenting a package-level document as this version's.
    assert!(got.package_level);
    assert_eq!(got.digest, readme_digest("upstream edited it"));

    assert_eq!(
        t.repo
            .list_versions_with_readme(&t.registry, "express")
            .await
            .unwrap(),
        ["4.18.2"]
    );
}

/// Each version keeps its own text, which is the whole reason the store is
/// keyed by version: a package-level row would show 2.x's API to a 1.x reader.
#[tokio::test]
async fn every_version_keeps_its_own_readme() {
    let Some(url) = db_url() else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let t = make_repo(&url).await;

    t.repo
        .upsert(t.readme("lib", "1.0.0", "the 1.x API"))
        .await
        .unwrap();
    t.repo
        .upsert(t.readme("lib", "2.0.0", "the 2.x API"))
        .await
        .unwrap();

    assert_eq!(
        t.repo
            .get(&t.registry, "lib", "1.0.0")
            .await
            .unwrap()
            .unwrap()
            .content,
        "the 1.x API"
    );
    assert_eq!(
        t.repo
            .get(&t.registry, "lib", "2.0.0")
            .await
            .unwrap()
            .unwrap()
            .content,
        "the 2.x API"
    );
    assert!(t
        .repo
        .get(&t.registry, "lib", "3.0.0")
        .await
        .unwrap()
        .is_none());

    let mut versions = t
        .repo
        .list_versions_with_readme(&t.registry, "lib")
        .await
        .unwrap();
    versions.sort();
    assert_eq!(versions, ["1.0.0", "2.0.0"]);
}

/// The fallback rule hands its exclusions down because the store knows nothing
/// about firewall state. `NOT (version = ANY($3))` is what makes a blocked or
/// unlisted version ineligible as a fallback source.
#[tokio::test]
async fn the_fallback_query_honours_the_callers_exclusions() {
    let Some(url) = db_url() else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let t = make_repo(&url).await;

    let mut older = t.readme("lib", "1.4.2", "old but readable");
    older.extracted_at = Utc::now() - Duration::hours(2);
    t.repo.upsert(older).await.unwrap();
    t.repo
        .upsert(t.readme("lib", "2.0.0", "newest"))
        .await
        .unwrap();

    assert_eq!(
        t.repo
            .get_latest_with_readme(&t.registry, "lib", &[])
            .await
            .unwrap()
            .unwrap()
            .version,
        "2.0.0"
    );
    assert_eq!(
        t.repo
            .get_latest_with_readme(&t.registry, "lib", &["2.0.0".to_owned()])
            .await
            .unwrap()
            .unwrap()
            .version,
        "1.4.2"
    );
    // Everything excluded is "no fallback", not "the first row anyway".
    assert!(t
        .repo
        .get_latest_with_readme(
            &t.registry,
            "lib",
            &["1.4.2".to_owned(), "2.0.0".to_owned()]
        )
        .await
        .unwrap()
        .is_none());
    // An empty exclusion list must not be read as "exclude everything", which
    // is what a naïve `NOT IN ()` would do.
    assert!(t
        .repo
        .get_latest_with_readme(&t.registry, "lib", &[])
        .await
        .unwrap()
        .is_some());
}

/// The table has no foreign key — a cascade from anything evictable would take
/// the README with the bytes, which §5.4 rules out — so a `DELETE` that is too
/// wide has nothing to catch it.
#[tokio::test]
async fn deletion_is_scoped_to_what_it_names() {
    let Some(url) = db_url() else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let t = make_repo(&url).await;

    t.repo.upsert(t.readme("a", "1.0.0", "a1")).await.unwrap();
    t.repo.upsert(t.readme("a", "2.0.0", "a2")).await.unwrap();
    t.repo.upsert(t.readme("b", "1.0.0", "b1")).await.unwrap();

    t.repo
        .delete_for_version(&t.registry, "a", "1.0.0")
        .await
        .unwrap();
    assert!(t
        .repo
        .get(&t.registry, "a", "1.0.0")
        .await
        .unwrap()
        .is_none());
    assert!(t
        .repo
        .get(&t.registry, "a", "2.0.0")
        .await
        .unwrap()
        .is_some());

    t.repo.delete_for_package(&t.registry, "a").await.unwrap();
    assert!(t
        .repo
        .list_versions_with_readme(&t.registry, "a")
        .await
        .unwrap()
        .is_empty());
    assert!(t
        .repo
        .get(&t.registry, "b", "1.0.0")
        .await
        .unwrap()
        .is_some());

    // Deleting a coordinate that was never stored is not an error: the local
    // delete path calls this unconditionally.
    t.repo
        .delete_for_version(&t.registry, "never", "0.0.0")
        .await
        .unwrap();
    t.repo
        .delete_for_package(&t.registry, "never")
        .await
        .unwrap();
}

// ── Full-text search (RFC 0007-bis §5.2) ─────────────────────────────────────

/// The measurement that reversed this RFC's own recommendation, as a test.
///
/// It was drafted specifying `simple`, on the argument that stemming mangles
/// identifiers. It does — the stored vector holds `axio`, not `axios` — and it
/// does so **symmetrically**, because the query is stemmed by the same
/// configuration. What `simple` cannot do is find a README that says `retrying`
/// when a reader types `retry`, which is the exact shape of question §2.2 says
/// this feature exists to answer (§13.3).
#[tokio::test]
async fn english_stemming_finds_the_word_a_reader_would_type() {
    let Some(url) = db_url() else {
        eprintln!("DATABASE_URL not set; skipping");
        return;
    };
    let t = make_repo(&url).await;
    t.repo
        .upsert(t.readme(
            "backoff",
            "1.0.0",
            "A tiny helper for retrying failed requests with exponential backoff. \
             It handles caching of the last successful response, and serialisation \
             of the retry state. Built on axios.",
        ))
        .await
        .unwrap();

    let regs = vec![t.registry.clone()];
    for query in [
        "retry",
        "retrying",
        "cache",
        "caching",
        "exponential backoff",
    ] {
        let hits = t.repo.search(&regs, query, 10).await.unwrap();
        assert_eq!(hits.len(), 1, "'{query}' should match");
        assert_eq!(hits[0].name, "backoff", "'{query}'");
    }

    // An identifier is stemmed too, and still matches — which is the half of the
    // draft's reasoning that was right about the mechanism and wrong about the
    // consequence.
    let hits = t.repo.search(&regs, "axios", 10).await.unwrap();
    assert_eq!(hits.len(), 1, "an identifier still matches after stemming");

    // A word that is not there is not found. Without this the test above would
    // pass against a `search` that ignored its query.
    assert!(t
        .repo
        .search(&regs, "kubernetes", 10)
        .await
        .unwrap()
        .is_empty());
}

/// The snippet is **plain text**, because it is a second surface for
/// package-authored content and it is not going to be a second place where
/// markup is interpreted (§7.4). `ts_headline` is asked for empty delimiters, so
/// nothing downstream has to strip anything.
#[tokio::test]
async fn the_snippet_comes_back_as_text_and_never_as_markup() {
    let Some(url) = db_url() else {
        eprintln!("DATABASE_URL not set; skipping");
        return;
    };
    let t = make_repo(&url).await;
    t.repo
        .upsert(t.readme(
            "markup",
            "1.0.0",
            "# Heading\n\n<script>alert(1)</script>\n\nThis library performs \
             deduplication of concurrent requests, which is the interesting part.",
        ))
        .await
        .unwrap();

    let hits = t
        .repo
        .search(std::slice::from_ref(&t.registry), "deduplication", 10)
        .await
        .unwrap();
    assert_eq!(hits.len(), 1);
    let snippet = &hits[0].snippet;
    assert!(snippet.contains("deduplication"), "{snippet}");
    // Whatever `ts_headline` returns, it adds no markup of its own — the
    // highlight delimiters are empty. This is the assertion that caught them
    // being written bare (`StartSel=`), which made Postgres read the next
    // option's name as the value and leave `StopSel` at its default.
    assert!(!snippet.contains("<b>"), "{snippet}");
    assert!(!snippet.contains("</b>"), "{snippet}");
    assert!(!snippet.contains("StopSel"), "{snippet}");
    assert!(!snippet.contains("StartSel"), "{snippet}");
    // And nothing the package wrote survives as markup either.
    assert!(!snippet.contains("<script"), "{snippet}");
}

/// One row per **package**, not per version: a README repeated across forty
/// patch releases would otherwise fill the page by itself.
#[tokio::test]
async fn a_package_appears_once_however_many_versions_share_its_readme() {
    let Some(url) = db_url() else {
        eprintln!("DATABASE_URL not set; skipping");
        return;
    };
    let t = make_repo(&url).await;
    for version in ["1.0.0", "1.0.1", "1.0.2", "1.1.0"] {
        t.repo
            .upsert(t.readme("repeated", version, "Handles websocket reconnection."))
            .await
            .unwrap();
    }

    let hits = t
        .repo
        .search(std::slice::from_ref(&t.registry), "reconnection", 10)
        .await
        .unwrap();
    assert_eq!(hits.len(), 1, "one row for the package, not four");
    assert_eq!(hits[0].name, "repeated");
}

/// The accessible set is applied **in the query**, not after it. A search that
/// read rows it then discarded would make `limit` mean something different for
/// different callers — and would be reading a package an `internal` visibility
/// gate exists to hide (§7.3).
#[tokio::test]
async fn a_registry_outside_the_accessible_set_is_not_searched() {
    let Some(url) = db_url() else {
        eprintln!("DATABASE_URL not set; skipping");
        return;
    };
    let t = make_repo(&url).await;
    t.repo
        .upsert(t.readme("secret", "1.0.0", "Internal telemetry ingestion pipeline."))
        .await
        .unwrap();

    assert_eq!(
        t.repo
            .search(std::slice::from_ref(&t.registry), "telemetry", 10)
            .await
            .unwrap()
            .len(),
        1
    );
    // Not in the caller's set: no row, whatever it says.
    assert!(t
        .repo
        .search(&["some-other-registry".to_owned()], "telemetry", 10)
        .await
        .unwrap()
        .is_empty());
    // An empty set is not "everything".
    assert!(t
        .repo
        .search(&[], "telemetry", 10)
        .await
        .unwrap()
        .is_empty());
}

/// `websearch_to_tsquery` accepts what a person types. A search box that 500s on
/// an apostrophe is not a search box (§5.2).
#[tokio::test]
async fn a_query_a_person_would_type_does_not_error() {
    let Some(url) = db_url() else {
        eprintln!("DATABASE_URL not set; skipping");
        return;
    };
    let t = make_repo(&url).await;
    t.repo
        .upsert(t.readme(
            "parser",
            "1.0.0",
            "A fast JSON parser that doesn't allocate.",
        ))
        .await
        .unwrap();
    let regs = vec![t.registry.clone()];

    for query in [
        "doesn't",
        "\"json parser\"",
        "json or yaml",
        "parser -yaml",
        "&&&",
        "((((",
        "   ",
        "",
    ] {
        t.repo
            .search(&regs, query, 10)
            .await
            .unwrap_or_else(|e| panic!("{query:?} should not error: {e}"));
    }

    // The phrase query is not merely non-erroring — it matches.
    assert_eq!(
        t.repo
            .search(&regs, "\"json parser\"", 10)
            .await
            .unwrap()
            .len(),
        1
    );
}

/// Changing `[search] text_config` rebuilds the generated column, and the
/// rebuild is idempotent: running it again with the same value is two catalogue
/// queries and no DDL.
#[tokio::test]
async fn the_text_configuration_can_be_changed_and_settling_on_one_is_idempotent() {
    let Some(url) = db_url() else {
        eprintln!("DATABASE_URL not set; skipping");
        return;
    };
    let t = make_repo(&url).await;
    let pool = PgPool::connect(&url).await.unwrap();

    // An unknown configuration is refused rather than interpolated.
    let err = batlehub_adapters::db::ensure_readme_text_config(
        &pool,
        "not_a_configuration; DROP TABLE package_readmes",
    )
    .await
    .expect_err("an unknown configuration must be refused");
    assert!(err.to_string().contains("text_config"), "{err}");
    // And the table is still there.
    t.repo
        .upsert(t.readme("still-here", "1.0.0", "Intact."))
        .await
        .unwrap();

    // Settling on the default is a no-op.
    for _ in 0..2 {
        let chosen = batlehub_adapters::db::ensure_readme_text_config(&pool, "english")
            .await
            .expect("english exists on every Postgres");
        assert_eq!(chosen, "english");
    }

    // What the column reports about itself is what was settled — the property
    // the startup path relies on to answer "which configuration do queries use"
    // when prose search is off and nothing was rebuilt. It is also what makes
    // the no-op above a no-op: read it wrong and every startup drops and rebuilds
    // the generated column.
    assert_eq!(
        batlehub_adapters::db::column_text_config(&pool)
            .await
            .unwrap()
            .as_deref(),
        Some("english")
    );

    // The list a reload validates a candidate `[search] text_config` against.
    let names = batlehub_adapters::db::text_config_names(&pool)
        .await
        .unwrap();
    assert!(names.iter().any(|n| n == "english"), "{names:?}");
    assert!(
        !names.iter().any(|n| n == "not_a_configuration"),
        "{names:?}"
    );
    // And the search still works afterwards.
    assert_eq!(
        t.repo
            .search(std::slice::from_ref(&t.registry), "intact", 10)
            .await
            .unwrap()
            .len(),
        1
    );
}
