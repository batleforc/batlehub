//! Postgres storage for provider signing keys (RFC 0015 §4.2).

use async_trait::async_trait;
use sqlx::{PgPool, Row};

use batlehub_core::{entities::SigningKey, error::CoreError, ports::SigningKeyPort};

use crate::db::DbResultExt;

pub struct PgSigningKeyStore {
    pool: PgPool,
}

impl PgSigningKeyStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn map_key(r: &sqlx::postgres::PgRow) -> SigningKey {
    SigningKey {
        key_id: r.get("key_id"),
        ascii_armor: r.get("ascii_armor"),
        trust_signature: r.get("trust_signature"),
        source: r.get("source"),
        source_url: r.get("source_url"),
    }
}

#[async_trait]
impl SigningKeyPort for PgSigningKeyStore {
    async fn list_signing_keys(
        &self,
        registry: &str,
        namespace: &str,
    ) -> Result<Vec<SigningKey>, CoreError> {
        let rows = sqlx::query(
            "SELECT key_id, ascii_armor, trust_signature, source, source_url \
             FROM provider_signing_keys \
             WHERE registry = $1 AND namespace = $2 \
             ORDER BY id",
        )
        .bind(registry)
        .bind(namespace)
        .fetch_all(&self.pool)
        .await
        .db_err()?;
        Ok(rows.iter().map(map_key).collect())
    }

    async fn set_signing_key(
        &self,
        registry: &str,
        namespace: &str,
        key: SigningKey,
    ) -> Result<(), CoreError> {
        // Upsert on the id: re-registering the same key id replaces its armour,
        // which is what a rotation that keeps its id looks like. Two rows for one
        // id would make the download response's key list depend on read order.
        sqlx::query(
            "INSERT INTO provider_signing_keys \
                 (registry, namespace, key_id, ascii_armor, trust_signature, source, source_url) \
             VALUES ($1, $2, $3, $4, $5, $6, $7) \
             ON CONFLICT (registry, namespace, key_id) \
             DO UPDATE SET ascii_armor = EXCLUDED.ascii_armor, \
                           trust_signature = EXCLUDED.trust_signature, \
                           source = EXCLUDED.source, \
                           source_url = EXCLUDED.source_url, \
                           set_at = NOW()",
        )
        .bind(registry)
        .bind(namespace)
        .bind(&key.key_id)
        .bind(&key.ascii_armor)
        .bind(&key.trust_signature)
        .bind(&key.source)
        .bind(&key.source_url)
        .execute(&self.pool)
        .await
        .db_err()?;
        Ok(())
    }

    async fn delete_signing_key(
        &self,
        registry: &str,
        namespace: &str,
        key_id: &str,
    ) -> Result<(), CoreError> {
        sqlx::query(
            "DELETE FROM provider_signing_keys \
             WHERE registry = $1 AND namespace = $2 AND key_id = $3",
        )
        .bind(registry)
        .bind(namespace)
        .bind(key_id)
        .execute(&self.pool)
        .await
        .db_err()?;
        Ok(())
    }
}
