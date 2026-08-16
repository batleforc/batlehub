//! Integration tests for `PgStatsHistoryRepository` (RFC 0004 §10).
//!
//! Worth a real database: the upsert is what keeps a second tick inside one
//! hour from losing the first one's delta, and an in-memory double agreeing
//! with itself proves nothing about `ON CONFLICT`.
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

/// The prefix has to be unique per *run*, not just per test, because `append`
/// accumulates on conflict: a second run against the same database would find
/// the previous run's row for `(registry, window_start)` and add to it, and the
/// suite would fail with counters that grow by one run's worth each time. The
/// process id supplies that, as it does in `pg_explore_resolution.rs`.
///
/// Pids are recycled eventually, so the fixture also clears any rows left under
/// its own prefix by a long-dead namesake. It touches nothing else.
async fn make_repo(url: &str) -> TestRepo {
    let id = TEST_ID.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let prefix = format!("t{pid}-{id}");
    let pool = PgPool::connect(url).await.expect("connect to postgres");
    batlehub_adapters::migrations::embedded_migrator()
        .run(&pool)
        .await
        .expect("run migrations");
    sqlx::query("DELETE FROM stats_history WHERE registry LIKE $1")
        .bind(format!("%-{prefix}"))
        .execute(&pool)
        .await
        .expect("clear rows from a previous run under this prefix");
    TestRepo {
        repo: PgStatsHistoryRepository::new(pool),
        prefix,
    }
}

/// A fixed base so windows never collide with another test's rows.
fn at(hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2020, 1, 1, hour, 0, 0).unwrap()
}

/// The day `prune_deletes_strictly_before_the_cutoff` works on.
///
/// `prune_before` is global — it deletes every registry's rows before the
/// cutoff, which is the contract, since retention is an operational setting and
/// not a per-registry one. So that test cannot share a day with the others: its
/// cutoff would delete rows a concurrently-running test had just written and
/// was about to read. Living a day *earlier* than every other test's rows is
/// what keeps its blast radius to its own.
fn at_prune_day(hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2019, 12, 31, hour, 0, 0).unwrap()
}

fn row_at(registry: &str, window_start: DateTime<Utc>, hits: u64, misses: u64) -> StatsRollupRow {
    StatsRollupRow {
        registry: registry.to_owned(),
        window_start,
        hits,
        misses,
        cached_bytes: 2_048,
    }
}

fn row(registry: &str, hour: u32, hits: u64, misses: u64) -> StatsRollupRow {
    row_at(registry, at(hour), hits, misses)
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
/// inside one hour must merge into one row rather than duplicate it — and it
/// merges by *summing* the counters, because each row is a delta since the
/// previous tick and two ticks in one hour carry disjoint deltas. Replacing
/// would mean a redeploy at 11:20 overwrote the 11:00 window the previous
/// process had already recorded, losing the hour instead of the partial window
/// the rollup's contract promises.
///
/// `cached_bytes` goes the other way: it is a level read from storage, not a
/// delta, so the later value replaces rather than adds. The in-memory double
/// asserts the same two properties in
/// `in_memory::stats_history`'s `append_accumulates_deltas_within_one_window`
/// and `append_replaces_cached_bytes_rather_than_summing_it`.
#[tokio::test]
async fn appending_the_same_window_twice_accumulates_deltas() {
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
    assert_eq!(mine[0].hits, 14, "5 + 9 — deltas of the same hour");
    assert_eq!(mine[0].misses, 6, "5 + 1");
    // Both writes carry 2 048; summing would report 4 096 bytes held.
    assert_eq!(mine[0].cached_bytes, 2_048, "a level, replaced not summed");
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
            row_at(&reg, at_prune_day(10), 1, 0),
            row_at(&reg, at_prune_day(11), 2, 0),
            row_at(&reg, at_prune_day(12), 3, 0),
        ])
        .await
        .unwrap();

    t.repo.prune_before(at_prune_day(12)).await.unwrap();

    let mine: Vec<_> = t
        .repo
        .read_window(at_prune_day(0), at(23))
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.registry == reg)
        .collect();
    assert_eq!(mine.len(), 1);
    assert_eq!(
        mine[0].window_start,
        at_prune_day(12),
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
