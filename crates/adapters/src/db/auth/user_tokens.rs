use crate::db::DbResultExt;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::Row;
use uuid::Uuid;

use batlehub_core::{
    entities::Role,
    error::CoreError,
    ports::{UserToken, UserTokenRepository},
};

use crate::db::packages::PgPackageRepository;

fn role_to_str(role: &Role) -> &'static str {
    match role {
        Role::Anonymous => "anonymous",
        Role::User => "user",
        Role::Admin => "admin",
    }
}

#[async_trait]
impl UserTokenRepository for PgPackageRepository {
    async fn create_token(
        &self,
        id: Uuid,
        user_id: &str,
        name: &str,
        token_hash: &str,
        role: Role,
        expires_at: DateTime<Utc>,
    ) -> Result<UserToken, CoreError> {
        let row = sqlx::query(
            r#"
            INSERT INTO user_tokens (id, user_id, name, token_hash, role, expires_at, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, NOW())
            RETURNING id, user_id, name, role, expires_at, created_at, revoked_at
            "#,
        )
        .bind(id)
        .bind(user_id)
        .bind(name)
        .bind(token_hash)
        .bind(role_to_str(&role))
        .bind(expires_at)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| {
            if let sqlx::Error::Database(ref db_err) = e {
                if db_err.constraint() == Some("uq_user_token_name") {
                    return CoreError::Conflict(format!("a token named '{}' already exists", name));
                }
            }
            CoreError::Database(e.to_string())
        })?;

        Ok(UserToken {
            id: row.get("id"),
            user_id: row.get("user_id"),
            name: row.get("name"),
            role: row
                .get::<&str, _>("role")
                .parse()
                .map_err(|e| CoreError::Database(format!("invalid role in db: {e}")))?,
            expires_at: row.get("expires_at"),
            created_at: row.get("created_at"),
            revoked_at: row.get("revoked_at"),
        })
    }

    async fn find_by_hash(&self, token_hash: &str) -> Result<Option<UserToken>, CoreError> {
        let row = sqlx::query(
            r#"
            SELECT id, user_id, name, role, expires_at, created_at, revoked_at
            FROM user_tokens
            WHERE token_hash = $1 AND revoked_at IS NULL AND expires_at > NOW()
            "#,
        )
        .bind(token_hash)
        .fetch_optional(&self.pool)
        .await
        .db_err()?;

        row.map(|r| {
            Ok(UserToken {
                id: r.get("id"),
                user_id: r.get("user_id"),
                name: r.get("name"),
                role: r
                    .get::<&str, _>("role")
                    .parse()
                    .map_err(|e| CoreError::Database(format!("invalid role in db: {e}")))?,
                expires_at: r.get("expires_at"),
                created_at: r.get("created_at"),
                revoked_at: r.get("revoked_at"),
            })
        })
        .transpose()
    }

    async fn list_for_user(&self, user_id: &str) -> Result<Vec<UserToken>, CoreError> {
        let rows = sqlx::query(
            r#"
            SELECT id, user_id, name, role, expires_at, created_at, revoked_at
            FROM user_tokens
            WHERE user_id = $1 AND revoked_at IS NULL AND expires_at > NOW()
            ORDER BY created_at DESC
            "#,
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .db_err()?;

        rows.into_iter()
            .map(|r| {
                Ok(UserToken {
                    id: r.get("id"),
                    user_id: r.get("user_id"),
                    name: r.get("name"),
                    role: r
                        .get::<&str, _>("role")
                        .parse()
                        .map_err(|e| CoreError::Database(format!("invalid role in db: {e}")))?,
                    expires_at: r.get("expires_at"),
                    created_at: r.get("created_at"),
                    revoked_at: r.get("revoked_at"),
                })
            })
            .collect()
    }

    async fn revoke(&self, id: Uuid, user_id: &str) -> Result<bool, CoreError> {
        let result = sqlx::query(
            r#"
            UPDATE user_tokens
            SET revoked_at = NOW()
            WHERE id = $1 AND user_id = $2 AND revoked_at IS NULL
            RETURNING id
            "#,
        )
        .bind(id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
        .db_err()?;

        Ok(result.is_some())
    }
}
