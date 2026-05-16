use std::collections::HashMap;

use async_trait::async_trait;

use proxy_cache_core::{
    entities::{Identity, Role},
    error::CoreError,
    ports::{AuthProvider, RawAuthRequest},
};

struct TokenRecord {
    user_id: Option<String>,
    role: Role,
}

/// Authenticates requests via static Bearer tokens configured in `config.toml`.
///
/// Checks the `Authorization: Bearer <token>` header.
pub struct StaticTokenAuthProvider {
    tokens: HashMap<String, TokenRecord>,
}

impl StaticTokenAuthProvider {
    pub fn new(entries: impl IntoIterator<Item = (String, Option<String>, Role)>) -> Self {
        let tokens = entries
            .into_iter()
            .map(|(value, user_id, role)| (value, TokenRecord { user_id, role }))
            .collect();
        Self { tokens }
    }
}

#[async_trait]
impl AuthProvider for StaticTokenAuthProvider {
    fn name(&self) -> &str {
        "static-token"
    }

    async fn authenticate(&self, req: &RawAuthRequest) -> Result<Option<Identity>, CoreError> {
        let auth_header = req
            .headers
            .get("authorization")
            .or_else(|| req.headers.get("Authorization"));

        let Some(value) = auth_header else {
            return Ok(None);
        };

        let token = value
            .strip_prefix("Bearer ")
            .or_else(|| value.strip_prefix("bearer "));

        let Some(token) = token else {
            return Ok(None);
        };

        match self.tokens.get(token) {
            Some(record) => Ok(Some(Identity {
                user_id: record.user_id.clone(),
                role: record.role.clone(),
                auth_provider: Some("static-token".to_owned()),
            })),
            None => Ok(None),
        }
    }
}
