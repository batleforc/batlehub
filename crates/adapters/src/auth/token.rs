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
    groups: Vec<String>,
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
            .map(|(value, user_id, role)| (value, TokenRecord { user_id, role, groups: vec![] }))
            .collect();
        Self { tokens }
    }

    /// Add entries that carry explicit group memberships (e.g. for testing OIDC group flows).
    pub fn with_group_entries(
        mut self,
        entries: impl IntoIterator<Item = (String, Option<String>, Role, Vec<String>)>,
    ) -> Self {
        for (value, user_id, role, groups) in entries {
            self.tokens.insert(value, TokenRecord { user_id, role, groups });
        }
        self
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
                groups: record.groups.clone(),
            })),
            None => Ok(None),
        }
    }
}
