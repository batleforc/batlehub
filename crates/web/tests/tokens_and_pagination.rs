//! Integration tests split from the former monolithic `integration.rs`
//! (see `tests/common/mod.rs` for shared app-factory infrastructure).

mod common;
#[allow(unused_imports)]
use common::*;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use actix_web::test::{call_service, read_body_json, TestRequest};
use async_trait::async_trait;
use chrono::Utc;
use serde_json::Value;

use batlehub_adapters::auth::StaticTokenAuthProvider;
use batlehub_adapters::cache::InMemoryCacheStore;
use batlehub_adapters::in_memory::{
    InMemoryPackageRepository as InMemoryRepo, InMemoryStorageBackend as InMemoryStorage,
    NoopArtifactMetaRepository as NoopArtifactMeta,
};
use batlehub_core::{
    entities::{AccessEvent, PackageId, PackageStatus, Role},
    error::CoreError,
    ports::{
        AuthProvider, CacheStore, PackageRepository, RegistryClient, StorageBackend, TokenOwner,
        UserToken, UserTokenRepository,
    },
    services::{new_hot_lock, AdminService, HotConfig, ProxyMetrics, ProxyService, RegistryPolicy},
};
use batlehub_web::RegistryModeMap;
use uuid::Uuid;

// ── InMemoryTokenRepository ───────────────────────────────────────────────────

struct InMemoryTokenRepository {
    tokens: Mutex<Vec<UserToken>>,
}

impl InMemoryTokenRepository {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            tokens: Mutex::new(vec![]),
        })
    }
}

#[async_trait]
impl UserTokenRepository for InMemoryTokenRepository {
    async fn create_token(
        &self,
        id: Uuid,
        owner: &TokenOwner,
        name: &str,
        _token_hash: &str,
        role: Role,
        expires_at: chrono::DateTime<Utc>,
    ) -> Result<UserToken, CoreError> {
        // Names are unique per *principal*, and a principal is (provider,
        // user_id) — matching the `uq_user_token_name` index in Postgres.
        let mut tokens = self.tokens.lock().unwrap();
        if tokens
            .iter()
            .any(|t| owns(t, owner) && t.name == name && t.revoked_at.is_none())
        {
            return Err(CoreError::Conflict(format!(
                "a token named '{}' already exists",
                name
            )));
        }
        let tok = UserToken {
            id,
            user_id: owner.user_id.clone(),
            provider: owner.provider.clone(),
            name: name.to_owned(),
            role,
            expires_at,
            created_at: Utc::now(),
            revoked_at: None,
            last_used_at: None,
        };
        tokens.push(tok);
        Ok(tokens.last().unwrap().clone_token())
    }

    async fn find_by_hash(&self, _token_hash: &str) -> Result<Option<UserToken>, CoreError> {
        Ok(None)
    }

    async fn list_for_user(&self, owner: &TokenOwner) -> Result<Vec<UserToken>, CoreError> {
        let tokens = self.tokens.lock().unwrap();
        Ok(tokens
            .iter()
            .filter(|t| owns(t, owner) && t.revoked_at.is_none())
            .map(|t| t.clone_token())
            .collect())
    }

    async fn touch_last_used(&self, _id: Uuid) -> Result<(), CoreError> {
        Ok(())
    }

    async fn revoke(&self, id: Uuid, owner: &TokenOwner) -> Result<bool, CoreError> {
        let mut tokens = self.tokens.lock().unwrap();
        for t in tokens.iter_mut() {
            if t.id == id && owns(t, owner) && t.revoked_at.is_none() {
                t.revoked_at = Some(Utc::now());
                return Ok(true);
            }
        }
        Ok(false)
    }
}

/// Ownership is both halves. Matching on `user_id` alone is exactly the bug the
/// `provider` column exists to close, so the test double must not take the
/// shortcut either.
fn owns(token: &UserToken, owner: &TokenOwner) -> bool {
    token.user_id == owner.user_id && token.provider == owner.provider
}

