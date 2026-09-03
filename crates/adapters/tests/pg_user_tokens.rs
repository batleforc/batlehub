//! RFC 0011-bis §4.4 — the PAT group snapshot, against real Postgres.
//!
//! The in-memory double in `crates/web/tests/tokens_and_pagination.rs` covers
//! the endpoint and the provider; it cannot cover migration 046, the `TEXT[]`
//! binding, or the four `SELECT` column lists that have to name the new column.
//! A `groups` left out of one of them is a token that authenticates with an
//! empty snapshot on one path and a full one on another — silently, because
//! `groups: vec![]` is exactly what the feature replaced.
//!
//! Requires a running PostgreSQL instance; set `DATABASE_URL` to opt in.
//! Without it every test here is skipped rather than passing vacuously.

use std::sync::atomic::{AtomicU64, Ordering};

use batlehub_adapters::db::packages::PoolOptions;
use batlehub_adapters::db::PgPackageRepository;
use batlehub_core::{
    entities::Role,
    ports::{TokenOwner, UserTokenRepository},
};
use chrono::{Duration, Utc};
use uuid::Uuid;

static TEST_ID: AtomicU64 = AtomicU64::new(0);

/// A fresh principal per test: `user_tokens` has a unique index on
/// `(user_id, name)`, so a shared owner would make these tests order-dependent
/// against a database that outlives the run.
fn owner() -> TokenOwner {
    TokenOwner::new(
        "authentik",
        format!(
            "alice-{}-{}",
            std::process::id(),
            TEST_ID.fetch_add(1, Ordering::Relaxed)
        ),
    )
}

async fn repo() -> Option<PgPackageRepository> {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        eprintln!("skipping: DATABASE_URL not set");
        return None;
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
    Some(repo)
}

fn hash(seed: &str) -> String {
    format!("hash-{seed}-{}", Uuid::new_v4())
}

/// Creation returns the snapshot, and every read path returns it too.
///
/// `find_by_hash` is the one the auth provider calls on every request and
/// `list_for_user` is the one the console renders; they select from the same
/// table through different statements, which is exactly how one of them ends up
/// missing a column.
#[tokio::test]
async fn a_snapshot_round_trips_through_every_read_path() {
    let Some(repo) = repo().await else { return };
    let owner = owner();
    let h = hash("round-trip");
    let groups = vec!["authentik:eng".to_owned(), "platform team".to_owned()];

    let created = repo
        .create_token(
            Uuid::new_v4(),
            &owner,
            "ci",
            &h,
            Role::User,
            Utc::now() + Duration::days(30),
            &groups,
        )
        .await
        .expect("create");
    assert_eq!(created.groups, groups, "RETURNING must name the column");

    let found = repo
        .find_by_hash(&h)
        .await
        .expect("find")
        .expect("token is live");
    assert_eq!(found.groups, groups, "the auth provider's read path");

    let listed = repo.list_for_user(&owner).await.expect("list");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].groups, groups, "the console's read path");
}

/// The column's default. A token minted before migration 046 has no array to
/// read, and `NOT NULL DEFAULT '{}'` is what makes that an empty snapshot
/// rather than a row this code cannot deserialise (§10).
#[tokio::test]
async fn a_token_minted_with_no_groups_reads_back_empty() {
    let Some(repo) = repo().await else { return };
    let owner = owner();
    let h = hash("empty");

    repo.create_token(
        Uuid::new_v4(),
        &owner,
        "quiet",
        &h,
        Role::User,
        Utc::now() + Duration::days(1),
        &[],
    )
    .await
    .expect("create");

    let found = repo.find_by_hash(&h).await.expect("find").expect("live");
    assert!(found.groups.is_empty());
}

/// Group ids are compared literally everywhere else in this tree, so they have
/// to survive storage literally: `%`, `_` and a `.` separator are data, not
/// pattern syntax, and a space is significant enough that `snapshot_pat_groups`
/// has a rule about it.
#[tokio::test]
async fn group_ids_survive_storage_verbatim() {
    let Some(repo) = repo().await else { return };
    let owner = owner();
    let h = hash("literal");
    let groups = vec![
        "k8s:system:serviceaccounts:digital".to_owned(),
        "50%_off".to_owned(),
        "platform team".to_owned(),
        "digital.pipeline".to_owned(),
    ];

    repo.create_token(
        Uuid::new_v4(),
        &owner,
        "literal",
        &h,
        Role::User,
        Utc::now() + Duration::days(1),
        &groups,
    )
    .await
    .expect("create");

    let found = repo.find_by_hash(&h).await.expect("find").expect("live");
    assert_eq!(found.groups, groups);
}

/// A revoked token is not a narrower token — it is no token. The snapshot must
/// not keep it authenticating.
#[tokio::test]
async fn a_revoked_token_stops_resolving_snapshot_and_all() {
    let Some(repo) = repo().await else { return };
    let owner = owner();
    let h = hash("revoked");
    let id = Uuid::new_v4();

    repo.create_token(
        id,
        &owner,
        "doomed",
        &h,
        Role::User,
        Utc::now() + Duration::days(1),
        &["authentik:eng".to_owned()],
    )
    .await
    .expect("create");

    assert!(repo.revoke(id, &owner).await.expect("revoke"));
    assert!(
        repo.find_by_hash(&h).await.expect("find").is_none(),
        "a revoked token resolves to nothing at all"
    );
}
