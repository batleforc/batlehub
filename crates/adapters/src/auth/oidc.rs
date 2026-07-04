use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use jsonwebtoken::jwk::JwkSet;
use jsonwebtoken::{decode, decode_header, DecodingKey, Validation};
use serde::Deserialize;
use tokio::sync::RwLock;

use batlehub_core::ports::OidcAuthConfig;
use batlehub_core::{
    entities::{Identity, Role},
    error::CoreError,
    ports::{AuthProvider, RawAuthRequest},
};

const JWKS_MIN_REFRESH: Duration = Duration::from_secs(300);

#[derive(Deserialize)]
struct OidcDiscovery {
    issuer: String,
    jwks_uri: String,
    authorization_endpoint: String,
    token_endpoint: String,
}

// ── SSO flow (Authorization Code) ────────────────────────────────────────────

/// Tokens returned by the OIDC provider after a successful code exchange or refresh.
#[derive(Debug, Clone, serde::Serialize)]
pub struct OidcTokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
    /// Lifetime of the access token in seconds as reported by the provider.
    pub expires_in: Option<u64>,
}

/// Holds everything the web layer needs to initiate and complete the browser-based
/// OIDC Authorization Code flow.  Cloneable so it can be stored in `web::Data`.
#[derive(Clone)]
pub struct OidcSsoFlow {
    /// Provider name — matches the `name` field in `[[auth]]` config (default: `"oidc"`).
    pub name: String,
    pub client_id: String,
    client_secret: Option<String>,
    pub redirect_uri: String,
    pub scopes: Vec<String>,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    /// Base URL of the SPA — used to build the post-callback redirect.
    pub frontend_url: String,
    http: reqwest::Client,
}

impl OidcSsoFlow {
    /// Build the provider's authorization URL for a given CSRF `state` value.
    pub fn authorization_url(&self, state: &str) -> String {
        let scope = self.scopes.join(" ");
        let params = [
            ("response_type", "code"),
            ("client_id", &self.client_id),
            ("redirect_uri", &self.redirect_uri),
            ("scope", &scope),
            ("state", state),
        ];
        let qs = params
            .iter()
            .map(|(k, v)| format!("{}={}", k, percent_encode(v)))
            .collect::<Vec<_>>()
            .join("&");
        format!("{}?{}", self.authorization_endpoint, qs)
    }

    /// Exchange an authorization `code` for tokens.
    pub async fn exchange_code(&self, code: &str) -> anyhow::Result<OidcTokens> {
        let mut params = vec![
            ("grant_type", "authorization_code"),
            ("code", code),
            ("client_id", &self.client_id),
            ("redirect_uri", &self.redirect_uri),
        ];
        if let Some(ref secret) = self.client_secret {
            params.push(("client_secret", secret.as_str()));
        }
        self.token_request(&params).await
    }

    /// Use a refresh token to obtain a fresh access token (and possibly a new refresh token).
    pub async fn refresh(&self, refresh_token: &str) -> anyhow::Result<OidcTokens> {
        let mut params = vec![
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", &self.client_id),
        ];
        if let Some(ref secret) = self.client_secret {
            params.push(("client_secret", secret.as_str()));
        }
        self.token_request(&params).await
    }

    async fn token_request(&self, params: &[(&str, &str)]) -> anyhow::Result<OidcTokens> {
        let resp: serde_json::Value = self
            .http
            .post(&self.token_endpoint)
            .form(params)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        let access_token = resp["access_token"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| anyhow::anyhow!("token response missing access_token"))?;

        Ok(OidcTokens {
            access_token,
            refresh_token: resp["refresh_token"].as_str().map(str::to_owned),
            expires_in: resp["expires_in"].as_u64(),
        })
    }
}

fn percent_encode(s: &str) -> String {
    s.chars()
        .flat_map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~') {
                vec![c]
            } else {
                c.to_string()
                    .bytes()
                    .flat_map(|b| format!("%{b:02X}").chars().collect::<Vec<_>>())
                    .collect()
            }
        })
        .collect()
}

struct JwksCache {
    keys: JwkSet,
    fetched_at: Instant,
}

