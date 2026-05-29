use std::collections::HashMap;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use batlehub_config::schema::KubernetesAuthConfig;
use batlehub_core::{
    entities::{Identity, Role},
    error::CoreError,
    ports::{AuthProvider, RawAuthRequest},
};

const IN_CLUSTER_CA: &str = "/var/run/secrets/kubernetes.io/serviceaccount/ca.crt";
const IN_CLUSTER_TOKEN: &str = "/var/run/secrets/kubernetes.io/serviceaccount/token";

// ── TokenReview wire types ────────────────────────────────────────────────────

#[derive(Serialize)]
struct TokenReviewRequest {
    #[serde(rename = "apiVersion")]
    api_version: &'static str,
    kind: &'static str,
    spec: TokenReviewSpec,
}

#[derive(Serialize)]
struct TokenReviewSpec {
    token: String,
    audiences: Vec<String>,
}

#[derive(Deserialize)]
struct TokenReviewResponse {
    status: TokenReviewStatus,
}

#[derive(Deserialize, Default)]
struct TokenReviewStatus {
    #[serde(default)]
    authenticated: bool,
    #[serde(default)]
    user: Option<UserInfo>,
}

#[derive(Deserialize)]
struct UserInfo {
    username: String,
    #[serde(default)]
    groups: Vec<String>,
}

// ── Provider ──────────────────────────────────────────────────────────────────

pub struct KubernetesAuthProvider {
    name: String,
    http: reqwest::Client,
    tokenreview_url: String,
    self_token_path: String,
    audiences: Vec<String>,
    role_mappings: HashMap<String, Role>,
}

impl KubernetesAuthProvider {
    pub async fn new(cfg: &KubernetesAuthConfig) -> anyhow::Result<Self> {
        let ca_cert_path = cfg.ca_cert_path.as_deref().unwrap_or(IN_CLUSTER_CA);
        let ca_bytes = tokio::fs::read(ca_cert_path).await.map_err(|e| {
            anyhow::anyhow!("reading Kubernetes CA cert from '{ca_cert_path}': {e}")
        })?;
        let ca_cert = reqwest::Certificate::from_pem(&ca_bytes)
            .map_err(|e| anyhow::anyhow!("parsing Kubernetes CA cert: {e}"))?;

        let http = reqwest::Client::builder()
            .add_root_certificate(ca_cert)
            .build()
            .map_err(|e| anyhow::anyhow!("building HTTP client for Kubernetes auth: {e}"))?;

        let api_server = cfg.api_server.clone().unwrap_or_else(|| {
            let host = std::env::var("KUBERNETES_SERVICE_HOST")
                .unwrap_or_else(|_| "kubernetes.default.svc".to_owned());
            let port =
                std::env::var("KUBERNETES_SERVICE_PORT").unwrap_or_else(|_| "443".to_owned());
            format!("https://{host}:{port}")
        });

        let self_token_path = cfg
            .token_path
            .clone()
            .unwrap_or_else(|| IN_CLUSTER_TOKEN.to_owned());

        // Verify the file is readable on startup so misconfiguration fails fast.
        tokio::fs::read_to_string(&self_token_path)
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "reading batlehub service account token from '{self_token_path}': {e}"
                )
            })?;

        let audiences = if cfg.audiences.is_empty() {
            vec!["batlehub".to_owned()]
        } else {
            cfg.audiences.clone()
        };

        let role_mappings = cfg
            .role_mappings
            .iter()
            .filter_map(|(k, v)| {
                let role = match v.as_str() {
                    "admin" => Role::Admin,
                    "user" => Role::User,
                    "anonymous" => Role::Anonymous,
                    _ => return None,
                };
                Some((k.clone(), role))
            })
            .collect();

        Ok(Self {
            name: cfg.name.clone(),
            http,
            tokenreview_url: format!("{api_server}/apis/authentication.k8s.io/v1/tokenreviews"),
            self_token_path,
            audiences,
            role_mappings,
        })
    }

    fn resolve_role(&self, username: &str, groups: &[String]) -> Role {
        // Check username (most specific) then groups. Take the highest role found.
        std::iter::once(username)
            .chain(groups.iter().map(String::as_str))
            .filter_map(|key| self.role_mappings.get(key))
            .cloned()
            .max()
            .unwrap_or(Role::Anonymous)
    }

    fn resolve_groups(&self, k8s_groups: &[String]) -> Vec<String> {
        // Groups in role_mappings are known/configured — keep them as-is.
        // Unmapped groups are prefixed with the provider name to avoid cross-provider collisions.
        k8s_groups
            .iter()
            .map(|g| {
                if self.role_mappings.contains_key(g) {
                    g.clone()
                } else {
                    format!("{}:{g}", self.name)
                }
            })
            .collect()
    }
}

