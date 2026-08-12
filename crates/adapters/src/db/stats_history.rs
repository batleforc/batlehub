use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row};

use crate::db::DbResultExt;
use batlehub_core::{
    error::CoreError,
    ports::{StatsHistoryRepository, StatsRollupRow},
};

pub struct PgStatsHistoryRepository {
    pool: PgPool,
}

impl PgStatsHistoryRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn row_to_rollup(r: &sqlx::postgres::PgRow) -> StatsRollupRow {
    // Stored as BIGINT because Postgres has no unsigned integer type. The
    // writer only ever inserts values that came from a `u64` counter delta, so
    // a negative here would mean the table was edited by hand; clamp rather
    // than fail a dashboard read over it.
    let as_u64 = |v: i64| u64::try_from(v).unwrap_or(0);
    StatsRollupRow {
        registry: r.get("registry"),
        window_start: r.get("window_start"),
        hits: as_u64(r.get::<i64, _>("hits")),
        misses: as_u64(r.get::<i64, _>("misses")),
        cached_bytes: as_u64(r.get::<i64, _>("cached_bytes")),
    }
}

#[async_trait]
impl StatsHistoryRepository for PgStatsHistoryRepository {
    async fn append(&self, rows: &[StatsRollupRow]) -> Result<(), CoreError> {
        if rows.is_empty() {
            return Ok(());
        }

        // One statement for the whole window via `unnest`, matched positionally.
        // `ON CONFLICT … DO UPDATE` makes a second tick inside the same hour
        // idempotent rather than a double count — which is the difference
        // between a restart costing one partial window and a restart inventing
        // traffic.
        let registries: Vec<String> = rows.iter().map(|r| r.registry.clone()).collect();
        let windows: Vec<DateTime<Utc>> = rows.iter().map(|r| r.window_start).collect();
        let hits: Vec<i64> = rows.iter().map(|r| r.hits as i64).collect();
        let misses: Vec<i64> = rows.iter().map(|r| r.misses as i64).collect();
        let cached: Vec<i64> = rows.iter().map(|r| r.cached_bytes as i64).collect();

        sqlx::query(
            "INSERT INTO stats_history (registry, window_start, hits, misses, cached_bytes) \
             SELECT * FROM unnest($1::text[], $2::timestamptz[], $3::bigint[], $4::bigint[], $5::bigint[]) \
             ON CONFLICT (registry, window_start) DO UPDATE SET \
               hits = EXCLUDED.hits, \
               misses = EXCLUDED.misses, \
               cached_bytes = EXCLUDED.cached_bytes",
        )
        .bind(&registries)
        .bind(&windows)
        .bind(&hits)
        .bind(&misses)
        .bind(&cached)
        .execute(&self.pool)
        .await
        .db_err()?;
        Ok(())
    }

    async fn read_window(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<StatsRollupRow>, CoreError> {
        let rows = sqlx::query(
            "SELECT registry, window_start, hits, misses, cached_bytes \
             FROM stats_history \
             WHERE window_start >= $1 AND window_start < $2 \
             ORDER BY window_start ASC, registry ASC",
        )
        .bind(from)
        .bind(to)
        .fetch_all(&self.pool)
        .await
        .db_err()?;

        Ok(rows.iter().map(row_to_rollup).collect())
    }

    async fn prune_before(&self, cutoff: DateTime<Utc>) -> Result<u64, CoreError> {
        let result = sqlx::query("DELETE FROM stats_history WHERE window_start < $1")
            .bind(cutoff)
            .execute(&self.pool)
            .await
            .db_err()?;
        Ok(result.rows_affected())
    }
}