pub struct OidcAuthProvider {
    name: String,
    /// Canonical issuer identifier from the OIDC discovery document (`issuer` field).
    /// Used to validate the `iss` claim so that two providers with different issuers
    /// cannot validate each other's tokens.
    issuer: String,
    user_id_claim: String,
    role_claim: String,
    role_mappings: HashMap<String, String>,
    http: reqwest::Client,
    jwks_uri: String,
    cache: Arc<RwLock<JwksCache>>,
    sso: Option<OidcSsoFlow>,
}

impl OidcAuthProvider {
    pub async fn new(cfg: &OidcAuthConfig) -> anyhow::Result<Self> {
        for (claim_value, role) in &cfg.role_mappings {
            role.parse::<Role>()
                .map_err(|e| anyhow::anyhow!("role_mappings.{claim_value}: {e}"))?;
        }

        let http = reqwest::Client::new();

        let discovery_url = format!(
            "{}/.well-known/openid-configuration",
            cfg.issuer_url.trim_end_matches('/')
        );
        let discovery: OidcDiscovery = http
            .get(&discovery_url)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("fetching OIDC discovery document: {e}"))?
            .json()
            .await
            .map_err(|e| anyhow::anyhow!("parsing OIDC discovery document: {e}"))?;

        let keys = fetch_jwks(&http, &discovery.jwks_uri).await.map_err(|e| {
            anyhow::anyhow!("fetching initial JWKS from {}: {e}", discovery.jwks_uri)
        })?;

        let sso = cfg.redirect_uri.as_ref().map(|redirect_uri| OidcSsoFlow {
            name: cfg.name.clone(),
            client_id: cfg.client_id.clone(),
            client_secret: cfg.client_secret.clone(),
            redirect_uri: redirect_uri.clone(),
            scopes: cfg.scopes.clone(),
            authorization_endpoint: discovery.authorization_endpoint.clone(),
            token_endpoint: discovery.token_endpoint.clone(),
            frontend_url: cfg.frontend_url.clone(),
            http: http.clone(),
        });

        Ok(Self {
            name: cfg.name.clone(),
            issuer: discovery.issuer,
            user_id_claim: cfg.user_id_claim.clone(),
            role_claim: cfg.role_claim.clone(),
            role_mappings: cfg.role_mappings.clone(),
            http,
            jwks_uri: discovery.jwks_uri,
            cache: Arc::new(RwLock::new(JwksCache {
                keys,
                fetched_at: Instant::now(),
            })),
            sso,
        })
    }

    /// Returns the SSO flow helper if `redirect_uri` was configured, `None` otherwise.
    pub fn sso_flow(&self) -> Option<&OidcSsoFlow> {
        self.sso.as_ref()
    }

    async fn get_decoding_key(&self, kid: Option<&str>) -> Result<DecodingKey, CoreError> {
        // Try the current cache first.
        {
            let cache = self.cache.read().await;
            if let Some(key) = find_key(&cache.keys, kid) {
                return Ok(key);
            }
            // If the cache was refreshed very recently, don't hammer the JWKS endpoint.
            if cache.fetched_at.elapsed() < JWKS_MIN_REFRESH {
                return Err(CoreError::Auth("unknown JWT signing key".to_owned()));
            }
        }

        // Refresh JWKS and update cache.
        let new_keys = fetch_jwks(&self.http, &self.jwks_uri)
            .await
            .map_err(|e| CoreError::Auth(format!("JWKS refresh failed: {e}")))?;

        let key = find_key(&new_keys, kid)
            .ok_or_else(|| CoreError::Auth("unknown JWT signing key after refresh".to_owned()))?;

        *self.cache.write().await = JwksCache {
            keys: new_keys,
            fetched_at: Instant::now(),
        };

        Ok(key)
    }

    fn map_role(&self, claim_value: &serde_json::Value) -> Role {
        let candidates: Vec<&str> = match claim_value {
            serde_json::Value::String(s) => vec![s.as_str()],
            serde_json::Value::Array(arr) => arr.iter().filter_map(|v| v.as_str()).collect(),
            _ => vec![],
        };

        candidates
            .into_iter()
            .filter_map(|s| self.role_mappings.get(s))
            .filter_map(|mapped| mapped.parse::<Role>().ok())
            .max()
            .unwrap_or(Role::Anonymous)
    }
}