// UserToken doesn't derive Clone; add a helper method instead.
trait CloneToken {
    fn clone_token(&self) -> UserToken;
}

impl CloneToken for UserToken {
    fn clone_token(&self) -> UserToken {
        UserToken {
            id: self.id,
            user_id: self.user_id.clone(),
            provider: self.provider.clone(),
            name: self.name.clone(),
            role: self.role.clone(),
            expires_at: self.expires_at,
            created_at: self.created_at,
            revoked_at: self.revoked_at,
            last_used_at: self.last_used_at,
        }
    }
}

// ── OIDC-style test auth provider ─────────────────────────────────────────────
// The token endpoint only accepts identities coming from a *configured* OIDC
// provider. StaticTokenAuthProvider sets "static-token", so we use a thin
// wrapper — parameterised by name, because the provider name is operator-chosen
// (`name = "authentik"`) and the endpoint must not hardcode any one value.

use batlehub_core::ports::RawAuthRequest;

const OIDC_USER_TOKEN: &str = "oidc-user-token";
const OIDC_ADMIN_TOKEN: &str = "oidc-admin-token";

/// The name the OIDC-style provider reports for most of this suite. Deliberately
/// *not* `"oidc"`: a renamed provider is the common deployment, and the endpoint
/// used to 403 every one of them.
const OIDC_PROVIDER_NAME: &str = "authentik";

struct OidcStyleAuthProvider(&'static str);

#[async_trait]
impl AuthProvider for OidcStyleAuthProvider {
    fn name(&self) -> &str {
        self.0
    }

    async fn authenticate(
        &self,
        req: &RawAuthRequest,
    ) -> Result<Option<batlehub_core::entities::Identity>, CoreError> {
        use batlehub_core::entities::Identity;
        let auth = req
            .headers
            .get("authorization")
            .or_else(|| req.headers.get("Authorization"))
            .and_then(|v| v.strip_prefix("Bearer "));
        match auth {
            Some(OIDC_USER_TOKEN) => Ok(Some(Identity {
                user_id: Some("oidc-user".to_owned()),
                role: Role::User,
                auth_provider: Some(self.0.to_owned()),
                groups: vec![],
            })),
            Some(OIDC_ADMIN_TOKEN) => Ok(Some(Identity {
                user_id: Some("oidc-admin".to_owned()),
                role: Role::Admin,
                auth_provider: Some(self.0.to_owned()),
                groups: vec![],
            })),
            _ => Ok(None),
        }
    }
}

/// Build an app wired with both static + OIDC-style providers and an in-memory token repo.
async fn make_app_with_tokens(
    repo: Arc<InMemoryRepo>,
    token_repo: Arc<InMemoryTokenRepository>,
) -> impl actix_web::dev::Service<
    actix_http::Request,
    Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
    Error = actix_web::Error,
> {
    make_app_with_oidc_provider(repo, token_repo, OIDC_PROVIDER_NAME, &[OIDC_PROVIDER_NAME]).await
}

/// As [`make_app_with_tokens`], but the caller chooses what the OIDC-style
/// provider calls itself and which names the server has configured. The two
/// differ in the negative tests: an identity claiming a provider the server does
/// not know must not be able to mint a token.
async fn make_app_with_oidc_provider(
    repo: Arc<InMemoryRepo>,
    token_repo: Arc<InMemoryTokenRepository>,
    provider_name: &'static str,
    configured_names: &[&str],
) -> impl actix_web::dev::Service<
    actix_http::Request,
    Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
    Error = actix_web::Error,
> {
    let repo_dyn: Arc<dyn PackageRepository> = repo.clone();
    let storage: Arc<dyn StorageBackend> = InMemoryStorage::new();
    let cache: Arc<dyn CacheStore> = Arc::new(InMemoryCacheStore::new());

    let registries: HashMap<String, Arc<dyn RegistryClient>> = [(
        "npm".to_owned(),
        FixedRegistry::new("npm") as Arc<dyn RegistryClient>,
    )]
    .into();
    let policies: HashMap<String, Arc<RegistryPolicy>> =
        [("npm".to_owned(), Arc::new(rbac_policy(repo_dyn.clone())))].into();

    let hot = new_hot_lock(HotConfig {
        registries,
        policies,
        ..Default::default()
    });
    let local_svc = make_local_svc(hot.clone(), storage.clone());
    let proxy_svc = Arc::new(ProxyService {
        hot: hot.clone(),
        storage,
        cache,
        repo: repo_dyn.clone(),
        artifact_meta: NoopArtifactMeta::arc(),
        metrics: Arc::new(ProxyMetrics::new(&[])),
        sbom: None,
        readme: None,
        discovery: Default::default(),
    });
    let admin_svc = Arc::new(AdminService::new(repo_dyn));
    let tok_repo: Arc<dyn UserTokenRepository> = token_repo;
    let access_config = access_config_for(&["npm"]);
    let registry_map = registry_map_for(&[("npm", "npm")]);
    let cargo_indexes = batlehub_web::CargoIndexMap::default();

    let providers: Vec<Arc<dyn AuthProvider>> = vec![
        Arc::new(StaticTokenAuthProvider::new([
            (
                ADMIN_TOKEN.to_owned(),
                Some("admin".to_owned()),
                Role::Admin,
            ),
            (USER_TOKEN.to_owned(), Some("user-1".to_owned()), Role::User),
        ])),
        Arc::new(OidcStyleAuthProvider(provider_name)),
    ];

    finish_test_app(
        proxy_svc,
        admin_svc,
        tok_repo,
        access_config,
        registry_map,
        local_svc,
        RegistryModeMap::default(),
        cargo_indexes,
        ConfigureAppDefaults {
            oidc_provider_names: batlehub_web::OidcProviderNames::new(
                configured_names.iter().copied(),
            ),
            ..Default::default()
        },
        providers,
    )
    .await
}

// ── Token API tests ───────────────────────────────────────────────────────────

#[actix_web::test]
async fn create_token_returns_403_for_anonymous() {
    let app = make_app_with_tokens(InMemoryRepo::new(), InMemoryTokenRepository::new()).await;
    let req = TestRequest::post()
        .uri("/api/v1/auth/tokens")
        .set_json(serde_json::json!({"name": "ci", "expires_in_days": 7, "role": "user"}))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 403);
}

