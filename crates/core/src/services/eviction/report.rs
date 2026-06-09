/// Summary of a completed eviction run.
#[derive(Debug, Default, Clone)]
pub struct EvictionReport {
    pub total: usize,
    pub evicted_ttl: usize,
    pub evicted_idle: usize,
    pub evicted_old_versions: usize,
    pub evicted_lru: usize,
}

/// Summary of a coherence check run.
#[derive(Debug, Clone)]
pub struct CoherenceReport {
    pub storage_keys: usize,
    pub meta_rows: usize,
    pub orphaned_deleted: usize,
}
