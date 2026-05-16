use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use jsonwebtoken::{decode, decode_header, DecodingKey, Validation};
use jsonwebtoken::jwk::JwkSet;
use serde::Deserialize;
use tokio::sync::RwLock;

use proxy_cache_config::schema::OidcAuthConfig;
use proxy_cache_core::{
    entities::{Identity, Role},
    error::CoreError,
    ports::{AuthProvider, RawAuthRequest},
};

const JWKS_MIN_REFRESH: Duration = Duration::from_secs(300);

#[derive(Deserialize)]
struct OidcDiscovery {
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

        let keys = fetch_jwks(&http, &discovery.jwks_uri)
            .await
            .map_err(|e| anyhow::anyhow!("fetching initial JWKS from {}: {e}", discovery.jwks_uri))?;

        let sso = cfg.redirect_uri.as_ref().map(|redirect_uri| OidcSsoFlow {
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
            .filter_map(|mapped| match mapped.as_str() {
                "admin" => Some(Role::Admin),
                "user" => Some(Role::User),
                "anonymous" => Some(Role::Anonymous),
                _ => None,
            })
            .max()
            .unwrap_or(Role::Anonymous)
    }
}

/// Test-only constructor that skips the network bootstrap.
#[cfg(test)]
impl OidcAuthProvider {
    fn for_testing(
        user_id_claim: impl Into<String>,
        role_claim: impl Into<String>,
        role_mappings: HashMap<String, String>,
        jwks: JwkSet,
    ) -> Self {
        Self {
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
        "oidc"
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

        // We are a resource server: skip audience validation since the audience
        // value is deployment-specific and not standardised across providers.
        let mut validation = Validation::new(header.alg);
        validation.validate_aud = false;

        let token_data = decode::<serde_json::Map<String, serde_json::Value>>(
            token,
            &decoding_key,
            &validation,
        )
        .map_err(|e| CoreError::Auth(format!("JWT validation failed: {e}")))?;

        let claims = token_data.claims;

        let user_id = claims
            .get(&self.user_id_claim)
            .and_then(|v| v.as_str())
            .map(str::to_owned);

        let role = claims
            .get(&self.role_claim)
            .map(|v| self.map_role(v))
            .unwrap_or(Role::Anonymous);

        Ok(Some(Identity {
            user_id,
            role,
            auth_provider: Some("oidc".to_owned()),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::time::{SystemTime, UNIX_EPOCH};
    use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
    use serde_json::json;

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
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64 + 3600
    }

    fn past_exp() -> i64 {
        // Use an hour in the past to stay clear of jsonwebtoken's default 60-second leeway.
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64 - 3600
    }

    fn test_jwks() -> JwkSet {
        serde_json::from_str(TEST_JWKS_JSON).unwrap()
    }

    fn make_provider(
        user_id_claim: &str,
        role_claim: &str,
        role_mappings: HashMap<String, String>,
    ) -> OidcAuthProvider {
        OidcAuthProvider::for_testing(user_id_claim, role_claim, role_mappings, test_jwks())
    }

    fn default_provider() -> OidcAuthProvider {
        make_provider(
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
        RawAuthRequest { headers: Default::default(), query_params: Default::default() }
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
        let err = p.authenticate(&bearer("not.a.valid.jwt")).await.unwrap_err();
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
    async fn expired_token_returns_auth_error() {
        let p = default_provider();
        let token = signed_token(
            Some("test-kid"),
            json!({ "sub": "frank", "role": "admin", "exp": past_exp() }),
        );
        let err = p.authenticate(&bearer(&token)).await.unwrap_err();
        assert!(matches!(err, CoreError::Auth(_)));
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
    async fn auth_provider_name_is_oidc() {
        assert_eq!(default_provider().name(), "oidc");
    }

    #[tokio::test]
    async fn identity_has_oidc_provider_field() {
        let p = default_provider();
        let token = signed_token(
            Some("test-kid"),
            json!({ "sub": "iris", "role": "admin", "exp": future_exp() }),
        );
        let id = p.authenticate(&bearer(&token)).await.unwrap().unwrap();
        assert_eq!(id.auth_provider.as_deref(), Some("oidc"));
    }
}
