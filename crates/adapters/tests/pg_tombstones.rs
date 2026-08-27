//! Integration tests for the tombstone half of `PostgresLocalRegistry`
//! (RFC 0016 §4.4, §4.5).
//!
//! Worth a real database for four things the in-memory double agreeing with
//! itself proves nothing about. The `uq_local_package` unique constraint is what
//! makes the tombstone physically occupy its coordinate — the in-memory store
//! has one row per version by construction and cannot be wrong about it. The
//! `ck_local_packages_live_checksum` CHECK is what stops compaction's nulled
//! checksum from ever appearing on a live row, and it exists only in SQL. The
//! `UPDATE … RETURNING` in compaction is the statement whose report cannot
//! disagree with its own write, which is the property being claimed. And a
//! `DROP NOT NULL` that a reader then decodes into a `String` is the kind of
//! mismatch that only fails against a column that can actually be null.
//!
//!   task test:pg-tombstones
//!   DATABASE_URL=postgresql://postgres:pass@localhost/postgres \
//!     cargo test -p batlehub-adapters --test pg_tombstones

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use chrono::Utc;
use sqlx::{PgPool, Row};

use batlehub_adapters::local_registry::PostgresLocalRegistry;
use batlehub_core::{
    entities::{PublishedPackage, Visibility},
    error::CoreError,
    ports::LocalRegistryBackend,
};

fn db_url() -> Option<String> {
    std::env::var("DATABASE_URL").ok()
}

static TEST_ID: AtomicU64 = AtomicU64::new(0);

struct TestRegistry {
    backend: PostgresLocalRegistry,
    pool: PgPool,
    registry: String,
}

/// A registry name unique per *run*, for the reason `pg_readmes.rs` gives: these
/// tests write and delete by `(registry, name, version)`, so a second run
/// against the same database would read the previous run's rows. Pids are
/// recycled, so the fixture also clears anything a long-dead namesake left.
async fn make_registry(url: &str) -> TestRegistry {
    let id = TEST_ID.fetch_add(1, Ordering::Relaxed);
    let registry = format!("tomb-t{}-{id}", std::process::id());
    let pool = PgPool::connect(url).await.expect("connect to postgres");
    batlehub_adapters::migrations::embedded_migrator()
        .run(&pool)
        .await
        .expect("run migrations");
    sqlx::query("DELETE FROM local_packages WHERE registry = $1")
        .bind(&registry)
        .execute(&pool)
        .await
        .expect("clear rows from a previous run under this registry name");
    TestRegistry {
        backend: PostgresLocalRegistry::new(pool.clone()),
        pool,
        registry,
    }
}

impl TestRegistry {
    fn pkg(&self, name: &str, version: &str) -> PublishedPackage {
        PublishedPackage {
            registry: self.registry.clone(),
            name: name.to_owned(),
            version: version.to_owned(),
            checksum: format!("sum-{name}-{version}"),
            yanked: false,
            deprecated: false,
            deprecation_message: None,
            unlisted: false,
            index_metadata: serde_json::json!({ "big": "x".repeat(512) }),
            published_at: Utc::now(),
            published_by: Some("publisher-1".to_owned()),
            signature_bytes: Some(vec![1, 2, 3]),
            signature_type: Some("ed25519".to_owned()),
            visibility: Visibility::Public,
        }
    }

    /// Publish and commit in one step, for the tests that are about what happens
    /// afterwards.
    async fn publish(&self, name: &str, version: &str) {
        self.backend.publish(self.pkg(name, version)).await.unwrap();
        self.backend
            .commit_publish(&self.registry, name, version)
            .await
            .unwrap();
    }

    /// Backdate a tombstone so a compaction window can pass over it without the
    /// test waiting. Compaction's predicate reads `deleted_at`, so this is the
    /// one column that has to move.
    async fn backdate_deletion(&self, name: &str, version: &str, days: i64) {
        sqlx::query(
            "UPDATE local_packages SET deleted_at = NOW() - ($4 || ' days')::INTERVAL \
             WHERE registry = $1 AND name = $2 AND version = $3",
        )
        .bind(&self.registry)
        .bind(name)
        .bind(version)
        .bind(days)
        .execute(&self.pool)
        .await
        .unwrap();
    }