/// Test-only constructor that skips the network bootstrap.
#[cfg(test)]
impl OidcAuthProvider {
    fn for_testing(
        name: impl Into<String>,
        user_id_claim: impl Into<String>,
        role_claim: impl Into<String>,
        role_mappings: HashMap<String, String>,
        jwks: JwkSet,
    ) -> Self {
        Self {
            name: name.into(),
            issuer: String::new(), // no issuer validation in tests
            user_id_claim: user_id_claim.into(),
            role_claim: role_claim.into(),
            role_mappings,
            http: reqwest::Client::new(),
            jwks_uri: String::new(),
            cache: Arc::new(RwLock::new(JwksCache {
                keys: jwks,
                fetched_at: Instant::now(),
            })),
            sso: None,
        }
    }
}

fn find_key(jwks: &JwkSet, kid: Option<&str>) -> Option<DecodingKey> {
    let jwk = if let Some(kid) = kid {
        jwks.find(kid)
    } else {
        jwks.keys.first()
    }?;
    DecodingKey::from_jwk(jwk).ok()
}

async fn fetch_jwks(http: &reqwest::Client, uri: &str) -> Result<JwkSet, reqwest::Error> {
    http.get(uri).send().await?.json().await
}

#[async_trait]
impl AuthProvider for OidcAuthProvider {
    fn name(&self) -> &str {
        &self.name
    }