#[actix_web::test]
async fn create_token_returns_403_for_static_token_user() {
    // Static token provider sets auth_provider = "static-token", not "oidc"
    let app = make_app_with_tokens(InMemoryRepo::new(), InMemoryTokenRepository::new()).await;
    let req = TestRequest::post()
        .uri("/api/v1/auth/tokens")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_json(serde_json::json!({"name": "ci", "expires_in_days": 7, "role": "user"}))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 403);
}

#[actix_web::test]
async fn create_token_succeeds_for_oidc_user() {
    let app = make_app_with_tokens(InMemoryRepo::new(), InMemoryTokenRepository::new()).await;
    let req = TestRequest::post()
        .uri("/api/v1/auth/tokens")
        .insert_header(("Authorization", bearer(OIDC_USER_TOKEN)))
        .set_json(serde_json::json!({"name": "ci-token", "expires_in_days": 30, "role": "user"}))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 201);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["name"], "ci-token");
    assert!(body["token"].is_string(), "raw token should be returned");
}

// ── Provider-name matching (regression) ───────────────────────────────────────
// `create_token` used to compare `auth_provider` against the literal `"oidc"`.
// The name is operator-chosen and defaults to `"oidc"` only when unset, so every
// deployment that wrote `name = "authentik"` — or configured two providers — got
// a blanket 403 with no explanation, and fell back to the non-expiring static
// tokens in config.toml. The whole suite above now runs on a provider named
// "authentik"; these three pin the edges.

