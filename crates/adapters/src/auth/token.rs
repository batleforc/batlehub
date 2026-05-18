use std::collections::HashMap;

use async_trait::async_trait;
use base64::Engine as _;

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
            .map(|(value, user_id, role)| {
                (
                    value,
                    TokenRecord {
                        user_id,
                        role,
                        groups: vec![],
                    },
                )
            })
            .collect();
        Self { tokens }
    }

    /// Add entries that carry explicit group memberships (e.g. for testing OIDC group flows).
    pub fn with_group_entries(
        mut self,
        entries: impl IntoIterator<Item = (String, Option<String>, Role, Vec<String>)>,
    ) -> Self {
        for (value, user_id, role, groups) in entries {
            self.tokens.insert(
                value,
                TokenRecord {
                    user_id,
                    role,
                    groups,
                },
            );
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

        let token = if let Some(t) = value
            .strip_prefix("Bearer ")
            .or_else(|| value.strip_prefix("bearer "))
        {
            t.to_owned()
        } else if let Some(encoded) = value
            .strip_prefix("Basic ")
            .or_else(|| value.strip_prefix("basic "))
        {
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(encoded.trim())
                .ok()
                .and_then(|b| String::from_utf8(b).ok());
            let Some(decoded) = decoded else {
                return Ok(None);
            };
            // user:token — split at first `:` only, token may contain `:`
            decoded
                .split_once(':')
                .map(|x| x.1)
                .unwrap_or("")
                .to_owned()
        } else {
            return Ok(None);
        };

        match self.tokens.get(token.as_str()) {
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