    async fn authenticate(&self, req: &RawAuthRequest) -> Result<Option<Identity>, CoreError> {
        let auth_header = req
            .headers
            .get("authorization")
            .or_else(|| req.headers.get("Authorization"));

        let Some(value) = auth_header else {
            return Ok(None);
        };

        let Some(token) = value
            .strip_prefix("Bearer ")
            .or_else(|| value.strip_prefix("bearer "))
        else {
            return Ok(None);
        };

        let header = decode_header(token)
            .map_err(|e| CoreError::Auth(format!("invalid JWT header: {e}")))?;

        let decoding_key = self.get_decoding_key(header.kid.as_deref()).await?;

        // Validate the issuer so each provider only accepts tokens from its own issuer.
        // This prevents two providers that share JWKS keys (e.g. same identity server,
        // different client apps) from processing each other's tokens.
        // Audience validation is skipped — it's deployment-specific and not standardised.
        let mut validation = Validation::new(header.alg);
        validation.validate_aud = false;
        if !self.issuer.is_empty() {
            validation.set_issuer(&[&self.issuer]);
        }

        let token_data = match decode::<serde_json::Map<String, serde_json::Value>>(
            token,
            &decoding_key,
            &validation,
        ) {
            Ok(data) => data,
            Err(e) if matches!(e.kind(), jsonwebtoken::errors::ErrorKind::ExpiredSignature) => {
                tracing::debug!(provider = %self.name, "JWT expired");
                return Ok(None);
            }
            Err(e) => return Err(CoreError::Auth(format!("JWT validation failed: {e}"))),
        };

        let claims = token_data.claims;

        let user_id = claims
            .get(&self.user_id_claim)
            .and_then(|v| v.as_str())
            .map(str::to_owned);

        let role_claim_value = claims.get(&self.role_claim);

        let role = role_claim_value
            .map(|v| self.map_role(v))
            .unwrap_or(Role::Anonymous);

        // Extract raw strings from the claim, then namespace-prefix any value that is
        // not explicitly in role_mappings so groups from different providers stay distinct.
        let raw_groups: Vec<String> = role_claim_value
            .map(|v| match v {
                serde_json::Value::String(s) => vec![s.clone()],
                serde_json::Value::Array(arr) => arr
                    .iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect(),
                _ => vec![],
            })
            .unwrap_or_default();

        let groups: Vec<String> = raw_groups
            .into_iter()
            .map(|s| {
                if self.role_mappings.contains_key(&s) {
                    s
                } else {
                    format!("{}:{s}", self.name)
                }
            })
            .collect();

        Ok(Some(Identity {
            user_id,
            role,
            auth_provider: Some(self.name.clone()),
            groups,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
    use serde_json::json;
    use std::collections::HashMap;
    use std::time::{SystemTime, UNIX_EPOCH};

    // ECDSA P-256 test key pair taken from jsonwebtoken's own test fixtures.
    // Private: PKCS#8 PEM; public key encoded as JWK below.
    const TEST_EC_PRIVATE_KEY: &str = "-----BEGIN PRIVATE KEY-----\n\
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgWTFfCGljY6aw3Hrt\n\
kHmPRiazukxPLb6ilpRAewjW8nihRANCAATDskChT+Altkm9X7MI69T3IUmrQU0L\n\
950IxEzvw/x5BMEINRMrXLBJhqzO9Bm+d6JbqA21YQmd1Kt4RzLJR1W+\n\
-----END PRIVATE KEY-----";

    // JWK Set whose public key matches TEST_EC_PRIVATE_KEY above.
    // x/y coordinates derived from the SubjectPublicKeyInfo DER.
    const TEST_JWKS_JSON: &str = r#"{
      "keys": [{
        "kty": "EC",
        "crv": "P-256",
        "use": "sig",
        "kid": "test-kid",
        "x": "w7JAoU_gJbZJvV-zCOvU9yFJq0FNC_edCMRM78P8eQQ",
        "y": "wQg1EytcsEmGrM70Gb53oluoDbVhCZ3Uq3hHMslHVb4"
      }]
    }"#;

    fn future_exp() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
            + 3600
    }

    fn past_exp() -> i64 {
        // Use an hour in the past to stay clear of jsonwebtoken's default 60-second leeway.
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
            - 3600
    }

    fn test_jwks() -> JwkSet {
        serde_json::from_str(TEST_JWKS_JSON).unwrap()
    }

    fn make_provider(
        name: &str,
        user_id_claim: &str,
        role_claim: &str,
        role_mappings: HashMap<String, String>,
    ) -> OidcAuthProvider {
        OidcAuthProvider::for_testing(name, user_id_claim, role_claim, role_mappings, test_jwks())
    }

    fn default_provider() -> OidcAuthProvider {
        make_provider(
            "oidc",
            "sub",
            "role",
            [
                ("admin".to_owned(), "admin".to_owned()),
                ("developer".to_owned(), "user".to_owned()),
                ("viewer".to_owned(), "anonymous".to_owned()),
            ]
            .into(),
        )
    }

    fn signed_token(extra_header_kid: Option<&str>, claims: serde_json::Value) -> String {
        let header = Header {
            alg: Algorithm::ES256,
            kid: extra_header_kid.map(str::to_owned),
            ..Default::default()
        };
        let key = EncodingKey::from_ec_pem(TEST_EC_PRIVATE_KEY.as_bytes()).unwrap();
        encode(&header, &claims, &key).unwrap()
    }

    fn bearer(token: &str) -> RawAuthRequest {
        RawAuthRequest {
            headers: [("authorization".to_owned(), format!("Bearer {token}"))].into(),
            query_params: Default::default(),
        }
    }

    fn no_auth() -> RawAuthRequest {
        RawAuthRequest {
            headers: Default::default(),
            query_params: Default::default(),
        }
    }

    // ── Header parsing ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn no_auth_header_returns_none() {
        let p = default_provider();
        assert!(p.authenticate(&no_auth()).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn basic_auth_header_returns_none() {
        let p = default_provider();
        let req = RawAuthRequest {
            headers: [("authorization".to_owned(), "Basic dXNlcjpwYXNz".to_owned())].into(),
            query_params: Default::default(),
        };
        assert!(p.authenticate(&req).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn malformed_token_string_returns_auth_error() {
        let p = default_provider();
        let err = p
            .authenticate(&bearer("not.a.valid.jwt"))
            .await
            .unwrap_err();
        assert!(matches!(err, CoreError::Auth(_)));
    }

    // ── Role mapping ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn string_role_claim_maps_to_correct_role() {
        let p = default_provider();
        let token = signed_token(
            Some("test-kid"),
            json!({ "sub": "alice", "role": "developer", "exp": future_exp() }),
        );
        let id = p.authenticate(&bearer(&token)).await.unwrap().unwrap();
        assert_eq!(id.role, Role::User);
        assert_eq!(id.user_id.as_deref(), Some("alice"));
    }

    #[tokio::test]
    async fn array_role_claim_picks_highest_role() {
        let p = default_provider();
        let token = signed_token(
            Some("test-kid"),
            json!({ "sub": "bob", "role": ["viewer", "developer", "admin"], "exp": future_exp() }),
        );
        let id = p.authenticate(&bearer(&token)).await.unwrap().unwrap();
        assert_eq!(id.role, Role::Admin);
    }

    #[tokio::test]
    async fn array_with_one_known_entry_returns_that_role() {
        let p = default_provider();
        let token = signed_token(
            Some("test-kid"),
            json!({ "sub": "carol", "role": ["unknown-group", "developer"], "exp": future_exp() }),
        );
        let id = p.authenticate(&bearer(&token)).await.unwrap().unwrap();
        assert_eq!(id.role, Role::User);
    }

    #[tokio::test]
    async fn unmapped_string_role_defaults_to_anonymous() {
        let p = default_provider();
        let token = signed_token(
            Some("test-kid"),
            json!({ "sub": "dave", "role": "superuser", "exp": future_exp() }),
        );
        let id = p.authenticate(&bearer(&token)).await.unwrap().unwrap();
        assert_eq!(id.role, Role::Anonymous);
    }

    #[tokio::test]
    async fn all_unmapped_array_values_default_to_anonymous() {
        let p = default_provider();
        let token = signed_token(
            Some("test-kid"),
            json!({ "sub": "dave", "role": ["unknown1", "unknown2"], "exp": future_exp() }),
        );
        let id = p.authenticate(&bearer(&token)).await.unwrap().unwrap();
        assert_eq!(id.role, Role::Anonymous);
    }

    #[tokio::test]
    async fn missing_role_claim_defaults_to_anonymous() {
        let p = default_provider();
        let token = signed_token(
            Some("test-kid"),
            json!({ "sub": "eve", "exp": future_exp() }),
        );
        let id = p.authenticate(&bearer(&token)).await.unwrap().unwrap();
        assert_eq!(id.role, Role::Anonymous);
    }

    #[tokio::test]
    async fn custom_user_id_claim_is_extracted() {
        let p = make_provider(
            "oidc",
            "email",
            "role",
            [("admin".to_owned(), "admin".to_owned())].into(),
        );
        let token = signed_token(
            Some("test-kid"),
            json!({ "email": "alice@example.com", "role": "admin", "exp": future_exp() }),
        );
        let id = p.authenticate(&bearer(&token)).await.unwrap().unwrap();
        assert_eq!(id.user_id.as_deref(), Some("alice@example.com"));
        assert_eq!(id.role, Role::Admin);
    }

    #[tokio::test]
    async fn missing_user_id_claim_leaves_user_id_none() {
        let p = default_provider();
        let token = signed_token(
            Some("test-kid"),
            json!({ "role": "admin", "exp": future_exp() }),
        );
        let id = p.authenticate(&bearer(&token)).await.unwrap().unwrap();
        assert_eq!(id.user_id, None);
    }

    // ── JWT validation errors ─────────────────────────────────────────────────

    #[tokio::test]
    async fn expired_token_returns_none() {
        let p = default_provider();
        let token = signed_token(
            Some("test-kid"),
            json!({ "sub": "frank", "role": "admin", "exp": past_exp() }),
        );
        assert!(p.authenticate(&bearer(&token)).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn unknown_kid_returns_auth_error() {
        let p = default_provider();
        let token = signed_token(
            Some("unknown-key-id"),
            json!({ "sub": "grace", "exp": future_exp() }),
        );
        let err = p.authenticate(&bearer(&token)).await.unwrap_err();
        assert!(matches!(err, CoreError::Auth(_)));
    }

    #[tokio::test]
    async fn token_without_kid_uses_first_jwk() {
        let p = default_provider();
        // No kid in header — falls back to jwks.keys[0]
        let token = signed_token(
            None,
            json!({ "sub": "henry", "role": "developer", "exp": future_exp() }),
        );
        let id = p.authenticate(&bearer(&token)).await.unwrap().unwrap();
        assert_eq!(id.role, Role::User);
    }

    // ── Identity metadata ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn auth_provider_name_defaults_to_oidc() {
        assert_eq!(default_provider().name(), "oidc");
    }

    #[tokio::test]
    async fn auth_provider_name_is_configurable() {
        let p = make_provider("authentik", "sub", "role", HashMap::new());
        assert_eq!(p.name(), "authentik");
    }

    #[tokio::test]
    async fn identity_auth_provider_reflects_configured_name() {
        let p = make_provider("oidc1", "sub", "role", HashMap::new());
        let token = signed_token(
            Some("test-kid"),
            json!({ "sub": "iris", "exp": future_exp() }),
        );
        let id = p.authenticate(&bearer(&token)).await.unwrap().unwrap();
        assert_eq!(id.auth_provider.as_deref(), Some("oidc1"));
    }

    #[tokio::test]
    async fn array_role_claim_populates_groups_with_provider_name_prefix() {
        let p = default_provider(); // name="oidc", role_mappings: admin/developer/viewer
        let token = signed_token(
            Some("test-kid"),
            json!({ "sub": "alice", "role": ["team-a", "team-b"], "exp": future_exp() }),
        );
        let id = p.authenticate(&bearer(&token)).await.unwrap().unwrap();
        // Neither "team-a" nor "team-b" is in role_mappings → prefixed with provider name
        assert!(id.groups.contains(&"oidc:team-a".to_owned()));
        assert!(id.groups.contains(&"oidc:team-b".to_owned()));
    }

    #[tokio::test]
    async fn named_provider_uses_its_name_as_prefix() {
        let p = make_provider("oidc2", "sub", "role", HashMap::new());
        let token = signed_token(
            Some("test-kid"),
            json!({ "sub": "alice", "role": "team-a", "exp": future_exp() }),
        );
        let id = p.authenticate(&bearer(&token)).await.unwrap().unwrap();
        assert_eq!(id.groups, vec!["oidc2:team-a".to_owned()]);
    }

    #[tokio::test]
    async fn mapped_role_claim_values_have_no_prefix() {
        let p = default_provider(); // "admin" is in role_mappings
        let token = signed_token(
            Some("test-kid"),
            json!({ "sub": "alice", "role": ["admin", "team-a"], "exp": future_exp() }),
        );
        let id = p.authenticate(&bearer(&token)).await.unwrap().unwrap();
        assert!(
            id.groups.contains(&"admin".to_owned()),
            "mapped value stored without prefix"
        );
        assert!(
            id.groups.contains(&"oidc:team-a".to_owned()),
            "unmapped value stored with provider name prefix"
        );
    }

    #[tokio::test]
    async fn string_role_claim_populates_single_group_with_prefix() {
        let p = default_provider();
        let token = signed_token(
            Some("test-kid"),
            json!({ "sub": "alice", "role": "team-a", "exp": future_exp() }),
        );
        let id = p.authenticate(&bearer(&token)).await.unwrap().unwrap();
        assert_eq!(id.groups, vec!["oidc:team-a".to_owned()]);
    }

    #[tokio::test]
    async fn missing_role_claim_yields_empty_groups() {
        let p = default_provider();
        let token = signed_token(
            Some("test-kid"),
            json!({ "sub": "alice", "exp": future_exp() }),
        );
        let id = p.authenticate(&bearer(&token)).await.unwrap().unwrap();
        assert!(id.groups.is_empty());
    }

    // ── lowercase bearer prefix ───────────────────────────────────────────────

    #[tokio::test]
    async fn lowercase_bearer_prefix_is_accepted() {
        let p = default_provider();
        let token = signed_token(
            Some("test-kid"),
            json!({ "sub": "alice", "role": "admin", "exp": future_exp() }),
        );
        let req = RawAuthRequest {
            headers: [("authorization".to_owned(), format!("bearer {token}"))].into(),
            query_params: Default::default(),
        };
        let id = p.authenticate(&req).await.unwrap().unwrap();
        assert_eq!(id.role, Role::Admin);
    }

    // ── percent_encode ────────────────────────────────────────────────────────

    #[test]
    fn percent_encode_alphanumeric_and_safe_chars_unchanged() {
        assert_eq!(percent_encode("abc123-_.~"), "abc123-_.~");
    }

    #[test]
    fn percent_encode_encodes_space_and_special_chars() {
        let encoded = percent_encode("hello world+foo=bar");
        assert!(encoded.contains("%20"), "space should be %20");
        assert!(encoded.contains("%2B"), "plus should be %2B");
        assert!(encoded.contains("%3D"), "equals should be %3D");
        assert!(
            !encoded.contains(' '),
            "encoded string should have no raw spaces"
        );
    }

    // ── OidcSsoFlow::authorization_url ────────────────────────────────────────

    #[test]
    fn authorization_url_contains_required_params() {
        let flow = OidcSsoFlow {
            name: "oidc".to_owned(),
            client_id: "my-client".to_owned(),
            client_secret: None,
            redirect_uri: "https://app.example.com/callback".to_owned(),
            scopes: vec!["openid".to_owned(), "profile".to_owned()],
            authorization_endpoint: "https://idp.example.com/auth".to_owned(),
            token_endpoint: "https://idp.example.com/token".to_owned(),
            frontend_url: "https://app.example.com".to_owned(),
            http: reqwest::Client::new(),
        };
        let url = flow.authorization_url("csrf-state-123");
        assert!(url.starts_with("https://idp.example.com/auth?"));
        assert!(url.contains("client_id=my-client"));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("state=csrf-state-123"));
        assert!(url.contains("redirect_uri="));
        assert!(url.contains("scope="));
    }

    // ── sso_flow() accessor ───────────────────────────────────────────────────

    #[test]
    fn sso_flow_returns_none_when_not_configured() {
        let p = default_provider();
        assert!(p.sso_flow().is_none());
    }

    // ── OidcAuthProvider::new() + exchange_code + refresh (mockito) ───────────

    fn discovery_json(base_url: &str) -> String {
        serde_json::json!({
            "issuer": base_url,
            "jwks_uri": format!("{base_url}/jwks"),
            "authorization_endpoint": format!("{base_url}/auth"),
            "token_endpoint": format!("{base_url}/token"),
        })
        .to_string()
    }

    #[tokio::test]
    async fn new_bootstraps_provider_from_discovery_document() {
        let mut server = mockito::Server::new_async().await;
        let base = server.url();

        let _discovery = server
            .mock("GET", "/.well-known/openid-configuration")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(discovery_json(&base))
            .create_async()
            .await;

        let _jwks = server
            .mock("GET", "/jwks")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(TEST_JWKS_JSON)
            .create_async()
            .await;

        use batlehub_core::ports::OidcAuthConfig;
        let cfg = OidcAuthConfig {
            name: "test".to_owned(),
            issuer_url: base.clone(),
            client_id: "my-client".to_owned(),
            client_secret: None,
            redirect_uri: None,
            frontend_url: String::new(),
            scopes: vec!["openid".to_owned()],
            user_id_claim: "sub".to_owned(),
            role_claim: "role".to_owned(),
            role_mappings: HashMap::new(),
        };

        let provider = OidcAuthProvider::new(&cfg)
            .await
            .expect("provider construction failed");
        assert!(provider.sso_flow().is_none());
    }

    #[tokio::test]
    async fn new_with_redirect_uri_creates_sso_flow() {
        let mut server = mockito::Server::new_async().await;
        let base = server.url();

        let _discovery = server
            .mock("GET", "/.well-known/openid-configuration")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(discovery_json(&base))
            .create_async()
            .await;

        let _jwks = server
            .mock("GET", "/jwks")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(TEST_JWKS_JSON)
            .create_async()
            .await;

        use batlehub_core::ports::OidcAuthConfig;
        let cfg = OidcAuthConfig {
            name: "oidc".to_owned(),
            issuer_url: base.clone(),
            client_id: "my-client".to_owned(),
            client_secret: Some("secret".to_owned()),
            redirect_uri: Some("https://app.example.com/callback".to_owned()),
            frontend_url: "https://app.example.com".to_owned(),
            scopes: vec!["openid".to_owned()],
            user_id_claim: "sub".to_owned(),
            role_claim: "role".to_owned(),
            role_mappings: HashMap::new(),
        };

        let provider = OidcAuthProvider::new(&cfg)
            .await
            .expect("provider construction failed");
        let sso = provider
            .sso_flow()
            .expect("sso_flow should be Some with redirect_uri");
        let auth_url = sso.authorization_url("test-state");
        assert!(auth_url.contains("state=test-state"));
    }

    #[tokio::test]
    async fn exchange_code_sends_code_grant_request() {
        let mut server = mockito::Server::new_async().await;
        let base = server.url();

        let _discovery = server
            .mock("GET", "/.well-known/openid-configuration")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(discovery_json(&base))
            .create_async()
            .await;

        let _jwks = server
            .mock("GET", "/jwks")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(TEST_JWKS_JSON)
            .create_async()
            .await;

        let _token = server
            .mock("POST", "/token")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"access_token":"at-123","refresh_token":"rt-xyz","expires_in":3600}"#)
            .create_async()
            .await;

        use batlehub_core::ports::OidcAuthConfig;
        let cfg = OidcAuthConfig {
            name: "oidc".to_owned(),
            issuer_url: base.clone(),
            client_id: "my-client".to_owned(),
            client_secret: Some("secret".to_owned()),
            redirect_uri: Some("https://app.example.com/callback".to_owned()),
            frontend_url: String::new(),
            scopes: vec!["openid".to_owned()],
            user_id_claim: "sub".to_owned(),
            role_claim: "role".to_owned(),
            role_mappings: HashMap::new(),
        };

        let provider = OidcAuthProvider::new(&cfg).await.unwrap();
        let sso = provider.sso_flow().unwrap();
        let tokens = sso.exchange_code("auth-code-abc").await.unwrap();
        assert_eq!(tokens.access_token, "at-123");
        assert_eq!(tokens.refresh_token.as_deref(), Some("rt-xyz"));
        assert_eq!(tokens.expires_in, Some(3600));
    }

    #[tokio::test]
    async fn refresh_sends_refresh_token_grant_request() {
        let mut server = mockito::Server::new_async().await;
        let base = server.url();

        let _discovery = server
            .mock("GET", "/.well-known/openid-configuration")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(discovery_json(&base))
            .create_async()
            .await;

        let _jwks = server
            .mock("GET", "/jwks")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(TEST_JWKS_JSON)
            .create_async()
            .await;

        let _token = server
            .mock("POST", "/token")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"access_token":"new-at","expires_in":1800}"#)
            .create_async()
            .await;

        use batlehub_core::ports::OidcAuthConfig;
        let cfg = OidcAuthConfig {
            name: "oidc".to_owned(),
            issuer_url: base,
            client_id: "my-client".to_owned(),
            client_secret: None,
            redirect_uri: Some("https://app.example.com/callback".to_owned()),
            frontend_url: String::new(),
            scopes: vec!["openid".to_owned()],
            user_id_claim: "sub".to_owned(),
            role_claim: "role".to_owned(),
            role_mappings: HashMap::new(),
        };

        let provider = OidcAuthProvider::new(&cfg).await.unwrap();
        let sso = provider.sso_flow().unwrap();
        let tokens = sso.refresh("old-refresh-token").await.unwrap();
        assert_eq!(tokens.access_token, "new-at");
        assert_eq!(tokens.expires_in, Some(1800));
    }

    // ── JWKS cache refresh path ───────────────────────────────────────────────

    #[tokio::test]
    async fn get_decoding_key_refreshes_stale_jwks_cache() {
        let mut server = mockito::Server::new_async().await;
        let jwks_url = format!("{}/jwks", server.url());

        let _m = server
            .mock("GET", "/jwks")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(TEST_JWKS_JSON)
            .create_async()
            .await;

        // Create a provider with a stale JWKS cache (older than JWKS_MIN_REFRESH).
        let p = OidcAuthProvider {
            name: "oidc".to_owned(),
            issuer: String::new(),
            user_id_claim: "sub".to_owned(),
            role_claim: "role".to_owned(),
            role_mappings: HashMap::new(),
            http: reqwest::Client::new(),
            jwks_uri: jwks_url,
            cache: Arc::new(RwLock::new(JwksCache {
                keys: serde_json::from_str::<JwkSet>(r#"{"keys":[]}"#).unwrap(),
                fetched_at: Instant::now() - Duration::from_secs(301),
            })),
            sso: None,
        };

        // Token signed with test-kid — not in the stale empty cache but in the fresh JWKS.
        let token = signed_token(
            Some("test-kid"),
            json!({ "sub": "alice", "exp": future_exp() }),
        );
        let id = p.authenticate(&bearer(&token)).await.unwrap().unwrap();
        assert_eq!(id.user_id.as_deref(), Some("alice"));
    }
}