#[actix_web::test]
async fn create_token_succeeds_for_a_second_configured_oidc_provider() {
    // Two providers configured, the caller arrives through the second one.
    let app = make_app_with_oidc_provider(
        InMemoryRepo::new(),
        InMemoryTokenRepository::new(),
        "keycloak",
        &["authentik", "keycloak"],
    )
    .await;
    let req = TestRequest::post()
        .uri("/api/v1/auth/tokens")
        .insert_header(("Authorization", bearer(OIDC_USER_TOKEN)))
        .set_json(serde_json::json!({"name": "ci-token", "expires_in_days": 30, "role": "user"}))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(
        resp.status(),
        201,
        "a caller from any configured OIDC provider may mint a token"
    );
}

#[actix_web::test]
async fn create_token_rejects_provider_name_the_server_did_not_configure() {
    // The fix is an allow-list, not a rename: an identity reporting a provider
    // the server has no `[[auth]] type = "oidc"` entry for is still refused —
    // including one that calls itself the historical "oidc".
    let app = make_app_with_oidc_provider(
        InMemoryRepo::new(),
        InMemoryTokenRepository::new(),
        "oidc",
        &["authentik"],
    )
    .await;
    let req = TestRequest::post()
        .uri("/api/v1/auth/tokens")
        .insert_header(("Authorization", bearer(OIDC_USER_TOKEN)))
        .set_json(serde_json::json!({"name": "ci-token", "expires_in_days": 30, "role": "user"}))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 403);
}

#[actix_web::test]
async fn create_token_rejects_everyone_when_no_oidc_provider_is_configured() {
    // Absent configuration denies rather than admits.
    let app = make_app_with_oidc_provider(
        InMemoryRepo::new(),
        InMemoryTokenRepository::new(),
        "authentik",
        &[],
    )
    .await;
    let req = TestRequest::post()
        .uri("/api/v1/auth/tokens")
        .insert_header(("Authorization", bearer(OIDC_USER_TOKEN)))
        .set_json(serde_json::json!({"name": "ci-token", "expires_in_days": 30, "role": "user"}))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 403);
}

// ── Ownership is (provider, user_id) ──────────────────────────────────────────
// `user_id` is a bare string each provider picks for itself, so it is not an
// identity on its own. Listing and revoking used to match on it alone, and
// accepted any authenticated caller — so a static `[[auth.tokens]]` entry with
// `user_id = "oidc-user"`, or a service account whose username happened to equal
// an OIDC `sub`, could enumerate and destroy that person's tokens.

#[actix_web::test]
async fn tokens_are_invisible_to_the_same_user_id_from_another_provider() {
    let token_repo = InMemoryTokenRepository::new();
    let app = make_app_with_oidc_provider(
        InMemoryRepo::new(),
        token_repo.clone(),
        "authentik",
        // Both providers are legitimate OIDC providers here — the point is that
        // the *same* user_id under a different one is a different principal.
        &["authentik", "keycloak"],
    )
    .await;

    let create = TestRequest::post()
        .uri("/api/v1/auth/tokens")
        .insert_header(("Authorization", bearer(OIDC_USER_TOKEN)))
        .set_json(serde_json::json!({"name": "mine", "expires_in_days": 7, "role": "user"}))
        .to_request();
    assert_eq!(call_service(&app, create).await.status(), 201);

    // Same user_id ("oidc-user"), different provider.
    let other = make_app_with_oidc_provider(
        InMemoryRepo::new(),
        token_repo.clone(),
        "keycloak",
        &["authentik", "keycloak"],
    )
    .await;
    let list = TestRequest::get()
        .uri("/api/v1/auth/tokens")
        .insert_header(("Authorization", bearer(OIDC_USER_TOKEN)))
        .to_request();
    let body: Value = read_body_json(call_service(&other, list).await).await;
    assert_eq!(
        body.as_array().unwrap().len(),
        0,
        "the same user_id from another provider is a different principal"
    );
}