    /// Read a row's raw columns, including the ones no port method exposes.
    async fn raw(&self, name: &str, version: &str) -> sqlx::postgres::PgRow {
        sqlx::query(
            "SELECT status, deleted_at, deleted_by, detail_compacted_at, checksum, \
                    index_metadata, published_by, signature_bytes, signature_type \
             FROM local_packages WHERE registry = $1 AND name = $2 AND version = $3",
        )
        .bind(&self.registry)
        .bind(name)
        .bind(version)
        .fetch_one(&self.pool)
        .await
        .expect("the row must still exist")
    }
}

// ── The coordinate is spent ───────────────────────────────────────────────────

/// The headline invariant, against the store that actually enforces it.
#[tokio::test]
async fn a_deleted_coordinate_refuses_a_republish() {
    let Some(url) = db_url() else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let t = make_registry(&url).await;

    t.publish("widgets", "1.4.0").await;
    assert!(t
        .backend
        .tombstone_version(&t.registry, "widgets", "1.4.0", Some("alice"))
        .await
        .unwrap());

    let err = t
        .backend
        .publish(t.pkg("widgets", "1.4.0"))
        .await
        .expect_err("a spent coordinate must refuse a re-publish");
    match err {
        CoreError::Conflict(m) => assert!(
            m.contains("never reused"),
            "the refusal must say the coordinate is spent, not that it is published: {m}"
        ),
        other => panic!("expected a Conflict, got {other:?}"),
    }
}

/// The tombstone leaves every listing the moment it is written, and the row is
/// marked `deleted` as well as timestamped — the belt to the predicate's braces.
#[tokio::test]
async fn a_tombstone_leaves_every_listing_and_carries_both_markers() {
    let Some(url) = db_url() else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let t = make_registry(&url).await;

    t.publish("widgets", "1.0.0").await;
    t.publish("widgets", "2.0.0").await;
    t.backend
        .tombstone_version(&t.registry, "widgets", "1.0.0", Some("alice"))
        .await
        .unwrap();

    let versions = t
        .backend
        .get_versions(&t.registry, "widgets")
        .await
        .unwrap();
    assert_eq!(versions.len(), 1);
    assert_eq!(versions[0].version, "2.0.0");

    let row = t.raw("widgets", "1.0.0").await;
    assert_eq!(row.get::<String, _>("status"), "deleted");
    assert!(row
        .get::<Option<chrono::DateTime<Utc>>, _>("deleted_at")
        .is_some());
    assert_eq!(
        row.get::<Option<String>, _>("deleted_by").as_deref(),
        Some("alice")
    );
}

/// A package whose every version is tombstoned stops being a package: absent
/// from `exists` and from the name catalogue that `list_package_names` builds.
/// That is a different query from `get_versions` and would not be fixed by the
/// same edit.
#[tokio::test]
async fn a_fully_deleted_package_leaves_exists_and_the_catalogue() {
    let Some(url) = db_url() else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let t = make_registry(&url).await;

    t.publish("gone", "1.0.0").await;
    t.publish("stays", "1.0.0").await;
    t.backend
        .tombstone_version(&t.registry, "gone", "1.0.0", None)
        .await
        .unwrap();

    assert!(!t.backend.exists(&t.registry, "gone").await.unwrap());
    assert!(t.backend.exists(&t.registry, "stays").await.unwrap());
    assert_eq!(
        t.backend.list_package_names(&t.registry).await.unwrap(),
        vec!["stays".to_owned()],
    );
}

