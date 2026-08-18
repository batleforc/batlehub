//! Integration tests for `PgReadmeRepository` (RFC 0007 §10).
//!
//! Worth a real database for three things an in-memory double agreeing with
//! itself proves nothing about: the `ON CONFLICT` that makes a re-resolve
//! *replace* a version's README rather than duplicate it, the `= ANY($3)`
//! exclusion the fallback rule depends on to skip blocked versions, and that
//! deletion is scoped to what it names — the table has no foreign key, so
//! nothing else will clean up after a `DELETE` that is too wide.
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