#[actix_web::test]
async fn a_token_cannot_be_revoked_from_another_provider() {
    let token_repo = InMemoryTokenRepository::new();
    let app = make_app_with_oidc_provider(
        InMemoryRepo::new(),
        token_repo.clone(),
        "authentik",
        &["authentik", "keycloak"],
    )
    .await;

    let create = TestRequest::post()
        .uri("/api/v1/auth/tokens")
        .insert_header(("Authorization", bearer(OIDC_USER_TOKEN)))
        .set_json(serde_json::json!({"name": "mine", "expires_in_days": 7, "role": "user"}))
        .to_request();
    let created: Value = read_body_json(call_service(&app, create).await).await;
    let id = created["id"].as_str().unwrap();

    let other = make_app_with_oidc_provider(
        InMemoryRepo::new(),
        token_repo.clone(),
        "keycloak",
        &["authentik", "keycloak"],
    )
    .await;
    let revoke = TestRequest::delete()
        .uri(&format!("/api/v1/auth/tokens/{id}"))
        .insert_header(("Authorization", bearer(OIDC_USER_TOKEN)))
        .to_request();
    assert_eq!(call_service(&other, revoke).await.status(), 404);

    // And the owner can still revoke it, so the scoping did not simply break.
    let mine = TestRequest::delete()
        .uri(&format!("/api/v1/auth/tokens/{id}"))
        .insert_header(("Authorization", bearer(OIDC_USER_TOKEN)))
        .to_request();
    assert_eq!(call_service(&app, mine).await.status(), 204);
}

#[actix_web::test]
async fn a_static_token_session_cannot_list_or_revoke() {
    // Managing tokens takes an interactive login. A leaked machine credential
    // must not be able to enumerate its victim's other tokens, nor revoke them.
    let app = make_app_with_tokens(InMemoryRepo::new(), InMemoryTokenRepository::new()).await;
    for req in [
        TestRequest::get()
            .uri("/api/v1/auth/tokens")
            .insert_header(("Authorization", bearer(USER_TOKEN)))
            .to_request(),
        TestRequest::delete()
            .uri(&format!("/api/v1/auth/tokens/{}", Uuid::new_v4()))
            .insert_header(("Authorization", bearer(USER_TOKEN)))
            .to_request(),
    ] {
        assert_eq!(call_service(&app, req).await.status(), 403);
    }
}

#[actix_web::test]
async fn the_same_name_is_free_for_a_different_provider() {
    // Uniqueness is per principal, matching the `uq_user_token_name` index.
    let token_repo = InMemoryTokenRepository::new();
    let body = serde_json::json!({"name": "ci", "expires_in_days": 7, "role": "user"});

    for provider in ["authentik", "keycloak"] {
        let app = make_app_with_oidc_provider(
            InMemoryRepo::new(),
            token_repo.clone(),
            // `make_app_with_oidc_provider` takes &'static str, so match on it.
            if provider == "authentik" {
                "authentik"
            } else {
                "keycloak"
            },
            &["authentik", "keycloak"],
        )
        .await;
        let req = TestRequest::post()
            .uri("/api/v1/auth/tokens")
            .insert_header(("Authorization", bearer(OIDC_USER_TOKEN)))
            .set_json(&body)
            .to_request();
        assert_eq!(
            call_service(&app, req).await.status(),
            201,
            "'ci' must be free under {provider}"
        );
    }
}

#[actix_web::test]
async fn create_token_rejects_zero_days() {
    let app = make_app_with_tokens(InMemoryRepo::new(), InMemoryTokenRepository::new()).await;
    let req = TestRequest::post()
        .uri("/api/v1/auth/tokens")
        .insert_header(("Authorization", bearer(OIDC_USER_TOKEN)))
        .set_json(serde_json::json!({"name": "bad", "expires_in_days": 0, "role": "user"}))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 400);
}