/// Deleting twice returns `false` and leaves the first `deleted_at` alone. That
/// timestamp is what compaction ages against, so a re-stamp would silently
/// postpone the window every time someone re-ran a bulk delete.
#[tokio::test]
async fn tombstoning_is_idempotent_and_keeps_the_first_timestamp() {
    let Some(url) = db_url() else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let t = make_registry(&url).await;

    t.publish("twice", "1.0.0").await;
    assert!(t
        .backend
        .tombstone_version(&t.registry, "twice", "1.0.0", Some("alice"))
        .await
        .unwrap());
    let first = t
        .backend
        .find_tombstone(&t.registry, "twice", "1.0.0")
        .await
        .unwrap()
        .unwrap()
        .deleted_at;

    assert!(!t
        .backend
        .tombstone_version(&t.registry, "twice", "1.0.0", Some("bob"))
        .await
        .unwrap());
    let ts = t
        .backend
        .find_tombstone(&t.registry, "twice", "1.0.0")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(ts.deleted_at, first);
    assert_eq!(ts.deleted_by.as_deref(), Some("alice"));
}

/// `remove_version` is the publish rollback and must not be able to erase a
/// tombstone — it is the only `DELETE` left against this table.
#[tokio::test]
async fn remove_version_erases_a_pending_row_but_never_a_tombstone() {
    let Some(url) = db_url() else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let t = make_registry(&url).await;

    // The case it exists for: a reserved row whose publish never committed.
    t.backend
        .publish(t.pkg("rolled-back", "1.0.0"))
        .await
        .unwrap();
    t.backend
        .remove_version(&t.registry, "rolled-back", "1.0.0")
        .await
        .unwrap();
    assert!(t
        .backend
        .publish(t.pkg("rolled-back", "1.0.0"))
        .await
        .is_ok());

    // The case it must refuse.
    t.publish("burned", "1.0.0").await;
    t.backend
        .tombstone_version(&t.registry, "burned", "1.0.0", None)
        .await
        .unwrap();
    t.backend
        .remove_version(&t.registry, "burned", "1.0.0")
        .await
        .unwrap();
    assert!(
        t.backend
            .find_tombstone(&t.registry, "burned", "1.0.0")
            .await
            .unwrap()
            .is_some(),
        "the tombstone must survive the rollback primitive"
    );
}

/// A live row can never carry the null checksum compaction writes. Asserted
/// against the CHECK constraint directly, because that is the guard: nothing in
/// Rust stops a future writer from binding a `None` here.
#[tokio::test]
async fn a_live_row_cannot_have_a_null_checksum() {
    let Some(url) = db_url() else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let t = make_registry(&url).await;
    t.publish("checked", "1.0.0").await;

    let err = sqlx::query(
        "UPDATE local_packages SET checksum = NULL \
         WHERE registry = $1 AND name = $2 AND version = $3",
    )
    .bind(&t.registry)
    .bind("checked")
    .bind("1.0.0")
    .execute(&t.pool)
    .await
    .expect_err("ck_local_packages_live_checksum must refuse this");
    assert!(
        err.to_string().contains("ck_local_packages_live_checksum"),
        "expected the live-checksum CHECK to fire, got: {err}"
    );
}

// ── Compaction ────────────────────────────────────────────────────────────────

