use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::BatleHubClient;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeResponse {
    pub user_id: Option<String>,
    pub role: String,
    pub auth_provider: Option<String>,
    pub groups: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenListItem {
    pub id: Uuid,
    pub name: String,
    pub role: String,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateTokenResponse {
    pub id: Uuid,
    pub name: String,
    pub token: String,
    pub role: String,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct CreateTokenRequest {
    pub name: String,
    pub expires_in_days: u64,
    pub role: String,
}

impl BatleHubClient {
    pub async fn whoami(&self) -> Result<MeResponse> {
        self.get("/api/v1/me").await
    }

    pub async fn list_tokens(&self) -> Result<Vec<TokenListItem>> {
        self.get("/api/v1/auth/tokens").await
    }

    pub async fn create_token(&self, req: CreateTokenRequest) -> Result<CreateTokenResponse> {
        self.post("/api/v1/auth/tokens", &req).await
    }

    pub async fn revoke_token(&self, id: Uuid) -> Result<()> {
        self.delete(&format!("/api/v1/auth/tokens/{id}")).await
    }
}