#[actix_web::test]
async fn create_token_rejects_91_days() {
    let app = make_app_with_tokens(InMemoryRepo::new(), InMemoryTokenRepository::new()).await;
    let req = TestRequest::post()
        .uri("/api/v1/auth/tokens")
        .insert_header(("Authorization", bearer(OIDC_USER_TOKEN)))
        .set_json(serde_json::json!({"name": "bad", "expires_in_days": 91, "role": "user"}))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 400);
}

#[actix_web::test]
async fn create_token_rejects_empty_name() {
    let app = make_app_with_tokens(InMemoryRepo::new(), InMemoryTokenRepository::new()).await;
    let req = TestRequest::post()
        .uri("/api/v1/auth/tokens")
        .insert_header(("Authorization", bearer(OIDC_USER_TOKEN)))
        .set_json(serde_json::json!({"name": "   ", "expires_in_days": 7, "role": "user"}))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 400);
}

#[actix_web::test]
async fn create_token_rejects_invalid_role() {
    let app = make_app_with_tokens(InMemoryRepo::new(), InMemoryTokenRepository::new()).await;
    let req = TestRequest::post()
        .uri("/api/v1/auth/tokens")
        .insert_header(("Authorization", bearer(OIDC_USER_TOKEN)))
        .set_json(serde_json::json!({"name": "t", "expires_in_days": 7, "role": "superadmin"}))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 400);
}

#[actix_web::test]
async fn create_token_user_cannot_escalate_to_admin_role() {
    let app = make_app_with_tokens(InMemoryRepo::new(), InMemoryTokenRepository::new()).await;
    let req = TestRequest::post()
        .uri("/api/v1/auth/tokens")
        .insert_header(("Authorization", bearer(OIDC_USER_TOKEN)))
        .set_json(serde_json::json!({"name": "escalate", "expires_in_days": 7, "role": "admin"}))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 403);
}

#[actix_web::test]
async fn list_tokens_returns_created_tokens() {
    let tok_repo = InMemoryTokenRepository::new();
    let app = make_app_with_tokens(InMemoryRepo::new(), tok_repo).await;

    // Create a token
    let create_req = TestRequest::post()
        .uri("/api/v1/auth/tokens")
        .insert_header(("Authorization", bearer(OIDC_USER_TOKEN)))
        .set_json(serde_json::json!({"name": "my-token", "expires_in_days": 7, "role": "user"}))
        .to_request();
    let create_resp = call_service(&app, create_req).await;
    assert_eq!(create_resp.status(), 201);

    // List tokens
    let list_req = TestRequest::get()
        .uri("/api/v1/auth/tokens")
        .insert_header(("Authorization", bearer(OIDC_USER_TOKEN)))
        .to_request();
    let list_resp = call_service(&app, list_req).await;
    assert_eq!(list_resp.status(), 200);
    let body: Value = read_body_json(list_resp).await;
    let items = body.as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "my-token");
}

#[actix_web::test]
async fn revoke_token_returns_204() {
    let tok_repo = InMemoryTokenRepository::new();
    let app = make_app_with_tokens(InMemoryRepo::new(), tok_repo).await;

    let create_req = TestRequest::post()
        .uri("/api/v1/auth/tokens")
        .insert_header(("Authorization", bearer(OIDC_USER_TOKEN)))
        .set_json(serde_json::json!({"name": "to-revoke", "expires_in_days": 7, "role": "user"}))
        .to_request();
    let create_resp = call_service(&app, create_req).await;
    assert_eq!(create_resp.status(), 201);
    let created: Value = read_body_json(create_resp).await;
    let id = created["id"].as_str().unwrap();

    let revoke_req = TestRequest::delete()
        .uri(&format!("/api/v1/auth/tokens/{id}"))
        .insert_header(("Authorization", bearer(OIDC_USER_TOKEN)))
        .to_request();
    let revoke_resp = call_service(&app, revoke_req).await;
    assert_eq!(revoke_resp.status(), 204);
}

