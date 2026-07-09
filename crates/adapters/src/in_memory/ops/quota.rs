use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::RwLock;

use batlehub_core::{
    error::CoreError,
    ports::{QuotaOutcome, QuotaRepository, QuotaUsage},
};

/// In-memory [`QuotaRepository`].
///
/// Stores quota usage per `(user_id, registry)` pair. All counters are
/// floored at 0 (no negative values).
#[derive(Debug, Default)]
pub struct InMemoryQuotaRepository {
    data: Arc<RwLock<HashMap<(String, String), QuotaUsage>>>,
}

impl InMemoryQuotaRepository {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
}

fn zero_usage(user_id: &str, registry: &str) -> QuotaUsage {
    QuotaUsage {
        user_id: user_id.to_owned(),
        registry: registry.to_owned(),
        bytes_published: 0,
        packages_count: 0,
    }
}

#[async_trait]
impl QuotaRepository for InMemoryQuotaRepository {
    async fn get_usage(&self, user_id: &str, registry: &str) -> Result<QuotaUsage, CoreError> {
        let map = self.data.read().await;
        Ok(map
            .get(&(user_id.to_owned(), registry.to_owned()))
            .cloned()
            .unwrap_or_else(|| zero_usage(user_id, registry)))
    }

    async fn record_publish(
        &self,
        user_id: &str,
        registry: &str,
        bytes: u64,
    ) -> Result<(), CoreError> {
        let mut map = self.data.write().await;
        let entry = map
            .entry((user_id.to_owned(), registry.to_owned()))
            .or_insert_with(|| zero_usage(user_id, registry));
        entry.bytes_published = entry.bytes_published.saturating_add(bytes);
        entry.packages_count = entry.packages_count.saturating_add(1);
        Ok(())
    }

    async fn try_record_publish(
        &self,
        user_id: &str,
        registry: &str,
        bytes: u64,
        max_bytes: Option<u64>,
        max_packages: Option<u32>,
    ) -> Result<QuotaOutcome, CoreError> {
        // Hold the write guard across the whole check-then-write so a
        // concurrent call for the same key can't observe (or clobber) an
        // in-between state.
        let mut map = self.data.write().await;
        let entry = map
            .entry((user_id.to_owned(), registry.to_owned()))
            .or_insert_with(|| zero_usage(user_id, registry));

        let new_bytes = entry.bytes_published.saturating_add(bytes);
        let new_packages = entry.packages_count.saturating_add(1);

        let exceeded = max_bytes.is_some_and(|max| new_bytes > max)
            || max_packages.is_some_and(|max| new_packages > max);

        if exceeded {
            return Ok(QuotaOutcome::Exceeded {
                bytes_used: new_bytes,
                packages_used: new_packages,
            });
        }

        entry.bytes_published = new_bytes;
        entry.packages_count = new_packages;

        Ok(QuotaOutcome::Recorded {
            bytes_used: new_bytes,
            packages_used: new_packages,
        })
    }

    async fn revoke_publish(
        &self,
        user_id: &str,
        registry: &str,
        bytes: u64,
    ) -> Result<(), CoreError> {
        let mut map = self.data.write().await;
        let entry = map
            .entry((user_id.to_owned(), registry.to_owned()))
            .or_insert_with(|| zero_usage(user_id, registry));
        entry.bytes_published = entry.bytes_published.saturating_sub(bytes);
        entry.packages_count = entry.packages_count.saturating_sub(1);
        Ok(())
    }

    async fn reset_usage(&self, user_id: &str, registry: &str) -> Result<(), CoreError> {
        let mut map = self.data.write().await;
        map.insert(
            (user_id.to_owned(), registry.to_owned()),
            zero_usage(user_id, registry),
        );
        Ok(())
    }

