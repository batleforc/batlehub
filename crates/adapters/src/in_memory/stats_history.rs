use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use tokio::sync::RwLock;

use batlehub_core::{
    error::CoreError,
    ports::{StatsHistoryRepository, StatsRollupRow},
};

/// In-memory [`StatsHistoryRepository`], so `crates/web/tests/*` and any
/// deployment without Postgres keep working — as every other port has one.
#[derive(Debug, Default)]
pub struct InMemoryStatsHistory {
    rows: Arc<RwLock<Vec<StatsRollupRow>>>,
}

impl InMemoryStatsHistory {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
}

#[async_trait]
impl StatsHistoryRepository for InMemoryStatsHistory {
    async fn append(&self, rows: &[StatsRollupRow]) -> Result<(), CoreError> {
        let mut guard = self.rows.write().await;
        for row in rows {
            // Upsert on (registry, window_start), mirroring the Postgres
            // primary key: a writer that runs twice in one hour must not
            // double-count.
            guard.retain(|r| !(r.registry == row.registry && r.window_start == row.window_start));
            guard.push(row.clone());
        }
        guard.sort_by(|a, b| {
            a.window_start
                .cmp(&b.window_start)
                .then_with(|| a.registry.cmp(&b.registry))
        });
        Ok(())
    }

    async fn read_window(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<StatsRollupRow>, CoreError> {
        Ok(self
            .rows
            .read()
            .await
            .iter()
            .filter(|r| r.window_start >= from && r.window_start < to)
            .cloned()
            .collect())
    }

    async fn prune_before(&self, cutoff: DateTime<Utc>) -> Result<u64, CoreError> {
        let mut guard = self.rows.write().await;
        let before = guard.len();
        guard.retain(|r| r.window_start >= cutoff);
        Ok((before - guard.len()) as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn row(registry: &str, hour: u32, hits: u64) -> StatsRollupRow {
        StatsRollupRow {
            registry: registry.to_owned(),
            window_start: Utc.with_ymd_and_hms(2026, 8, 12, hour, 0, 0).unwrap(),
            hits,
            misses: 1,
            cached_bytes: 100,
        }
    }

    fn at(hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 12, hour, 0, 0).unwrap()
    }

    #[tokio::test]
    async fn append_then_read_window_is_half_open() {
        let repo = InMemoryStatsHistory::new();
        repo.append(&[row("npm", 10, 1), row("npm", 11, 2), row("npm", 12, 3)])
            .await
            .unwrap();

        let rows = repo.read_window(at(11), at(12)).await.unwrap();
        assert_eq!(rows.len(), 1, "[from, to) excludes the upper bound");
        assert_eq!(rows[0].hits, 2);
    }

    #[tokio::test]
    async fn append_upserts_on_registry_and_window() {
        let repo = InMemoryStatsHistory::new();
        repo.append(&[row("npm", 10, 1)]).await.unwrap();
        repo.append(&[row("npm", 10, 9)]).await.unwrap();

        let rows = repo.read_window(at(0), at(23)).await.unwrap();
        assert_eq!(rows.len(), 1, "same key, replaced not duplicated");
        assert_eq!(rows[0].hits, 9);
    }

    #[tokio::test]
    async fn registries_keep_their_own_rows_in_one_window() {
        let repo = InMemoryStatsHistory::new();
        repo.append(&[row("npm", 10, 1), row("cargo", 10, 2)])
            .await
            .unwrap();
        assert_eq!(repo.read_window(at(0), at(23)).await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn prune_deletes_only_rows_before_the_cutoff() {
        let repo = InMemoryStatsHistory::new();
        repo.append(&[row("npm", 10, 1), row("npm", 11, 2), row("npm", 12, 3)])
            .await
            .unwrap();

        let deleted = repo.prune_before(at(12)).await.unwrap();
        assert_eq!(deleted, 2);
        let rows = repo.read_window(at(0), at(23)).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].window_start, at(12), "the cutoff row itself stays");
    }

    #[tokio::test]
    async fn reads_come_back_oldest_first() {
        let repo = InMemoryStatsHistory::new();
        repo.append(&[row("npm", 12, 3)]).await.unwrap();
        repo.append(&[row("npm", 10, 1)]).await.unwrap();

        let rows = repo.read_window(at(0), at(23)).await.unwrap();
        assert_eq!(rows[0].window_start, at(10));
        assert_eq!(rows[1].window_start, at(12));
    }
}
