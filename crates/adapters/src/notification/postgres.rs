use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use batlehub_core::{
    entities::{InboundWebhookEvent, NotificationEventType, NotificationSubscription},
    error::CoreError,
    ports::NotificationPort,
};

use crate::db::DbResultExt;

pub struct PgNotificationStore {
    pool: PgPool,
}

impl PgNotificationStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn parse_event_types(raw: Vec<String>) -> Vec<NotificationEventType> {
    raw.into_iter().filter_map(|s| s.parse().ok()).collect()
}

fn row_to_subscription(r: &sqlx::postgres::PgRow) -> NotificationSubscription {
    let raw_types: Vec<String> = r.get("event_types");
    NotificationSubscription {
        id: r.get("id"),
        registry: r.get("registry"),
        package_name: r.get("package_name"),
        event_types: parse_event_types(raw_types),
        channel_name: r.get("channel_name"),
        created_by: r.get("created_by"),
        created_at: r.get::<DateTime<Utc>, _>("created_at"),
        enabled: r.get("enabled"),
    }
}

#[async_trait]
impl NotificationPort for PgNotificationStore {
    async fn add_subscription(&self, sub: NotificationSubscription) -> Result<(), CoreError> {
        let event_types: Vec<String> = sub
            .event_types
            .iter()
            .map(|e| e.as_str().to_owned())
            .collect();
        sqlx::query(
            "INSERT INTO notification_subscriptions \
             (id, registry, package_name, event_types, channel_name, created_by, created_at, enabled) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(sub.id)
        .bind(&sub.registry)
        .bind(&sub.package_name)
        .bind(&event_types)
        .bind(&sub.channel_name)
        .bind(&sub.created_by)
        .bind(sub.created_at)
        .bind(sub.enabled)
        .execute(&self.pool)
        .await
        .db_err()?;
        Ok(())
    }

    async fn list_subscriptions(&self) -> Result<Vec<NotificationSubscription>, CoreError> {
        let rows = sqlx::query(
            "SELECT id, registry, package_name, event_types, channel_name, \
                    created_by, created_at, enabled \
             FROM notification_subscriptions \
             ORDER BY created_at ASC",
        )
        .fetch_all(&self.pool)
        .await
        .db_err()?;
        Ok(rows.iter().map(row_to_subscription).collect())
    }

    async fn get_subscription(
        &self,
        id: Uuid,
    ) -> Result<Option<NotificationSubscription>, CoreError> {
        let row = sqlx::query(
            "SELECT id, registry, package_name, event_types, channel_name, \
                    created_by, created_at, enabled \
             FROM notification_subscriptions \
             WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .db_err()?;
        Ok(row.as_ref().map(row_to_subscription))
    }

    async fn update_subscription(&self, sub: NotificationSubscription) -> Result<(), CoreError> {
        let event_types: Vec<String> = sub
            .event_types
            .iter()
            .map(|e| e.as_str().to_owned())
            .collect();
        let result = sqlx::query(
            "UPDATE notification_subscriptions \
             SET registry = $2, package_name = $3, event_types = $4, \
                 channel_name = $5, enabled = $6 \
             WHERE id = $1",
        )
        .bind(sub.id)
        .bind(&sub.registry)
        .bind(&sub.package_name)
        .bind(&event_types)
        .bind(&sub.channel_name)
        .bind(sub.enabled)
        .execute(&self.pool)
        .await
        .db_err()?;
        if result.rows_affected() == 0 {
            return Err(CoreError::NotFound(format!("subscription {}", sub.id)));
        }
        Ok(())
    }

    async fn remove_subscription(&self, id: Uuid) -> Result<(), CoreError> {
        sqlx::query("DELETE FROM notification_subscriptions WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .db_err()?;
        Ok(())
    }

    async fn get_matching_subscriptions(
        &self,
        registry: &str,
        package: &str,
        event_type: &NotificationEventType,
    ) -> Result<Vec<NotificationSubscription>, CoreError> {
        let event_str = event_type.as_str();
        let rows = sqlx::query(
            "SELECT id, registry, package_name, event_types, channel_name, \
                    created_by, created_at, enabled \
             FROM notification_subscriptions \
             WHERE enabled = TRUE \
               AND (registry IS NULL OR registry = $1) \
               AND (package_name IS NULL OR package_name = $2) \
               AND event_types && ARRAY[$3::TEXT]",
        )
        .bind(registry)
        .bind(package)
        .bind(event_str)
        .fetch_all(&self.pool)
        .await
        .db_err()?;
        Ok(rows.iter().map(row_to_subscription).collect())
    }

    async fn record_inbound_event(&self, event: InboundWebhookEvent) -> Result<(), CoreError> {
        sqlx::query(
            "INSERT INTO inbound_webhook_events \
             (id, webhook_name, payload, source_ip, received_at, signature_valid) \
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(event.id)
        .bind(&event.webhook_name)
        .bind(&event.payload)
        .bind(&event.source_ip)
        .bind(event.received_at)
        .bind(event.signature_valid)
        .execute(&self.pool)
        .await
        .db_err()?;
        Ok(())
    }

    async fn list_inbound_events(&self, limit: i64) -> Result<Vec<InboundWebhookEvent>, CoreError> {
        let limit = if limit <= 0 { 100i64 } else { limit };
        let rows = sqlx::query(
            "SELECT id, webhook_name, payload, source_ip, received_at, signature_valid \
             FROM inbound_webhook_events \
             ORDER BY received_at DESC \
             LIMIT $1",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .db_err()?;
        Ok(rows
            .iter()
            .map(|r| InboundWebhookEvent {
                id: r.get("id"),
                webhook_name: r.get("webhook_name"),
                payload: r.get("payload"),
                source_ip: r.get("source_ip"),
                received_at: r.get::<DateTime<Utc>, _>("received_at"),
                signature_valid: r.get("signature_valid"),
            })
            .collect())
    }
}