#[async_trait]
impl AuthProvider for KubernetesAuthProvider {
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

        // Re-read the service account token each call — Kubernetes rotates it.
        let self_token = tokio::fs::read_to_string(&self.self_token_path)
            .await
            .map_err(|e| CoreError::Auth(format!("reading service account token: {e}")))?;

        let body = TokenReviewRequest {
            api_version: "authentication.k8s.io/v1",
            kind: "TokenReview",
            spec: TokenReviewSpec {
                token: token.to_owned(),
                audiences: self.audiences.clone(),
            },
        };

        let resp: TokenReviewResponse = self
            .http
            .post(&self.tokenreview_url)
            .bearer_auth(self_token.trim())
            .json(&body)
            .send()
            .await
            .map_err(|e| CoreError::Auth(format!("Kubernetes TokenReview request failed: {e}")))?
            .json()
            .await
            .map_err(|e| {
                CoreError::Auth(format!("parsing Kubernetes TokenReview response: {e}"))
            })?;

        if !resp.status.authenticated {
            // Not a valid k8s token — let other providers have a turn.
            return Ok(None);
        }

        let user = resp.status.user.unwrap_or(UserInfo {
            username: String::new(),
            groups: vec![],
        });

        let role = self.resolve_role(&user.username, &user.groups);
        let groups = self.resolve_groups(&user.groups);

        Ok(Some(Identity {
            user_id: Some(user.username),
            role,
            auth_provider: Some(self.name.clone()),
            groups,
        }))
    }
}

/// Test-only constructor that skips TLS setup and filesystem validation.
#[cfg(test)]
impl KubernetesAuthProvider {
    fn for_testing(
        http: reqwest::Client,
        tokenreview_url: impl Into<String>,
        self_token_path: impl Into<String>,
        audiences: Vec<String>,
        role_mappings: HashMap<String, Role>,
    ) -> Self {
        Self::for_testing_named(
            "kubernetes",
            http,
            tokenreview_url,
            self_token_path,
            audiences,
            role_mappings,
        )
    }

