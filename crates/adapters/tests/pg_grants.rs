//! `GrantRepository`, against both implementations.
//!
//! RFC 0015 §6.3's `grants` table, and the in-memory store that stands in for it
//! in every other suite. The properties are asserted against **both**, from one
//! body of test code, because agreement between an adapter and its double is not
//! evidence on its own: survey finding 2 shipped precisely because an empty list
//! meant "everything" in four repository implementations "that all agreed with
//! each other". What makes this useful is that the properties are stated once
//! and each store is made to satisfy them separately.
//!
//! The Postgres half is skipped when `DATABASE_URL` is unset, matching the other
//! `pg_*` suites — and it reports the skip rather than passing silently, because
//! a green suite that ran half of itself is the failure mode `task test:pg-*`
//! exists to avoid.

use std::sync::Arc;

use batlehub_adapters::db::PgGrantRepository;
use batlehub_adapters::in_memory::InMemoryGrantRepository;
use batlehub_core::entities::{Action, GroupProvider, SubjectMatcher};
use batlehub_core::ports::{version_node_key, GrantRepository, NodeKind, StoredGrant};

/// Both stores, or just the in-memory one when there is no database.
///
/// `registry` is the caller's own, and it is the *only* thing cleaned. A blanket
/// `DELETE … WHERE registry LIKE 'grants-test-%'` was the first attempt and was
/// wrong in a way worth recording: `cargo test` runs these concurrently, the
/// in-memory store is fresh per test but Postgres is not, and one test's cleanup
/// deleted another's rows mid-run. The Postgres half then disagreed with the
/// in-memory half — which is exactly the signal this file exists to produce, and
/// on the first run it was reporting the harness rather than the adapter.
async fn stores(registry: &str) -> Vec<(&'static str, Arc<dyn GrantRepository>)> {
    let mut out: Vec<(&'static str, Arc<dyn GrantRepository>)> =
        vec![("in-memory", InMemoryGrantRepository::new())];

    match std::env::var("DATABASE_URL") {
        Ok(url) => {
            let pool = sqlx::postgres::PgPoolOptions::new()
                .max_connections(2)
                .connect(&url)
                .await
                .expect("connect to DATABASE_URL");
            batlehub_adapters::migrations::embedded_migrator()
                .run(&pool)
                .await
                .expect("migrations");
            // This test's registry only. These tests assert on the *absence* of
            // rows as much as their presence, so a row left by a previous run
            // would make an absence assertion pass for the wrong reason — but
            // touching a registry this test does not own breaks the tests that
            // run beside it.
            sqlx::query("DELETE FROM grants WHERE registry = $1")
                .bind(registry)
                .execute(&pool)
                .await
                .expect("clean");
            out.push(("postgres", Arc::new(PgGrantRepository::new(pool))));
        }
        Err(_) => {
            eprintln!(
                "note: DATABASE_URL is unset, so only the in-memory store was exercised. \
                 Run `task test:pg-grants` (or set DATABASE_URL) to check the Postgres one."
            );
        }
    }
    out
}

fn grant(
    registry: &str,
    kind: NodeKind,
    key: &str,
    subject: SubjectMatcher,
    actions: Vec<Action>,
) -> StoredGrant {
    StoredGrant {
        registry: registry.to_owned(),
        node_kind: kind,
        node_key: key.to_owned(),
        subject,
        actions,
        granted_by: Some("tester".to_owned()),
    }
}

#[tokio::test]
async fn a_written_grant_reads_back_on_its_own_node() {
    let reg = "grants-test-roundtrip";
    for (name, store) in stores(reg).await {
        store
            .put_grant(grant(
                reg,
                NodeKind::Package,
                "@acme/billing",
                SubjectMatcher::parse("group:oidc1:payments").unwrap(),
                vec![Action::ReleasesPublish, Action::OwnersWrite],
            ))
            .await
            .unwrap_or_else(|e| panic!("{name}: {e}"));

        let rows = store
            .grants_for(reg, "@acme/billing", None)
            .await
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(rows.len(), 1, "{name}");
        assert_eq!(
            rows[0].actions,
            vec![Action::ReleasesPublish, Action::OwnersWrite]
        );
        assert_eq!(
            rows[0].subject,
            SubjectMatcher::Group {
                provider: GroupProvider::Named("oidc1".to_owned()),
                name: "payments".to_owned()
            },
            "{name}: the subject form must survive the round trip, or `explain` \
             reports a different row from the one that matched"
        );
    }
}

/// A neighbouring package does not see it.
///
/// The obvious property, and the one a prefix match would break: `@acme/billing`
/// and `@acme/billing-internal` are different packages, and RFC 0011-bis §4.2
/// records that exact confusion.
#[tokio::test]
async fn a_grant_does_not_leak_to_a_neighbouring_package() {
    let reg = "grants-test-neighbour";
    for (name, store) in stores(reg).await {
        store
            .put_grant(grant(
                reg,
                NodeKind::Package,
                "@acme/billing",
                SubjectMatcher::Anyone,
                vec![Action::ReleasesRead],
            ))
            .await
            .unwrap();

        for neighbour in ["@acme/billing-internal", "@acme/bill", "@other/billing"] {
            let rows = store.grants_for(reg, neighbour, None).await.unwrap();
            assert!(
                rows.is_empty(),
                "{name}: {neighbour} saw @acme/billing's grant"
            );
        }
    }
}

/// Version rows are only returned for the version asked about.
#[tokio::test]
async fn version_rows_are_scoped_to_the_version_named() {
    let reg = "grants-test-version";
    for (name, store) in stores(reg).await {
        store
            .put_grant(grant(
                reg,
                NodeKind::Version,
                &version_node_key("pkg", "1.0.0"),
                SubjectMatcher::Anyone,
                vec![Action::ReleasesRead],
            ))
            .await
            .unwrap();

        let asked = store.grants_for(reg, "pkg", Some("1.0.0")).await.unwrap();
        assert_eq!(asked.len(), 1, "{name}");

        let other = store.grants_for(reg, "pkg", Some("2.0.0")).await.unwrap();
        assert!(other.is_empty(), "{name}: a different version saw the row");

        // A listing names no version, so version rows must not appear: they
        // grant on a coordinate the caller did not ask about.
        let listing = store.grants_for(reg, "pkg", None).await.unwrap();
        assert!(
            listing.is_empty(),
            "{name}: a version-tier row reached a request that named no version"
        );
    }
}

/// Repeating a subject replaces that subject's row and leaves the others.
#[tokio::test]
async fn writing_a_subject_twice_replaces_only_that_subject() {
    let reg = "grants-test-upsert";
    for (name, store) in stores(reg).await {
        let alice = SubjectMatcher::parse("user:alice").unwrap();
        let bob = SubjectMatcher::parse("user:bob").unwrap();

        store
            .put_grant(grant(
                reg,
                NodeKind::Package,
                "pkg",
                alice.clone(),
                vec![Action::ReleasesRead],
            ))
            .await
            .unwrap();
        store
            .put_grant(grant(
                reg,
                NodeKind::Package,
                "pkg",
                bob.clone(),
                vec![Action::SourceRead],
            ))
            .await
            .unwrap();
        store
            .put_grant(grant(
                reg,
                NodeKind::Package,
                "pkg",
                alice.clone(),
                vec![Action::ReleasesPublish],
            ))
            .await
            .unwrap();

        let rows = store
            .grants_on_node(reg, NodeKind::Package, "pkg")
            .await
            .unwrap();
        assert_eq!(rows.len(), 2, "{name}: one row per subject");
        let alice_row = rows.iter().find(|r| r.subject == alice).expect("alice");
        assert_eq!(
            alice_row.actions,
            vec![Action::ReleasesPublish],
            "{name}: the second write replaces, it does not append"
        );
        assert!(
            rows.iter().any(|r| r.subject == bob),
            "{name}: bob survived"
        );
    }
}

/// A grant with no permissions is refused.
///
/// An empty action set is what a **seal** is, and §4.3 confines sealing to the
/// config file: a delegate holding `owners:write` may write package rows, and a
/// seal representable here would let them lock the registry owner out of a
/// package. §7 asks for that to be unwritable rather than rejected — the type
/// carries no way to say it, and this is the belt to that braces.
#[tokio::test]
async fn a_grant_with_no_permissions_is_refused() {
    for (name, store) in stores("grants-test-seal").await {
        let err = store
            .put_grant(grant(
                "grants-test-seal",
                NodeKind::Package,
                "pkg",
                SubjectMatcher::Anyone,
                vec![],
            ))
            .await;
        assert!(
            err.is_err(),
            "{name}: an empty grant is a seal and must not store"
        );
    }
}

/// Deleting a package's grants takes both tiers, and only that package's.
///
/// RFC 0016 §4.4: package-tier policy dies with the package, because grants
/// keyed by a name that outlive it leave a previous owner holding
/// `releases:publish` on a name someone else may take.
#[tokio::test]
async fn deleting_a_packages_grants_takes_both_tiers_and_no_neighbours() {
    let reg = "grants-test-delete";
    for (name, store) in stores(reg).await {
        for (kind, key) in [
            (NodeKind::Package, "@acme/billing".to_owned()),
            (
                NodeKind::Version,
                version_node_key("@acme/billing", "1.0.0"),
            ),
            (NodeKind::Package, "@acme/billing-internal".to_owned()),
            (
                NodeKind::Version,
                version_node_key("@acme/billing-internal", "1.0.0"),
            ),
        ] {
            store
                .put_grant(grant(
                    reg,
                    kind,
                    &key,
                    SubjectMatcher::Anyone,
                    vec![Action::ReleasesRead],
                ))
                .await
                .unwrap();
        }

        store
            .delete_package_grants(reg, "@acme/billing")
            .await
            .unwrap();

        assert!(
            store
                .grants_for(reg, "@acme/billing", Some("1.0.0"))
                .await
                .unwrap()
                .is_empty(),
            "{name}: the package's own rows survived"
        );
        let neighbour = store
            .grants_for(reg, "@acme/billing-internal", Some("1.0.0"))
            .await
            .unwrap();
        assert_eq!(
            neighbour.len(),
            2,
            "{name}: a neighbour's rows were deleted — the version tier is matched by \
             `package@`, not by a bare prefix"
        );
    }
}

/// A package name containing SQL pattern characters deletes only itself.
///
/// `%` and `_` are legal in an npm package name and are wildcards in `LIKE`. An
/// unescaped one would delete the version grants of every package that matched
/// the pattern — a destructive operation with a blast radius the caller did not
/// name.
#[tokio::test]
async fn a_package_name_with_pattern_characters_deletes_only_itself() {
    let reg = "grants-test-escape";
    for (name, store) in stores(reg).await {
        let hostile = "a_b";
        let bystander = "axb";

        for pkg in [hostile, bystander] {
            store
                .put_grant(grant(
                    reg,
                    NodeKind::Version,
                    &version_node_key(pkg, "1.0.0"),
                    SubjectMatcher::Anyone,
                    vec![Action::ReleasesRead],
                ))
                .await
                .unwrap();
        }

        store.delete_package_grants(reg, hostile).await.unwrap();

        assert!(
            store
                .grants_for(reg, hostile, Some("1.0.0"))
                .await
                .unwrap()
                .is_empty(),
            "{name}: the named package's rows survived"
        );
        assert_eq!(
            store
                .grants_for(reg, bystander, Some("1.0.0"))
                .await
                .unwrap()
                .len(),
            1,
            "{name}: `_` was treated as a wildcard and took an unrelated package with it"
        );
    }
}

/// An empty package name matches nothing.
///
/// Survey finding 2's shape: a predicate that runs, matches everything, and
/// looks like scoping. Asserted on both the read and the delete, because the
/// delete is the one where being wrong destroys data.
#[tokio::test]
async fn an_empty_package_name_matches_nothing() {
    let reg = "grants-test-empty";
    for (name, store) in stores(reg).await {
        store
            .put_grant(grant(
                reg,
                NodeKind::Package,
                "pkg",
                SubjectMatcher::Anyone,
                vec![Action::ReleasesRead],
            ))
            .await
            .unwrap();

        assert!(
            store.grants_for(reg, "", None).await.unwrap().is_empty(),
            "{name}: an empty package name returned rows"
        );
        store.delete_package_grants(reg, "").await.unwrap();
        assert_eq!(
            store.grants_for(reg, "pkg", None).await.unwrap().len(),
            1,
            "{name}: an empty package name deleted the registry's rows"
        );
    }
}

/// Registries do not see each other's grants.
#[tokio::test]
async fn grants_are_scoped_to_their_registry() {
    for (name, store) in stores("grants-test-reg-a").await {
        store
            .put_grant(grant(
                "grants-test-reg-a",
                NodeKind::Package,
                "pkg",
                SubjectMatcher::Anyone,
                vec![Action::ReleasesRead],
            ))
            .await
            .unwrap();
        assert!(
            store
                .grants_for("grants-test-reg-b", "pkg", None)
                .await
                .unwrap()
                .is_empty(),
            "{name}: a grant crossed a registry boundary"
        );
    }
}

// ── The ownership migration (§10 rule 9) ─────────────────────────────────────

/// `package_owners` rows become package-tier grants, with the subject form
/// preserved.
///
/// Postgres only: the migration is SQL, and running it is the thing under test.
/// Skipped without `DATABASE_URL`, and it says so — a silently-skipped migration
/// test is how a migration ships unrun.
#[tokio::test]
async fn ownership_rows_migrate_to_package_tier_grants() {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        eprintln!("note: DATABASE_URL unset — the ownership migration was not exercised");
        return;
    };
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .expect("connect");
    batlehub_adapters::migrations::embedded_migrator()
        .run(&pool)
        .await
        .expect("migrations");

    let reg = "grants-test-ownership";
    sqlx::query("DELETE FROM grants WHERE registry = $1")
        .bind(reg)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM package_owners WHERE registry = $1")
        .bind(reg)
        .execute(&pool)
        .await
        .unwrap();

    // Three principal shapes, and one package with no owner at all.
    for (ptype, pid, pkg) in [
        ("user", "alice", "owned-by-user"),
        ("group", "eng", "owned-by-bare-group"),
        ("group", "oidc1:ops", "owned-by-prefixed-group"),
    ] {
        sqlx::query(
            "INSERT INTO package_owners (registry, package_name, principal_type, principal_id) \
             VALUES ($1, $2, $3, $4)",
        )
        .bind(reg)
        .bind(pkg)
        .bind(ptype)
        .bind(pid)
        .execute(&pool)
        .await
        .unwrap();
    }

    // Re-run the migration body against the rows just inserted. The migrator
    // itself has already run and will not run again, so the statement is
    // replayed here — which is also what proves it is idempotent.
    let sql = include_str!("../migrations/042_ownership_to_grants.sql");
    sqlx::raw_sql(sql)
        .execute(&pool)
        .await
        .expect("migration body");
    sqlx::raw_sql(sql)
        .execute(&pool)
        .await
        .expect("re-run is a no-op");

    let store = PgGrantRepository::new(pool.clone());

    for (pkg, expected) in [
        ("owned-by-user", "user:alice"),
        // A bare group principal keeps its bare shape. Reading it as
        // `group:*:eng` would make it start matching `oidc1:eng` — the widening
        // RFC 0015 §13.5 records as the migration's sharpest edge.
        ("owned-by-bare-group", "group::eng"),
        ("owned-by-prefixed-group", "group:oidc1:ops"),
    ] {
        let rows = store.grants_for(reg, pkg, None).await.unwrap();
        assert_eq!(
            rows.len(),
            1,
            "{pkg}: exactly one grant, even after two runs"
        );
        assert_eq!(rows[0].subject.as_string(), expected, "{pkg}");
        assert_eq!(
            rows[0].actions,
            vec![
                Action::ReleasesPublish,
                Action::OwnersRead,
                Action::OwnersWrite
            ],
            "{pkg}: §10 rule 9's three verbs, and no more"
        );
    }

    // §7: an unowned package gets no grant. "The migration writes no grant for
    // an unowned package, and no grant denies" — the reading that turned into
    // survey finding 1 was the opposite one.
    assert!(
        store
            .grants_for(reg, "never-owned", None)
            .await
            .unwrap()
            .is_empty(),
        "an unowned package must not acquire a grant"
    );

    sqlx::query("DELETE FROM package_owners WHERE registry = $1")
        .bind(reg)
        .execute(&pool)
        .await
        .unwrap();
}
