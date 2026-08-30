//! RFC 0015 §4.1 — the team-namespace separator, asserted across **all three**
//! matchers.
//!
//! §6.3 requires `LOCAL_VISIBILITY_PREDICATE` to agree with `check_visibility`
//! *character for character*, and its own doc comment says why: **a listing more
//! permissive than the check discloses the names of packages the download path
//! would refuse.** Until migration 045 there were three implementations of one
//! rule — `find_namespace`'s SQL, the in-memory store, and the explore predicate
//! — and all three hardcoded `/`, so they agreed by coincidence rather than by
//! construction.
//!
//! They now read one column. This is the test that says they still agree, over a
//! table of cases that includes the ones a dotted ecosystem produces.
//!
//! Requires a running PostgreSQL instance; set `DATABASE_URL` to opt in. The
//! `covers` arm runs regardless, so the file is never silently green.

use std::sync::atomic::{AtomicU64, Ordering};

use batlehub_adapters::db::packages::PoolOptions;
use batlehub_adapters::db::{PgPackageRepository, PgTeamNamespaceStore};
use batlehub_adapters::in_memory::InMemoryTeamNamespaceStore;
use batlehub_core::{entities::TeamNamespace, ports::TeamNamespacePort};

static TEST_ID: AtomicU64 = AtomicU64::new(0);

fn registry() -> String {
    format!(
        "sep-{}-{}",
        std::process::id(),
        TEST_ID.fetch_add(1, Ordering::Relaxed)
    )
}

/// `(separator, prefix, package, covered)` — the cases that distinguish a
/// per-ecosystem separator from npm's.
///
/// The `.` rows are the ones that were wrong: on an OpenVSX registry a claim on
/// `digital` covered `digital` and nothing else, so `digital.pipeline-tools` was
/// outside every claim and its team visibility was unenforceable. The
/// `digitalpipeline` row is RFC 0011-bis §4.2's original bug in the other
/// direction, and has to keep failing.
const CASES: &[(char, &str, &str, bool)] = &[
    // npm and Go — unchanged, because `/` is what every row defaulted to.
    ('/', "@acme/billing", "@acme/billing", true),
    ('/', "@acme/billing", "@acme/billing/cards", true),
    ('/', "@acme/billing", "@acme/billing-internal", false),
    // OpenVSX publishers and NuGet ids.
    ('.', "digital", "digital", true),
    ('.', "digital", "digital.pipeline-tools", true),
    ('.', "digital", "digitalpipeline", false),
    ('.', "digital", "digital-tools", false),
    // Maven groupIds separate the group from the artifact with `:`.
    (':', "com.acme", "com.acme:widget", true),
    (':', "com.acme", "com.acme.internal:widget", false),
    (':', "com.acme", "com.acme", true),
    // A separator that appears *inside* the package name but not at the boundary.
    ('.', "a.b", "a.b.c", true),
    ('.', "a.b", "a.bc", false),
];

fn claim(registry: &str, prefix: &str, separator: char) -> TeamNamespace {
    TeamNamespace {
        registry: registry.to_owned(),
        prefix: prefix.to_owned(),
        group_id: "team".to_owned(),
        claimed_by: None,
        separator,
    }
}

/// The Rust matcher, which the other two are checked against.
#[test]
fn covers_matches_on_the_claims_own_separator() {
    for (sep, prefix, package, expected) in CASES {
        let ns = claim("reg", prefix, *sep);
        assert_eq!(
            ns.covers(package),
            *expected,
            "TeamNamespace::covers({package:?}) under prefix {prefix:?} separator {sep:?}"
        );
    }
}

/// The in-memory store's `find_namespace` agrees with it.
#[tokio::test]
async fn the_in_memory_store_agrees_with_covers() {
    for (sep, prefix, package, expected) in CASES {
        let store = InMemoryTeamNamespaceStore::new();
        let reg = registry();
        store
            .claim_namespace(claim(&reg, prefix, *sep))
            .await
            .expect("claim");

        let found = store.find_namespace(&reg, package).await.expect("find");
        assert_eq!(
            found.is_some(),
            *expected,
            "in-memory find_namespace({package:?}) under {prefix:?} separator {sep:?}"
        );
    }
}

/// …and so does the Postgres one, which is the arm that matters: its SQL is a
/// second implementation of `covers` written in a different language.
#[tokio::test]
async fn the_postgres_store_agrees_with_covers() {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let repo = PgPackageRepository::new(
        &url,
        PoolOptions {
            max_connections: 4,
            min_connections: 1,
            acquire_timeout_secs: 10,
        },
    )
    .await
    .expect("connect to postgres");
    repo.run_migrations().await.expect("run migrations");
    let store = PgTeamNamespaceStore::new(repo.pool());

    for (sep, prefix, package, expected) in CASES {
        let reg = registry();
        store
            .claim_namespace(claim(&reg, prefix, *sep))
            .await
            .expect("claim");

        let found = store.find_namespace(&reg, package).await.expect("find");
        assert_eq!(
            found.is_some(),
            *expected,
            "postgres find_namespace({package:?}) under {prefix:?} separator {sep:?}"
        );
        if let Some(ns) = found {
            assert_eq!(ns.separator, *sep, "the separator round-trips");
        }
    }
}

/// Longest prefix still wins outright, and it wins per separator.
///
/// `LOCAL_VISIBILITY_PREDICATE`'s doc comment is explicit that an `EXISTS` over
/// *all* matching claims would quietly widen it — the most specific claim decides
/// even when a shorter one would have admitted the caller. Adding a separator
/// must not have changed that.
#[tokio::test]
async fn the_longest_prefix_still_wins() {
    let store = InMemoryTeamNamespaceStore::new();
    let reg = registry();
    store.claim_namespace(claim(&reg, "a", '.')).await.unwrap();
    let mut deeper = claim(&reg, "a.b", '.');
    deeper.group_id = "deeper".to_owned();
    store.claim_namespace(deeper).await.unwrap();

    let found = store
        .find_namespace(&reg, "a.b.c")
        .await
        .unwrap()
        .expect("covered by both");
    assert_eq!(found.group_id, "deeper", "the most specific claim decides");
}

/// A row written before migration 045 keeps matching what it matched.
///
/// §10's promise, on the one column whose default decides it: `'/'` is what every
/// claim used before the column existed, so an upgrade changes no claim's
/// meaning. A claim made on a dotted ecosystem beforehand stays narrower until it
/// is re-claimed — the conservative direction, and not a regression, because it
/// is what the row already did.
#[test]
fn the_default_separator_is_what_every_claim_matched_before() {
    let ns = claim("reg", "@acme/billing", '/');
    assert!(ns.covers("@acme/billing/cards"));
    assert!(!ns.covers("@acme/billing-internal"));
}