    fn for_testing_named(
        name: impl Into<String>,
        http: reqwest::Client,
        tokenreview_url: impl Into<String>,
        self_token_path: impl Into<String>,
        audiences: Vec<String>,
        role_mappings: HashMap<String, Role>,
    ) -> Self {
        Self {
            name: name.into(),
            http,
            tokenreview_url: tokenreview_url.into(),
            self_token_path: self_token_path.into(),
            audiences,
            role_mappings,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockito::Server;
    use std::collections::HashMap;

    struct TempFile(String);
    impl Drop for TempFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    async fn write_temp_token(content: &str) -> TempFile {
        let path = format!(
            "/tmp/k8s-test-token-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        tokio::fs::write(&path, content).await.unwrap();
        TempFile(path)
    }

    fn default_mappings() -> HashMap<String, Role> {
        [
            (
                "system:serviceaccount:prod:ci-deployer".to_owned(),
                Role::Admin,
            ),
            ("system:serviceaccounts:dev".to_owned(), Role::User),
            ("system:serviceaccounts".to_owned(), Role::Anonymous),
        ]
        .into()
    }

    fn make_provider(server: &Server, token_path: &str) -> KubernetesAuthProvider {
        KubernetesAuthProvider::for_testing(
            reqwest::Client::new(),
            format!(
                "{}/apis/authentication.k8s.io/v1/tokenreviews",
                server.url()
            ),
            token_path.to_owned(),
            vec!["batlehub".to_owned()],
            default_mappings(),
        )
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
        let server = Server::new_async().await;
        let tf = write_temp_token("self-token").await;
        let p = make_provider(&server, &tf.0);
        assert!(p.authenticate(&no_auth()).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn basic_auth_header_returns_none() {
        let server = Server::new_async().await;
        let tf = write_temp_token("self-token").await;
        let p = make_provider(&server, &tf.0);
        let req = RawAuthRequest {
            headers: [("authorization".to_owned(), "Basic dXNlcjpwYXNz".to_owned())].into(),
            query_params: Default::default(),
        };
        assert!(p.authenticate(&req).await.unwrap().is_none());
    }

    // ── resolve_role ──────────────────────────────────────────────────────────

    #[test]
    fn username_alone_maps_to_admin() {
        let p = KubernetesAuthProvider::for_testing(
            reqwest::Client::new(),
            String::new(),
            String::new(),
            vec![],
            default_mappings(),
        );
        assert_eq!(
            p.resolve_role("system:serviceaccount:prod:ci-deployer", &[]),
            Role::Admin
        );
    }

    #[test]
    fn group_maps_to_user_when_username_unmapped() {
        let p = KubernetesAuthProvider::for_testing(
            reqwest::Client::new(),
            String::new(),
            String::new(),
            vec![],
            default_mappings(),
        );
        let groups = vec!["system:serviceaccounts:dev".to_owned()];
        assert_eq!(
            p.resolve_role("system:serviceaccount:staging:other", &groups),
            Role::User
        );
    }

    #[test]
    fn highest_role_wins_across_multiple_groups() {
        let p = KubernetesAuthProvider::for_testing(
            reqwest::Client::new(),
            String::new(),
            String::new(),
            vec![],
            default_mappings(),
        );
        let groups = vec![
            "system:serviceaccounts".to_owned(),     // → Anonymous
            "system:serviceaccounts:dev".to_owned(), // → User
        ];
        // User > Anonymous, so User wins
        assert_eq!(p.resolve_role("unmapped-user", &groups), Role::User);
    }

    #[test]
    fn username_beats_group_when_username_has_higher_role() {
        let p = KubernetesAuthProvider::for_testing(
            reqwest::Client::new(),
            String::new(),
            String::new(),
            vec![],
            default_mappings(),
        );
        // username → Admin, group → User: Admin should win
        let groups = vec!["system:serviceaccounts:dev".to_owned()];
        assert_eq!(
            p.resolve_role("system:serviceaccount:prod:ci-deployer", &groups),
            Role::Admin
        );
    }

    #[test]
    fn no_match_at_all_returns_anonymous() {
        let p = KubernetesAuthProvider::for_testing(
            reqwest::Client::new(),
            String::new(),
            String::new(),
            vec![],
            default_mappings(),
        );
        let groups = vec!["system:authenticated".to_owned()];
        assert_eq!(p.resolve_role("unknown-user", &groups), Role::Anonymous);
    }

    #[test]
    fn empty_mappings_always_returns_anonymous() {
        let p = KubernetesAuthProvider::for_testing(
            reqwest::Client::new(),
            String::new(),
            String::new(),
            vec![],
            HashMap::new(),
        );
        let groups = vec!["system:serviceaccounts:prod".to_owned()];
        assert_eq!(
            p.resolve_role("system:serviceaccount:prod:ci-deployer", &groups),
            Role::Anonymous
        );
    }

    // ── Full authenticate flow ────────────────────────────────────────────────

    #[tokio::test]
    async fn authenticated_token_username_maps_to_admin() {
        let mut server = Server::new_async().await;
        let _m = server
            .mock("POST", "/apis/authentication.k8s.io/v1/tokenreviews")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"status":{"authenticated":true,"user":{"username":"system:serviceaccount:prod:ci-deployer","groups":["system:serviceaccounts","system:serviceaccounts:prod","system:authenticated"]}}}"#)
            .create_async()
            .await;

        let tf = write_temp_token("self-token").await;
        let p = make_provider(&server, &tf.0);
        let id = p
            .authenticate(&bearer("k8s-sa-token"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(id.role, Role::Admin);
        assert_eq!(
            id.user_id.as_deref(),
            Some("system:serviceaccount:prod:ci-deployer")
        );
        assert_eq!(id.auth_provider.as_deref(), Some("kubernetes"));
    }

    #[tokio::test]
    async fn authenticated_token_group_maps_to_user() {
        let mut server = Server::new_async().await;
        let _m = server
            .mock("POST", "/apis/authentication.k8s.io/v1/tokenreviews")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"status":{"authenticated":true,"user":{"username":"system:serviceaccount:dev:my-app","groups":["system:serviceaccounts:dev","system:serviceaccounts","system:authenticated"]}}}"#)
            .create_async()
            .await;

        let tf = write_temp_token("self-token").await;
        let p = make_provider(&server, &tf.0);
        let id = p.authenticate(&bearer("dev-token")).await.unwrap().unwrap();
        assert_eq!(id.role, Role::User);
    }

    #[tokio::test]
    async fn unauthenticated_response_returns_none() {
        let mut server = Server::new_async().await;
        let _m = server
            .mock("POST", "/apis/authentication.k8s.io/v1/tokenreviews")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"status":{"authenticated":false}}"#)
            .create_async()
            .await;

        let tf = write_temp_token("self-token").await;
        let p = make_provider(&server, &tf.0);
        assert!(p
            .authenticate(&bearer("invalid-token"))
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn unmapped_service_account_defaults_to_anonymous() {
        let mut server = Server::new_async().await;
        let _m = server
            .mock("POST", "/apis/authentication.k8s.io/v1/tokenreviews")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"status":{"authenticated":true,"user":{"username":"system:serviceaccount:unknown-ns:pod","groups":["system:authenticated"]}}}"#)
            .create_async()
            .await;

        let tf = write_temp_token("self-token").await;
        let p = make_provider(&server, &tf.0);
        let id = p.authenticate(&bearer("sa-token")).await.unwrap().unwrap();
        assert_eq!(id.role, Role::Anonymous);
    }

    #[tokio::test]
    async fn k8s_api_server_error_propagates_as_auth_error() {
        let mut server = Server::new_async().await;
        let _m = server
            .mock("POST", "/apis/authentication.k8s.io/v1/tokenreviews")
            .with_status(500)
            .with_body("Internal Server Error")
            .create_async()
            .await;

        let tf = write_temp_token("self-token").await;
        let p = make_provider(&server, &tf.0);
        assert!(p.authenticate(&bearer("some-token")).await.is_err());
    }

    #[tokio::test]
    async fn tokenreview_request_sends_correct_audience() {
        let mut server = Server::new_async().await;
        let _m = server
            .mock("POST", "/apis/authentication.k8s.io/v1/tokenreviews")
            .match_body(mockito::Matcher::PartialJson(
                serde_json::json!({"spec":{"audiences":["batlehub"]}}),
            ))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"status":{"authenticated":false}}"#)
            .create_async()
            .await;

        let tf = write_temp_token("self-token").await;
        let p = make_provider(&server, &tf.0);
        let _ = p.authenticate(&bearer("any-token")).await;
        _m.assert_async().await;
    }

    #[tokio::test]
    async fn provider_name_defaults_to_kubernetes() {
        let server = Server::new_async().await;
        let tf = write_temp_token("self-token").await;
        let p = make_provider(&server, &tf.0);
        assert_eq!(p.name(), "kubernetes");
    }

    #[test]
    fn provider_name_is_configurable() {
        let p = KubernetesAuthProvider::for_testing_named(
            "k8s-prod",
            reqwest::Client::new(),
            String::new(),
            String::new(),
            vec![],
            HashMap::new(),
        );
        assert_eq!(p.name(), "k8s-prod");
    }

    // ── resolve_groups ────────────────────────────────────────────────────────

    #[test]
    fn mapped_group_stored_without_prefix() {
        let p = KubernetesAuthProvider::for_testing(
            reqwest::Client::new(),
            String::new(),
            String::new(),
            vec![],
            default_mappings(),
        );
        // "system:serviceaccounts:dev" is in default_mappings → no prefix
        let groups = p.resolve_groups(&["system:serviceaccounts:dev".to_owned()]);
        assert_eq!(groups, vec!["system:serviceaccounts:dev".to_owned()]);
    }

    #[test]
    fn unmapped_group_gets_provider_name_prefix() {
        let p = KubernetesAuthProvider::for_testing(
            reqwest::Client::new(),
            String::new(),
            String::new(),
            vec![],
            default_mappings(),
        );
        // "system:authenticated" is not in role_mappings → prefixed with provider name
        let groups = p.resolve_groups(&["system:authenticated".to_owned()]);
        assert_eq!(groups, vec!["kubernetes:system:authenticated".to_owned()]);
    }

    #[test]
    fn named_provider_uses_its_name_as_prefix() {
        let p = KubernetesAuthProvider::for_testing_named(
            "k8s-prod",
            reqwest::Client::new(),
            String::new(),
            String::new(),
            vec![],
            default_mappings(),
        );
        let groups = p.resolve_groups(&["team-a".to_owned()]);
        assert_eq!(groups, vec!["k8s-prod:team-a".to_owned()]);
    }

    #[test]
    fn mixed_groups_prefix_only_unmapped() {
        let p = KubernetesAuthProvider::for_testing(
            reqwest::Client::new(),
            String::new(),
            String::new(),
            vec![],
            default_mappings(),
        );
        let raw = vec![
            "system:serviceaccounts:dev".to_owned(), // mapped → no prefix
            "system:serviceaccounts".to_owned(),     // mapped → no prefix
            "system:authenticated".to_owned(),       // unmapped → kubernetes:
            "team-a".to_owned(),                     // unmapped → kubernetes:
        ];
        let groups = p.resolve_groups(&raw);
        assert!(groups.contains(&"system:serviceaccounts:dev".to_owned()));
        assert!(groups.contains(&"system:serviceaccounts".to_owned()));
        assert!(groups.contains(&"kubernetes:system:authenticated".to_owned()));
        assert!(groups.contains(&"kubernetes:team-a".to_owned()));
        assert!(
            !groups.contains(&"team-a".to_owned()),
            "unprefixed team-a should not exist"
        );
    }

    #[test]
    fn empty_groups_yields_empty_result() {
        let p = KubernetesAuthProvider::for_testing(
            reqwest::Client::new(),
            String::new(),
            String::new(),
            vec![],
            default_mappings(),
        );
        assert!(p.resolve_groups(&[]).is_empty());
    }

    // ── Full authenticate flow — groups field ─────────────────────────────────

    #[tokio::test]
    async fn authenticate_populates_identity_groups() {
        let mut server = Server::new_async().await;
        let _m = server
            .mock("POST", "/apis/authentication.k8s.io/v1/tokenreviews")
            .with_status(200)
            .with_header("content-type", "application/json")
            // Groups: one mapped ("system:serviceaccounts:dev"), one unmapped ("team-a")
            .with_body(r#"{"status":{"authenticated":true,"user":{"username":"system:serviceaccount:dev:my-app","groups":["system:serviceaccounts:dev","team-a","system:authenticated"]}}}"#)
            .create_async()
            .await;

        let tf = write_temp_token("self-token").await;
        let p = make_provider(&server, &tf.0);
        let id = p.authenticate(&bearer("dev-token")).await.unwrap().unwrap();

        assert!(
            id.groups.contains(&"system:serviceaccounts:dev".to_owned()),
            "mapped group stored without prefix"
        );
        assert!(
            id.groups.contains(&"kubernetes:team-a".to_owned()),
            "unmapped group stored with provider name prefix"
        );
        assert!(
            id.groups
                .contains(&"kubernetes:system:authenticated".to_owned()),
            "standard k8s group stored with provider name prefix"
        );
        assert!(
            !id.groups.contains(&"team-a".to_owned()),
            "unprefixed unmapped group must not exist"
        );
    }

    #[tokio::test]
    async fn authenticate_groups_empty_when_tokenreview_returns_no_groups() {
        let mut server = Server::new_async().await;
        let _m = server
            .mock("POST", "/apis/authentication.k8s.io/v1/tokenreviews")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"status":{"authenticated":true,"user":{"username":"system:serviceaccount:prod:ci-deployer","groups":[]}}}"#)
            .create_async()
            .await;

        let tf = write_temp_token("self-token").await;
        let p = make_provider(&server, &tf.0);
        let id = p.authenticate(&bearer("token")).await.unwrap().unwrap();
        assert!(id.groups.is_empty());
    }
}
