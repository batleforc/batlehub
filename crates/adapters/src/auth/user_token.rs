use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use rand::Rng;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use batlehub_core::{
    entities::Identity,
    error::CoreError,
    ports::{AuthProvider, RawAuthRequest, UserToken, UserTokenRepository},
};

/// Marks a string as a BatleHub personal access token.
///
/// Secret scanners find credentials by shape. 64 bare hex characters have no
/// shape — they are indistinguishable from a commit hash, a checksum or a UUID
/// with the dashes stripped — so this project's own gitleaks job could not see
/// its own tokens leak. A distinctive prefix is what makes a leaked token
/// findable, by the scanners here and by GitHub's.
///
/// The prefix is *not* a secret and adds no entropy: the 32 random bytes after
/// it are the whole strength.
pub const TOKEN_PREFIX: &str = "bh_pat_";

/// Mint a token: `(raw value to show once, SHA-256 of it for storage)`.
pub fn generate_token() -> (String, String) {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    let raw = format!("{TOKEN_PREFIX}{}", hex::encode(bytes));
    let hash = hash_token(&raw);
    (raw, hash)
}

/// Hash a token exactly as presented, prefix included.
///
/// Hashing the whole string rather than stripping the prefix first keeps one
/// rule for tokens minted before and after the prefix existed: whatever the user
/// pastes is what gets hashed, so pre-prefix tokens keep working untouched and
/// there is no branch that could disagree with itself.
pub fn hash_token(raw: &str) -> String {
    hex::encode(Sha256::digest(raw.as_bytes()))
}

/// How long a token's `last_used_at` is left alone after a write.
///
/// Long enough that a busy token costs one write a minute rather than one per
/// request; short enough that the column still answers "was this token used
/// today?" — which is the only question it exists for.
const LAST_USED_THROTTLE: Duration = Duration::from_secs(60);

pub struct UserTokenAuthProvider {
    repo: Arc<dyn UserTokenRepository>,
    /// Last time each token's use was written, so a token presented on every
    /// request does not turn every read into a write.
    ///
    /// Process-local by design: with several replicas each writes at most once
    /// per minute per token, and the `WHERE` clause in `touch_last_used` makes
    /// the overlap a no-op. Bounded by the number of *live* tokens seen by this
    /// process, which is bounded by what the deployment has issued.
    last_recorded: Mutex<HashMap<Uuid, Instant>>,
}

impl UserTokenAuthProvider {
    pub fn new(repo: Arc<dyn UserTokenRepository>) -> Self {
        Self {
            repo,
            last_recorded: Mutex::new(HashMap::new()),
        }
    }

    /// Whether enough time has passed to write `last_used_at` for `id` again.
    /// Claims the slot as it answers, so concurrent requests produce one write.
    fn should_record_use(&self, id: Uuid) -> bool {
        let now = Instant::now();
        let mut seen = self.last_recorded.lock().expect("last-used mutex");
        match seen.get(&id) {
            Some(at) if now.duration_since(*at) < LAST_USED_THROTTLE => false,
            _ => {
                seen.insert(id, now);
                true
            }
        }
    }
}

#[async_trait]
impl AuthProvider for UserTokenAuthProvider {
    fn name(&self) -> &str {
        "user-token"
    }

    async fn authenticate(&self, req: &RawAuthRequest) -> Result<Option<Identity>, CoreError> {
        let Some(raw) = req.bearer_token() else {
            return Ok(None);
        };

        // Fast path: OIDC JWTs contain dots; our hex tokens never do.
        if raw.contains('.') {
            return Ok(None);
        }

        let hash = hash_token(raw);
        match self.repo.find_by_hash(&hash).await? {
            None => Ok(None),
            Some(tok) if self.should_record_use(tok.id) => {
                // Best-effort: a token that is valid stays valid even if the
                // bookkeeping write fails. Awaited rather than spawned so a slow
                // database applies back-pressure here instead of queueing an
                // unbounded pile of writes behind the pool — the in-process
                // throttle above is what keeps this off the hot path.
                if let Err(e) = self.repo.touch_last_used(tok.id).await {
                    tracing::debug!(error = %e, "recording token last-used failed");
                }
                Ok(Some(to_identity(tok)))
            }
            Some(tok) => Ok(Some(to_identity(tok))),
        }
    }
}