/// Compaction strips exactly the detail columns, keeps the claim, and the claim
/// still refuses a re-publish afterwards.
#[tokio::test]
async fn compaction_strips_detail_and_keeps_the_claim() {
    let Some(url) = db_url() else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let t = make_registry(&url).await;

    t.publish("old", "1.0.0").await;
    t.backend
        .tombstone_version(&t.registry, "old", "1.0.0", Some("alice"))
        .await
        .unwrap();
    t.backdate_deletion("old", "1.0.0", 800).await;

    let report = t
        .backend
        .compact_tombstone_detail(&t.registry, Duration::from_secs(730 * 86_400), false)
        .await
        .unwrap();
    assert_eq!(report.compacted, 1);
    assert_eq!(report.coordinates, vec!["old@1.0.0".to_owned()]);

    let row = t.raw("old", "1.0.0").await;
    assert!(row.get::<Option<String>, _>("checksum").is_none());
    assert!(row.get::<Option<String>, _>("published_by").is_none());
    assert!(row.get::<Option<Vec<u8>>, _>("signature_bytes").is_none());
    assert!(row.get::<Option<String>, _>("signature_type").is_none());
    assert_eq!(
        row.get::<serde_json::Value, _>("index_metadata"),
        serde_json::json!({}),
        "index_metadata is emptied rather than nulled — it is NOT NULL and three bytes is not \
         what accumulates"
    );
    assert!(row
        .get::<Option<chrono::DateTime<Utc>>, _>("detail_compacted_at")
        .is_some());
    assert_eq!(
        row.get::<Option<String>, _>("deleted_by").as_deref(),
        Some("alice"),
        "who deleted it is the claim's provenance, not detail"
    );

    let err = t.backend.publish(t.pkg("old", "1.0.0")).await.unwrap_err();
    assert!(
        matches!(err, CoreError::Conflict(ref m) if m.contains("never reused")),
        "a compacted tombstone still spends its coordinate, got {err:?}"
    );
}

/// The dry run reports what the live run then strips — the two SQL statements
/// share a `WHERE` clause by repetition rather than by construction, and this is
/// what keeps them honest.
#[tokio::test]
async fn compaction_dry_run_writes_nothing_and_agrees_with_the_live_run() {
    let Some(url) = db_url() else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let t = make_registry(&url).await;

    for v in ["1.0.0", "2.0.0"] {
        t.publish("dry", v).await;
        t.backend
            .tombstone_version(&t.registry, "dry", v, None)
            .await
            .unwrap();
        t.backdate_deletion("dry", v, 800).await;
    }
    // One inside the window, so the run has something to skip.
    t.publish("dry", "3.0.0").await;
    t.backend
        .tombstone_version(&t.registry, "dry", "3.0.0", None)
        .await
        .unwrap();

    let window = Duration::from_secs(730 * 86_400);
    let preview = t
        .backend
        .compact_tombstone_detail(&t.registry, window, true)
        .await
        .unwrap();
    assert!(preview.dry_run);
    assert_eq!(preview.compacted, 2);
    assert_eq!(preview.skipped, 1);
    assert!(
        t.raw("dry", "1.0.0")
            .await
            .get::<Option<String>, _>("checksum")
            .is_some(),
        "a dry run must not have stripped anything"
    );

    let live = t
        .backend
        .compact_tombstone_detail(&t.registry, window, false)
        .await
        .unwrap();
    assert!(!live.dry_run);
    assert_eq!(live.coordinates, preview.coordinates);
    assert!(t
        .raw("dry", "3.0.0")
        .await
        .get::<Option<String>, _>("checksum")
        .is_some());
}

/// A second run is a no-op rather than a re-stamp, so `skipped` means the same
/// thing on every run.
#[tokio::test]
async fn compaction_is_a_no_op_the_second_time() {
    let Some(url) = db_url() else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let t = make_registry(&url).await;

    t.publish("twice", "1.0.0").await;
    t.backend
        .tombstone_version(&t.registry, "twice", "1.0.0", None)
        .await
        .unwrap();
    t.backdate_deletion("twice", "1.0.0", 800).await;

    let window = Duration::from_secs(730 * 86_400);
    let first = t
        .backend
        .compact_tombstone_detail(&t.registry, window, false)
        .await
        .unwrap();
    assert_eq!(first.compacted, 1);
    let stamp = t
        .raw("twice", "1.0.0")
        .await
        .get::<Option<chrono::DateTime<Utc>>, _>("detail_compacted_at");

    let second = t
        .backend
        .compact_tombstone_detail(&t.registry, window, false)
        .await
        .unwrap();
    assert_eq!(second.compacted, 0);
    assert_eq!(second.skipped, 1);
    assert_eq!(
        t.raw("twice", "1.0.0")
            .await
            .get::<Option<chrono::DateTime<Utc>>, _>("detail_compacted_at"),
        stamp,
        "the compaction timestamp must not move"
    );
}

