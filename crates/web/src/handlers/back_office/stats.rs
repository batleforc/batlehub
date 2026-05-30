use std::sync::Arc;

use actix_web::{get, web, Responder};
use chrono::{DateTime, Utc};
use serde::Serialize;
use utoipa::ToSchema;

use batlehub_core::services::{ProxyMetrics, ProxyService};

use super::require_admin;
use crate::{error::AppError, extractors::AuthIdentity, RegistryMap};

fn hit_rate(hits: u64, misses: u64) -> Option<f64> {
    let total = hits + misses;
    (total > 0).then(|| hits as f64 / total as f64)
}

#[derive(Serialize, ToSchema)]
pub struct RegistryStatsDto {
    pub registry: String,
    pub artifact_hits: u64,
    pub artifact_misses: u64,
    /// Artifact hit rate in [0, 1], or null if no requests yet.
    pub hit_rate: Option<f64>,
    /// Total bytes cached in storage for this registry (from storage backend).
    pub cached_bytes: Option<u64>,
}

#[derive(Serialize, ToSchema)]
pub struct AggregateStats {
    pub artifact_hits: u64,
    pub artifact_misses: u64,
    /// Aggregate artifact hit rate in [0, 1], or null if no requests yet.
    pub hit_rate: Option<f64>,
    /// Total bytes cached across all registries.
    pub cached_bytes: u64,
}

#[derive(Serialize, ToSchema)]
pub struct StatsResponse {
    /// When this process started (counters reset on restart).
    pub since_startup: DateTime<Utc>,
    pub aggregate: AggregateStats,
    pub per_registry: Vec<RegistryStatsDto>,
}

/// Aggregate cache hit/miss statistics since last restart (admin).
#[utoipa::path(
    get,
    path = "/api/v1/admin/stats",
    tag = "back-office",
    responses(
        (status = 200, description = "Cache statistics", body = StatsResponse),
        (status = 403, description = "Admin role required"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/api/v1/admin/stats")]
pub async fn admin_stats(
    identity: AuthIdentity,
    registry_map: web::Data<RegistryMap>,
    proxy_svc: web::Data<Arc<ProxyService>>,
    proxy_metrics: web::Data<Arc<ProxyMetrics>>,
) -> Result<impl Responder, AppError> {
    require_admin(&identity)?;

    let mut per_registry: Vec<RegistryStatsDto> = Vec::new();
    let mut total_hits: u64 = 0;
    let mut total_misses: u64 = 0;
    let mut total_cached_bytes: u64 = 0;

    let mut registries: Vec<String> = registry_map.0.keys().cloned().collect();
    registries.sort();

    for registry in registries {
        let (hits, misses) = if let Some(c) = proxy_metrics.all().get(&registry) {
            (c.hits(), c.misses())
        } else {
            (0, 0)
        };

        let prefix = format!("artifact:{}/", registry);
        let cached_bytes: Option<u64> = match proxy_svc.storage.stat_by_prefix(&prefix).await {
            Ok((_, bytes)) => Some(bytes),
            Err(_) => None,
        };

        total_hits += hits;
        total_misses += misses;
        total_cached_bytes += cached_bytes.unwrap_or(0);

        per_registry.push(RegistryStatsDto {
            registry,
            artifact_hits: hits,
            artifact_misses: misses,
            hit_rate: hit_rate(hits, misses),
            cached_bytes,
        });
    }

    let aggregate_hit_rate = hit_rate(total_hits, total_misses);

    Ok(web::Json(StatsResponse {
        since_startup: proxy_metrics.started_at,
        aggregate: AggregateStats {
            artifact_hits: total_hits,
            artifact_misses: total_misses,
            hit_rate: aggregate_hit_rate,
            cached_bytes: total_cached_bytes,
        },
        per_registry,
    }))
}