    async fn list_usage(&self, registry: Option<&str>) -> Result<Vec<QuotaUsage>, CoreError> {
        let map = self.data.read().await;
        let mut result: Vec<QuotaUsage> = map
            .values()
            .filter(|u| registry.is_none_or(|r| u.registry == r))
            .cloned()
            .collect();
        result.sort_by_key(|b| std::cmp::Reverse(b.bytes_published));
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use batlehub_core::ports::{QuotaOutcome, QuotaRepository};

    use super::InMemoryQuotaRepository;

    #[tokio::test]
    async fn try_record_publish_records_when_under_limit() {
        let repo = InMemoryQuotaRepository::new();
        let outcome = repo
            .try_record_publish("alice", "cargo", 100, Some(1000), Some(10))
            .await
            .unwrap();
        assert_eq!(
            outcome,
            QuotaOutcome::Recorded {
                bytes_used: 100,
                packages_used: 1
            }
        );
        let usage = repo.get_usage("alice", "cargo").await.unwrap();
        assert_eq!(usage.bytes_published, 100);
        assert_eq!(usage.packages_count, 1);
    }

    #[tokio::test]
    async fn try_record_publish_rejects_and_leaves_usage_unchanged_when_over_limit() {
        let repo = InMemoryQuotaRepository::new();
        repo.try_record_publish("alice", "cargo", 900, Some(1000), None)
            .await
            .unwrap();
        let outcome = repo
            .try_record_publish("alice", "cargo", 200, Some(1000), None)
            .await
            .unwrap();
        assert_eq!(
            outcome,
            QuotaOutcome::Exceeded {
                bytes_used: 1100,
                packages_used: 2
            }
        );
        // Usage must be unchanged by the rejected attempt.
        let usage = repo.get_usage("alice", "cargo").await.unwrap();
        assert_eq!(usage.bytes_published, 900);
        assert_eq!(usage.packages_count, 1);
    }

    #[tokio::test]
    async fn try_record_publish_concurrent_calls_never_exceed_limit() {
        let repo = InMemoryQuotaRepository::new();
        let mut tasks = Vec::new();
        for _ in 0..20 {
            let repo = repo.clone();
            tasks.push(tokio::spawn(async move {
                repo.try_record_publish("alice", "cargo", 100, Some(1000), None)
                    .await
                    .unwrap()
            }));
        }
        let mut recorded = 0;
        for t in tasks {
            if matches!(t.await.unwrap(), QuotaOutcome::Recorded { .. }) {
                recorded += 1;
            }
        }
        // At most 10 of the 20 concurrent 100-byte publishes can fit under a
        // 1000-byte cap; the race must never let more than that through.
        assert!(
            recorded <= 10,
            "recorded {recorded} publishes, expected <= 10"
        );
        let usage = repo.get_usage("alice", "cargo").await.unwrap();
        assert!(usage.bytes_published <= 1000);
    }

    #[tokio::test]
    async fn get_usage_returns_zero_for_new_user() {
        let repo = InMemoryQuotaRepository::new();
        let usage = repo.get_usage("alice", "cargo").await.unwrap();
        assert_eq!(usage.bytes_published, 0);
        assert_eq!(usage.packages_count, 0);
    }

    #[tokio::test]
    async fn record_publish_accumulates() {
        let repo = InMemoryQuotaRepository::new();
        repo.record_publish("alice", "cargo", 1000).await.unwrap();
        repo.record_publish("alice", "cargo", 500).await.unwrap();
        let usage = repo.get_usage("alice", "cargo").await.unwrap();
        assert_eq!(usage.bytes_published, 1500);
        assert_eq!(usage.packages_count, 2);
    }

    #[tokio::test]
    async fn revoke_publish_decrements_floored_at_zero() {
        let repo = InMemoryQuotaRepository::new();
        repo.record_publish("alice", "cargo", 1000).await.unwrap();
        repo.revoke_publish("alice", "cargo", 1500).await.unwrap();
        let usage = repo.get_usage("alice", "cargo").await.unwrap();
        assert_eq!(usage.bytes_published, 0);
        assert_eq!(usage.packages_count, 0);
    }

    #[tokio::test]
    async fn reset_usage_zeroes_counters() {
        let repo = InMemoryQuotaRepository::new();
        repo.record_publish("alice", "cargo", 5000).await.unwrap();
        repo.reset_usage("alice", "cargo").await.unwrap();
        let usage = repo.get_usage("alice", "cargo").await.unwrap();
        assert_eq!(usage.bytes_published, 0);
        assert_eq!(usage.packages_count, 0);
    }

    #[tokio::test]
    async fn list_usage_filters_by_registry() {
        let repo = InMemoryQuotaRepository::new();
        repo.record_publish("alice", "cargo", 100).await.unwrap();
        repo.record_publish("alice", "npm", 200).await.unwrap();
        repo.record_publish("bob", "cargo", 300).await.unwrap();

        let cargo = repo.list_usage(Some("cargo")).await.unwrap();
        assert_eq!(cargo.len(), 2);
        assert!(cargo.iter().all(|u| u.registry == "cargo"));

        let all = repo.list_usage(None).await.unwrap();
        assert_eq!(all.len(), 3);
    }

    #[tokio::test]
    async fn list_usage_sorted_by_bytes_desc() {
        let repo = InMemoryQuotaRepository::new();
        repo.record_publish("alice", "cargo", 100).await.unwrap();
        repo.record_publish("bob", "cargo", 500).await.unwrap();
        repo.record_publish("carol", "cargo", 300).await.unwrap();

        let list = repo.list_usage(None).await.unwrap();
        assert_eq!(list[0].user_id, "bob");
        assert_eq!(list[1].user_id, "carol");
        assert_eq!(list[2].user_id, "alice");
    }
}
