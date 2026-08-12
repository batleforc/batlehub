//! Integration tests for `PgStatsHistoryRepository` (RFC 0004 §10).
//!
//! Worth a real database: the upsert is what keeps a second tick inside one
//! hour from double-counting, and an in-memory double agreeing with itself
//! proves nothing about `ON CONFLICT`.
//!
//!   task test:pg-stats-history
//!   DATABASE_URL=postgresql://postgres:pass@localhost/postgres \
//!     cargo test -p batlehub-adapters --test pg_stats_history

use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{DateTime, Duration, TimeZone, Utc};
use sqlx::PgPool;

use batlehub_adapters::db::PgStatsHistoryRepository;
use batlehub_core::ports::{StatsHistoryRepository, StatsRollupRow};

fn db_url() -> Option<String> {
    std::env::var("DATABASE_URL").ok()
}

static TEST_ID: AtomicU64 = AtomicU64::new(0);

struct TestRepo {
    repo: PgStatsHistoryRepository,
    prefix: String,
}

impl TestRepo {
    fn reg(&self, name: &str) -> String {
        format!("{name}-{}", self.prefix)
    }
}

async fn make_repo(url: &str) -> TestRepo {
    let id = TEST_ID.fetch_add(1, Ordering::Relaxed);
    let pool = PgPool::connect(url).await.expect("connect to postgres");
    batlehub_adapters::migrations::embedded_migrator()
        .run(&pool)
        .await
        .expect("run migrations");
    TestRepo {
        repo: PgStatsHistoryRepository::new(pool),
        prefix: format!("t{id}"),
    }
}

/// A fixed base so windows never collide with another test's rows.
fn at(hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2020, 1, 1, hour, 0, 0).unwrap()
}

fn row(registry: &str, hour: u32, hits: u64, misses: u64) -> StatsRollupRow {
    StatsRollupRow {
        registry: registry.to_owned(),
        window_start: at(hour),
        hits,
        misses,
        cached_bytes: 2_048,
    }
}

#[tokio::test]
async fn append_then_read_window_round_trips() {
    let Some(url) = db_url() else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let t = make_repo(&url).await;
    let reg = t.reg("npm");

    t.repo
        .append(&[row(&reg, 10, 8, 2), row(&reg, 11, 6, 4)])
        .await
        .unwrap();

    let rows = t.repo.read_window(at(10), at(12)).await.unwrap();
    let mine: Vec<_> = rows.into_iter().filter(|r| r.registry == reg).collect();
    assert_eq!(mine.len(), 2);
    assert_eq!(mine[0].window_start, at(10), "oldest first");
    assert_eq!(mine[0].hits, 8);
    assert_eq!(mine[0].cached_bytes, 2_048);
    assert_eq!(mine[1].hits, 6);
}

/// The property `ON CONFLICT … DO UPDATE` exists for: a writer that runs twice
/// inside one hour must overwrite, not add.
#[tokio::test]
async fn appending_the_same_window_twice_upserts() {
    let Some(url) = db_url() else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let t = make_repo(&url).await;
    let reg = t.reg("npm");

    t.repo.append(&[row(&reg, 10, 5, 5)]).await.unwrap();
    t.repo.append(&[row(&reg, 10, 9, 1)]).await.unwrap();

    let mine: Vec<_> = t
        .repo
        .read_window(at(10), at(11))
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.registry == reg)
        .collect();
    assert_eq!(mine.len(), 1, "one row per (registry, window)");
    assert_eq!(mine[0].hits, 9, "the later write wins");
}

#[tokio::test]
async fn registries_do_not_share_a_window_row() {
    let Some(url) = db_url() else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let t = make_repo(&url).await;
    let (npm, cargo) = (t.reg("npm"), t.reg("cargo"));

    t.repo
        .append(&[row(&npm, 10, 1, 0), row(&cargo, 10, 2, 0)])
        .await
        .unwrap();

    let rows = t.repo.read_window(at(10), at(11)).await.unwrap();
    assert_eq!(rows.iter().filter(|r| r.registry == npm).count(), 1);
    assert_eq!(rows.iter().filter(|r| r.registry == cargo).count(), 1);
}

#[tokio::test]
async fn read_window_is_half_open() {
    let Some(url) = db_url() else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let t = make_repo(&url).await;
    let reg = t.reg("npm");

    t.repo
        .append(&[row(&reg, 10, 1, 0), row(&reg, 11, 2, 0)])
        .await
        .unwrap();

    let mine: Vec<_> = t
        .repo
        .read_window(at(10), at(11))
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.registry == reg)
        .collect();
    assert_eq!(mine.len(), 1, "[from, to) excludes the upper bound");
    assert_eq!(mine[0].window_start, at(10));
}

#[tokio::test]
async fn prune_deletes_strictly_before_the_cutoff() {
    let Some(url) = db_url() else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let t = make_repo(&url).await;
    let reg = t.reg("npm");

    t.repo
        .append(&[
            row(&reg, 10, 1, 0),
            row(&reg, 11, 2, 0),
            row(&reg, 12, 3, 0),
        ])
        .await
        .unwrap();

    t.repo.prune_before(at(12)).await.unwrap();

    let mine: Vec<_> = t
        .repo
        .read_window(at(0), at(23))
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.registry == reg)
        .collect();
    assert_eq!(mine.len(), 1);
    assert_eq!(
        mine[0].window_start,
        at(12),
        "the cutoff row itself survives"
    );
}

#[tokio::test]
async fn appending_nothing_is_not_an_error() {
    let Some(url) = db_url() else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let t = make_repo(&url).await;
    t.repo.append(&[]).await.unwrap();
}

/// A year of hourly rows per registry is the volume R9 called "not a storage
/// argument"; a 30-day read has to stay a single cheap query over it.
#[tokio::test]
async fn a_large_window_reads_back_completely() {
    let Some(url) = db_url() else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let t = make_repo(&url).await;
    let reg = t.reg("npm");

    let rows: Vec<StatsRollupRow> = (0..24).map(|h| row(&reg, h, u64::from(h), 1)).collect();
    t.repo.append(&rows).await.unwrap();

    let mine: Vec<_> = t
        .repo
        .read_window(at(0), at(23) + Duration::hours(1))
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.registry == reg)
        .collect();
    assert_eq!(mine.len(), 24);
    assert_eq!(
        mine.iter().map(|r| r.hits).sum::<u64>(),
        (0..24).sum::<u64>()
    );
}