/// Compaction never touches a live row, asserted by comparing every column it
/// could have written.
#[tokio::test]
async fn compaction_never_touches_a_live_row() {
    let Some(url) = db_url() else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let t = make_registry(&url).await;

    t.publish("alive", "1.0.0").await;
    let before = t.raw("alive", "1.0.0").await;

    let report = t
        .backend
        .compact_tombstone_detail(&t.registry, Duration::from_secs(0), false)
        .await
        .unwrap();
    assert_eq!(report.compacted, 0);

    let after = t.raw("alive", "1.0.0").await;
    assert_eq!(
        after.get::<Option<String>, _>("checksum"),
        before.get::<Option<String>, _>("checksum")
    );
    assert_eq!(
        after.get::<serde_json::Value, _>("index_metadata"),
        before.get::<serde_json::Value, _>("index_metadata")
    );
    assert_eq!(
        after.get::<Option<String>, _>("published_by"),
        before.get::<Option<String>, _>("published_by")
    );
    assert_eq!(
        after.get::<Option<Vec<u8>>, _>("signature_bytes"),
        before.get::<Option<Vec<u8>>, _>("signature_bytes")
    );
    assert!(after
        .get::<Option<chrono::DateTime<Utc>>, _>("detail_compacted_at")
        .is_none());
}

/// Compaction is scoped to the registry it names. The table is shared by every
/// registry on the instance, and a sweep that ignored the column would strip
/// another team's audit history.
#[tokio::test]
async fn compaction_is_scoped_to_one_registry() {
    let Some(url) = db_url() else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let a = make_registry(&url).await;
    let b = make_registry(&url).await;

    for t in [&a, &b] {
        t.publish("shared", "1.0.0").await;
        t.backend
            .tombstone_version(&t.registry, "shared", "1.0.0", None)
            .await
            .unwrap();
        t.backdate_deletion("shared", "1.0.0", 800).await;
    }

    let report = a
        .backend
        .compact_tombstone_detail(&a.registry, Duration::from_secs(730 * 86_400), false)
        .await
        .unwrap();
    assert_eq!(report.compacted, 1);
    assert!(
        b.raw("shared", "1.0.0")
            .await
            .get::<Option<String>, _>("checksum")
            .is_some(),
        "the other registry's tombstone detail must be untouched"
    );
}

/// The listing the audit view reads, including a compacted row — which is the
/// case the `Option` columns exist for.
#[tokio::test]
async fn list_tombstones_returns_both_compacted_and_intact_rows() {
    let Some(url) = db_url() else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let t = make_registry(&url).await;

    t.publish("mixed", "1.0.0").await;
    t.publish("mixed", "2.0.0").await;
    t.publish("other", "1.0.0").await;
    for (name, version) in [("mixed", "1.0.0"), ("mixed", "2.0.0"), ("other", "1.0.0")] {
        t.backend
            .tombstone_version(&t.registry, name, version, Some("alice"))
            .await
            .unwrap();
    }
    t.backdate_deletion("mixed", "1.0.0", 800).await;
    t.backend
        .compact_tombstone_detail(&t.registry, Duration::from_secs(730 * 86_400), false)
        .await
        .unwrap();

    let all = t.backend.list_tombstones(&t.registry, None).await.unwrap();
    assert_eq!(all.len(), 3);

    let mixed = t
        .backend
        .list_tombstones(&t.registry, Some("mixed"))
        .await
        .unwrap();
    assert_eq!(mixed.len(), 2, "the name filter narrows the list");

    let compacted = mixed
        .iter()
        .find(|ts| ts.version == "1.0.0")
        .expect("the compacted row is still listed");
    assert!(compacted.is_compacted());
    assert!(compacted.checksum.is_none());

    let intact = mixed
        .iter()
        .find(|ts| ts.version == "2.0.0")
        .expect("the intact row");
    assert!(!intact.is_compacted());
    assert!(intact.checksum.is_some());
}
