//! In-memory provider signing keys, for tests and single-node runs.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::RwLock;

use batlehub_core::{entities::SigningKey, error::CoreError, ports::SigningKeyPort};

#[derive(Default)]
pub struct InMemorySigningKeyStore {
    /// `(registry, namespace) -> keys`, insertion-ordered like the SQL `ORDER BY id`.
    inner: RwLock<HashMap<(String, String), Vec<SigningKey>>>,
}

impl InMemorySigningKeyStore {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
}

#[async_trait]
impl SigningKeyPort for InMemorySigningKeyStore {
    async fn list_signing_keys(
        &self,
        registry: &str,
        namespace: &str,
    ) -> Result<Vec<SigningKey>, CoreError> {
        Ok(self
            .inner
            .read()
            .await
            .get(&(registry.to_owned(), namespace.to_owned()))
            .cloned()
            .unwrap_or_default())
    }

    async fn set_signing_key(
        &self,
        registry: &str,
        namespace: &str,
        key: SigningKey,
    ) -> Result<(), CoreError> {
        let mut map = self.inner.write().await;
        let keys = map
            .entry((registry.to_owned(), namespace.to_owned()))
            .or_default();
        // Replace in place rather than append, so this agrees with the Postgres
        // upsert about both the key set *and* its order. `pg_signing_keys.rs`
        // runs one body of assertions against both stores for the reason §13.5
        // gives: agreement between an adapter and its double is not evidence
        // unless the double was written to be wrong in the same ways.
        match keys.iter_mut().find(|k| k.key_id == key.key_id) {
            Some(existing) => *existing = key,
            None => keys.push(key),
        }
        Ok(())
    }

    async fn delete_signing_key(
        &self,
        registry: &str,
        namespace: &str,
        key_id: &str,
    ) -> Result<(), CoreError> {
        if let Some(keys) = self
            .inner
            .write()
            .await
            .get_mut(&(registry.to_owned(), namespace.to_owned()))
        {
            keys.retain(|k| k.key_id != key_id);
        }
        Ok(())
    }
}
