use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{DateTime, Utc};

pub struct RegistryCounters {
    artifact_hits: AtomicU64,
    artifact_misses: AtomicU64,
}

impl RegistryCounters {
    fn new() -> Self {
        Self {
            artifact_hits: AtomicU64::new(0),
            artifact_misses: AtomicU64::new(0),
        }
    }

    pub fn hits(&self) -> u64 {
        self.artifact_hits.load(Ordering::Relaxed)
    }

    pub fn misses(&self) -> u64 {
        self.artifact_misses.load(Ordering::Relaxed)
    }
}

/// In-memory per-registry counters for the stats dashboard.
/// Reset on process restart; Prometheus metrics are the durable store.
pub struct ProxyMetrics {
    pub started_at: DateTime<Utc>,
    registries: HashMap<String, RegistryCounters>,
}

impl ProxyMetrics {
    pub fn new(registry_names: &[String]) -> Self {
        let registries = registry_names
            .iter()
            .map(|name| (name.clone(), RegistryCounters::new()))
            .collect();
        Self {
            started_at: Utc::now(),
            registries,
        }
    }

    pub fn record_artifact_hit(&self, registry: &str) {
        if let Some(c) = self.registries.get(registry) {
            c.artifact_hits.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn record_artifact_miss(&self, registry: &str) {
        if let Some(c) = self.registries.get(registry) {
            c.artifact_misses.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn all(&self) -> &HashMap<String, RegistryCounters> {
        &self.registries
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(s: &[&str]) -> Vec<String> {
        s.iter().map(|&n| n.to_owned()).collect()
    }

    #[test]
    fn new_registries_start_at_zero() {
        let m = ProxyMetrics::new(&names(&["npm", "cargo"]));
        let npm = m.all().get("npm").unwrap();
        assert_eq!(npm.hits(), 0);
        assert_eq!(npm.misses(), 0);
        let cargo = m.all().get("cargo").unwrap();
        assert_eq!(cargo.hits(), 0);
        assert_eq!(cargo.misses(), 0);
    }

    #[test]
    fn record_hit_increments_only_hits() {
        let m = ProxyMetrics::new(&names(&["npm"]));
        m.record_artifact_hit("npm");
        m.record_artifact_hit("npm");
        let c = m.all().get("npm").unwrap();
        assert_eq!(c.hits(), 2);
        assert_eq!(c.misses(), 0);
    }

    #[test]
    fn record_miss_increments_only_misses() {
        let m = ProxyMetrics::new(&names(&["npm"]));
        m.record_artifact_miss("npm");
        let c = m.all().get("npm").unwrap();
        assert_eq!(c.hits(), 0);
        assert_eq!(c.misses(), 1);
    }

    #[test]
    fn registries_tracked_independently() {
        let m = ProxyMetrics::new(&names(&["npm", "cargo"]));
        m.record_artifact_hit("npm");
        m.record_artifact_miss("cargo");
        m.record_artifact_miss("cargo");
        assert_eq!(m.all().get("npm").unwrap().hits(), 1);
        assert_eq!(m.all().get("npm").unwrap().misses(), 0);
        assert_eq!(m.all().get("cargo").unwrap().hits(), 0);
        assert_eq!(m.all().get("cargo").unwrap().misses(), 2);
    }

    #[test]
    fn unknown_registry_is_silently_ignored() {
        let m = ProxyMetrics::new(&names(&["npm"]));
        m.record_artifact_hit("unknown"); // should not panic
        m.record_artifact_miss("unknown");
        assert!(m.all().get("unknown").is_none());
    }
}