#[actix_web::test]
async fn revoke_nonexistent_token_returns_404() {
    let app = make_app_with_tokens(InMemoryRepo::new(), InMemoryTokenRepository::new()).await;
    let fake_id = Uuid::new_v4();
    let req = TestRequest::delete()
        .uri(&format!("/api/v1/auth/tokens/{fake_id}"))
        .insert_header(("Authorization", bearer(OIDC_USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 404);
}

#[actix_web::test]
async fn duplicate_token_name_returns_conflict() {
    let tok_repo = InMemoryTokenRepository::new();
    let app = make_app_with_tokens(InMemoryRepo::new(), tok_repo).await;

    for _ in 0..2 {
        let req = TestRequest::post()
            .uri("/api/v1/auth/tokens")
            .insert_header(("Authorization", bearer(OIDC_USER_TOKEN)))
            .set_json(serde_json::json!({"name": "dup", "expires_in_days": 7, "role": "user"}))
            .to_request();
        let _ = call_service(&app, req).await;
    }

    let req = TestRequest::post()
        .uri("/api/v1/auth/tokens")
        .insert_header(("Authorization", bearer(OIDC_USER_TOKEN)))
        .set_json(serde_json::json!({"name": "dup", "expires_in_days": 7, "role": "user"}))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 409);
}

// ── Pagination / Filtering tests ──────────────────────────────────────────────

#[actix_web::test]
async fn admin_packages_list_blocked_only_filter() {
    let repo = InMemoryRepo::new();

    let available = PackageId::new("npm", "lodash", "4.17.21");
    let blocked = PackageId::new("npm", "evil-pkg", "1.0.0");

    repo.record_access(AccessEvent::allowed_download(
        available,
        Some("u".to_owned()),
        Role::User,
    ))
    .await
    .unwrap();
    repo.set_status(
        &blocked,
        PackageStatus::Blocked {
            reason: "vuln".to_owned(),
            blocked_by: "admin".to_owned(),
            blocked_at: Utc::now(),
        },
    )
    .await
    .unwrap();

    let app = make_app(repo).await;
    let req = TestRequest::get()
        .uri("/api/v1/admin/packages?blocked_only=true")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    let items = body["items"].as_array().unwrap();
    assert!(
        items.iter().all(|i| i["status"]["status"] == "blocked"),
        "only blocked packages expected"
    );
}

#[actix_web::test]
async fn audit_log_denied_only_filter() {
    let repo = InMemoryRepo::new();
    let app = make_app(repo.clone()).await;

    // Cause a denied event (anonymous accessing tarball = source:read denied)
    let req = TestRequest::get()
        .uri("/proxy/npm/lodash/4.17.21/tarball")
        .to_request();
    let _ = call_service(&app, req).await;

    // Also cause an allowed event
    let req = TestRequest::get().uri("/proxy/npm/lodash").to_request();
    let _ = call_service(&app, req).await;

    let audit_req = TestRequest::get()
        .uri("/api/v1/admin/audit-log?denied_only=true")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let resp = call_service(&app, audit_req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    let events = body["items"].as_array().unwrap();
    assert!(!events.is_empty(), "at least one denied event expected");
    assert!(
        events.iter().all(|e| e["result"]["outcome"] == "denied"),
        "only denied events expected"
    );
}

#[actix_web::test]
async fn registries_endpoint_returns_list_for_anonymous() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get().uri("/api/v1/registries").to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    let registries = body.as_array().unwrap();
    // Anonymous has access to github, npm, cargo in make_app
    assert!(!registries.is_empty(), "should see at least one registry");
    let names: Vec<&str> = registries
        .iter()
        .filter_map(|r| r["name"].as_str())
        .collect();
    assert!(names.contains(&"npm"));
}

#[actix_web::test]
async fn registries_endpoint_returns_200_for_admin() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/api/v1/registries")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    let registries = body.as_array().unwrap();
    assert!(registries.len() >= 3, "admin should see github, npm, cargo");
}