/// `auth_provider` is `"user-token"`, not the provider that minted the token.
///
/// That is what the token endpoints key on to refuse a PAT session: reporting
/// the minting provider here would let a PAT mint another PAT and revoke its
/// siblings.
///
/// `groups` is the snapshot taken at creation (RFC 0011-bis §4.4), never a live
/// lookup: a PAT has no session to re-resolve from. The subset invariant was
/// decided by `snapshot_pat_groups` when the row was written, so nothing here
/// can widen it — this reads the column and stops.
fn to_identity(tok: UserToken) -> Identity {
    Identity {
        user_id: Some(tok.user_id),
        role: tok.role,
        auth_provider: Some("user-token".to_owned()),
        groups: tok.groups,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use batlehub_core::{
        entities::Role,
        error::CoreError,
        ports::{RawAuthRequest, TokenOwner, UserToken, UserTokenRepository},
    };
    use chrono::DateTime;
    use std::collections::HashMap;
    use std::sync::Arc;

    fn req(auth: &str) -> RawAuthRequest {
        RawAuthRequest {
            headers: HashMap::from([("authorization".to_owned(), auth.to_owned())]),
            query_params: HashMap::new(),
        }
    }

    fn no_auth_req() -> RawAuthRequest {
        RawAuthRequest {
            headers: HashMap::new(),
            query_params: HashMap::new(),
        }
    }

    struct StubRepo(Option<UserToken>);

    fn stub_token() -> UserToken {
        UserToken {
            id: uuid::Uuid::new_v4(),
            user_id: "carol".to_owned(),
            provider: "authentik".to_owned(),
            name: "test-token".to_owned(),
            role: Role::User,
            created_at: chrono::Utc::now(),
            expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
            revoked_at: None,
            last_used_at: None,
            groups: vec!["oidc1:eng".to_owned()],
        }
    }

    #[async_trait]
    impl UserTokenRepository for StubRepo {
        async fn create_token(
            &self,
            _: uuid::Uuid,
            _: &TokenOwner,
            _: &str,
            _: &str,
            _: Role,
            _: DateTime<chrono::Utc>,
            _: &[String],
        ) -> Result<UserToken, CoreError> {
            Ok(stub_token())
        }
        async fn find_by_hash(&self, _: &str) -> Result<Option<UserToken>, CoreError> {
            Ok(self.0.as_ref().map(|t| UserToken {
                id: t.id,
                user_id: t.user_id.clone(),
                provider: t.provider.clone(),
                name: t.name.clone(),
                role: t.role.clone(),
                created_at: t.created_at,
                expires_at: t.expires_at,
                revoked_at: t.revoked_at,
                last_used_at: t.last_used_at,
                groups: t.groups.clone(),
            }))
        }
        async fn list_for_user(&self, _: &TokenOwner) -> Result<Vec<UserToken>, CoreError> {
            Ok(vec![])
        }
        async fn touch_last_used(&self, _: uuid::Uuid) -> Result<(), CoreError> {
            Ok(())
        }
        async fn revoke(&self, _: uuid::Uuid, _: &TokenOwner) -> Result<bool, CoreError> {
            Ok(true)
        }
    }

    #[test]
    fn generate_token_produces_unique_values() {
        let (t1, h1) = generate_token();
        let (t2, h2) = generate_token();
        assert_ne!(t1, t2);
        assert_ne!(h1, h2);
    }

    #[test]
    fn hash_token_is_deterministic() {
        assert_eq!(hash_token("hello"), hash_token("hello"));
        assert_ne!(hash_token("hello"), hash_token("world"));
    }

    #[tokio::test]
    async fn no_auth_header_returns_none() {
        let p = UserTokenAuthProvider::new(Arc::new(StubRepo(None)));
        assert!(p.authenticate(&no_auth_req()).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn non_bearer_header_returns_none() {
        let p = UserTokenAuthProvider::new(Arc::new(StubRepo(None)));
        assert!(p
            .authenticate(&req("Basic dXNlcjpwYXNz"))
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn jwt_dot_in_token_short_circuits_without_repo_call() {
        // Repo would return a token, but the JWT detection must bypass it.
        let p = UserTokenAuthProvider::new(Arc::new(StubRepo(Some(stub_token()))));
        let result = p
            .authenticate(&req("Bearer header.payload.sig"))
            .await
            .unwrap();
        assert!(
            result.is_none(),
            "JWT tokens must not be looked up in the repo"
        );
    }

    #[tokio::test]
    async fn valid_hex_token_returns_identity() {
        let p = UserTokenAuthProvider::new(Arc::new(StubRepo(Some(stub_token()))));
        let id = p
            .authenticate(&req(
                "Bearer abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890",
            ))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(id.user_id.as_deref(), Some("carol"));
        assert_eq!(id.role, Role::User);
    }

    /// RFC 0011-bis §4.4 / G1. This returned `groups: vec![]` for every token,
    /// so RFC 0015's `group:` subjects could never match a PAT and automation
    /// read as an authenticated user belonging to no team.
    #[tokio::test]
    async fn a_token_resolves_to_the_groups_it_was_minted_with() {
        let p = UserTokenAuthProvider::new(Arc::new(StubRepo(Some(stub_token()))));
        let id = p
            .authenticate(&req(
                "Bearer abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890",
            ))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(id.groups, vec!["oidc1:eng".to_owned()]);
    }

    #[tokio::test]
    async fn lowercase_bearer_prefix_returns_identity() {
        // Sibling auth providers (oidc, kubernetes, static-token, actions-oidc)
        // accept a lowercase "bearer " prefix via RawAuthRequest::bearer_token;
        // user-token must match, not silently reject the request.
        let p = UserTokenAuthProvider::new(Arc::new(StubRepo(Some(stub_token()))));
        let id = p
            .authenticate(&req(
                "bearer abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890",
            ))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(id.user_id.as_deref(), Some("carol"));
    }

    #[tokio::test]
    async fn unknown_token_returns_none() {
        let p = UserTokenAuthProvider::new(Arc::new(StubRepo(None)));
        let result = p
            .authenticate(&req(
                "Bearer abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890",
            ))
            .await
            .unwrap();
        assert!(result.is_none());
    }
}

#[cfg(test)]
mod prefix_and_usage_tests {
    use super::*;
    use batlehub_core::{
        entities::Role,
        ports::{TokenOwner, UserToken},
    };
    use std::collections::HashMap as Map;
    use std::sync::atomic::{AtomicUsize, Ordering};

    // ── Token prefix ──────────────────────────────────────────────────────────
    // 64 bare hex characters look like a commit hash. This repository runs
    // gitleaks over its own history and could not have recognised its own
    // tokens; a prefix is what makes a leaked one findable.

    #[test]
    fn a_minted_token_is_prefixed() {
        let (raw, _) = generate_token();
        assert!(raw.starts_with(TOKEN_PREFIX), "got: {raw}");
        assert_eq!(
            raw.len(),
            TOKEN_PREFIX.len() + 64,
            "the prefix adds no entropy — the 32 random bytes are unchanged"
        );
        assert!(
            raw[TOKEN_PREFIX.len()..]
                .chars()
                .all(|c| c.is_ascii_hexdigit()),
            "got: {raw}"
        );
    }

    #[test]
    fn the_prefix_is_part_of_what_is_hashed() {
        // Whatever the user pastes is what gets hashed, so a token minted before
        // the prefix existed keeps working with no branch to get wrong.
        let (raw, hash) = generate_token();
        assert_eq!(hash, hash_token(&raw));
        assert_ne!(
            hash,
            hash_token(raw.strip_prefix(TOKEN_PREFIX).unwrap()),
            "the stored hash covers the whole string"
        );
    }

    #[test]
    fn a_token_minted_before_the_prefix_still_hashes() {
        let legacy = "a".repeat(64);
        assert_eq!(hash_token(&legacy), hash_token(&legacy));
    }

    // ── last_used_at ──────────────────────────────────────────────────────────

    struct CountingRepo {
        touches: AtomicUsize,
        id: uuid::Uuid,
    }

    impl CountingRepo {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                touches: AtomicUsize::new(0),
                id: uuid::Uuid::new_v4(),
            })
        }
    }

    #[async_trait]
    impl UserTokenRepository for CountingRepo {
        async fn create_token(
            &self,
            _: uuid::Uuid,
            _: &TokenOwner,
            _: &str,
            _: &str,
            _: Role,
            _: chrono::DateTime<chrono::Utc>,
            _: &[String],
        ) -> Result<UserToken, CoreError> {
            unreachable!()
        }
        async fn find_by_hash(&self, _: &str) -> Result<Option<UserToken>, CoreError> {
            Ok(Some(UserToken {
                id: self.id,
                user_id: "carol".to_owned(),
                provider: "authentik".to_owned(),
                name: "t".to_owned(),
                role: Role::User,
                created_at: chrono::Utc::now(),
                expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
                revoked_at: None,
                last_used_at: None,
                groups: vec![],
            }))
        }
        async fn list_for_user(&self, _: &TokenOwner) -> Result<Vec<UserToken>, CoreError> {
            Ok(vec![])
        }
        async fn touch_last_used(&self, _: uuid::Uuid) -> Result<(), CoreError> {
            self.touches.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        async fn revoke(&self, _: uuid::Uuid, _: &TokenOwner) -> Result<bool, CoreError> {
            Ok(false)
        }
    }

    fn req(auth: &str) -> RawAuthRequest {
        RawAuthRequest {
            headers: Map::from([("authorization".to_owned(), auth.to_owned())]),
            query_params: Map::new(),
        }
    }

    #[tokio::test]
    async fn the_first_use_is_recorded() {
        let repo = CountingRepo::new();
        let p = UserTokenAuthProvider::new(repo.clone());
        p.authenticate(&req(&format!("Bearer {}", "a".repeat(64))))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(repo.touches.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn repeated_use_within_the_window_writes_once() {
        // A token presented on every request must not turn every read into a
        // write — this is the difference between one write a minute and one per
        // artifact download.
        let repo = CountingRepo::new();
        let p = UserTokenAuthProvider::new(repo.clone());
        let request = req(&format!("Bearer {}", "a".repeat(64)));
        for _ in 0..50 {
            p.authenticate(&request).await.unwrap().unwrap();
        }
        assert_eq!(repo.touches.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_failed_write_does_not_reject_a_valid_token() {
        struct FailingRepo(Arc<CountingRepo>);

        #[async_trait]
        impl UserTokenRepository for FailingRepo {
            async fn create_token(
                &self,
                _: uuid::Uuid,
                _: &TokenOwner,
                _: &str,
                _: &str,
                _: Role,
                _: chrono::DateTime<chrono::Utc>,
                _: &[String],
            ) -> Result<UserToken, CoreError> {
                unreachable!()
            }
            async fn find_by_hash(&self, h: &str) -> Result<Option<UserToken>, CoreError> {
                self.0.find_by_hash(h).await
            }
            async fn list_for_user(&self, _: &TokenOwner) -> Result<Vec<UserToken>, CoreError> {
                Ok(vec![])
            }
            async fn touch_last_used(&self, _: uuid::Uuid) -> Result<(), CoreError> {
                Err(CoreError::Database("pool exhausted".into()))
            }
            async fn revoke(&self, _: uuid::Uuid, _: &TokenOwner) -> Result<bool, CoreError> {
                Ok(false)
            }
        }

        let p = UserTokenAuthProvider::new(Arc::new(FailingRepo(CountingRepo::new())));
        let id = p
            .authenticate(&req(&format!("Bearer {}", "a".repeat(64))))
            .await
            .unwrap();
        assert!(
            id.is_some(),
            "bookkeeping is best-effort; a valid credential stays valid"
        );
    }
}
