use std::collections::HashMap;

use async_trait::async_trait;
use base64::Engine as _;

use batlehub_core::{
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use batlehub_core::{entities::Role, ports::RawAuthRequest};
    use super::*;

    fn req(auth: &str) -> RawAuthRequest {
        RawAuthRequest {
            headers: HashMap::from([("authorization".to_owned(), auth.to_owned())]),
            query_params: HashMap::new(),
        }
    }

    fn provider() -> StaticTokenAuthProvider {
        StaticTokenAuthProvider::new([
            ("secret".to_owned(), Some("alice".to_owned()), Role::Admin),
        ])
    }

    #[tokio::test]
    async fn no_auth_header_returns_none() {
        let p = provider();
        let r = RawAuthRequest { headers: HashMap::new(), query_params: HashMap::new() };
        assert!(p.authenticate(&r).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn lowercase_bearer_prefix_works() {
        let p = provider();
        let id = p.authenticate(&req("bearer secret")).await.unwrap().unwrap();
        assert_eq!(id.role, Role::Admin);
    }

    #[tokio::test]
    async fn unknown_token_returns_none() {
        let p = provider();
        assert!(p.authenticate(&req("Bearer wrong")).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn basic_auth_extracts_token_from_password_field() {
        // base64("user:secret") = "dXNlcjpzZWNyZXQ="
        let p = provider();
        let id = p.authenticate(&req("Basic dXNlcjpzZWNyZXQ=")).await.unwrap().unwrap();
        assert_eq!(id.user_id.as_deref(), Some("alice"));
    }

    #[tokio::test]
    async fn basic_auth_invalid_base64_returns_none() {
        let p = provider();
        assert!(p.authenticate(&req("Basic !!!not-base64!!!")).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn basic_auth_no_colon_in_decoded_returns_none() {
        // base64("secretonly") — no colon → token becomes "" → not found
        use base64::Engine as _;
        let encoded = base64::engine::general_purpose::STANDARD.encode("secretonly");
        let p = provider();
        assert!(p.authenticate(&req(&format!("Basic {encoded}"))).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn with_group_entries_populates_groups() {
        let p = StaticTokenAuthProvider::new([])
            .with_group_entries([("tok".to_owned(), Some("bob".to_owned()), Role::User, vec!["team-a".to_owned()])]);
        let id = p.authenticate(&req("Bearer tok")).await.unwrap().unwrap();
        assert_eq!(id.groups, vec!["team-a"]);
        assert_eq!(id.role, Role::User);
    }

    #[tokio::test]
    async fn unrecognised_scheme_returns_none() {
        let p = provider();
        assert!(p.authenticate(&req("Digest something")).await.unwrap().is_none());
    }
}
