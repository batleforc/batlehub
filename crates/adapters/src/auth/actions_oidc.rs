use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use jsonwebtoken::jwk::JwkSet;
use jsonwebtoken::{decode, decode_header, DecodingKey, Validation};
use serde::Deserialize;
use tokio::sync::RwLock;

use batlehub_config::schema::{ActionsGroupRule, ActionsOidcAuthConfig, ConditionMatchType, RuleMatch};
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
}

struct JwksCache {
    keys: JwkSet,
    fetched_at: Instant,
}

enum CompiledCondition {
    Glob { claim: String, pattern: glob::Pattern },
    Regex { claim: String, re: regex::Regex },
}

impl CompiledCondition {
    fn compile(c: &batlehub_config::schema::Condition) -> anyhow::Result<Self> {
        let is_regex = match c.match_type {
            ConditionMatchType::Regex => true,
            ConditionMatchType::Glob => false,
            ConditionMatchType::Auto => detect_is_regex(&c.pattern),
        };
        if is_regex {
            let re = regex::Regex::new(&c.pattern)
                .map_err(|e| anyhow::anyhow!("invalid regex pattern {:?}: {e}", c.pattern))?;
            Ok(Self::Regex { claim: c.claim.clone(), re })
        } else {
            let pattern = glob::Pattern::new(&c.pattern)
                .map_err(|e| anyhow::anyhow!("invalid glob pattern {:?}: {e}", c.pattern))?;
            Ok(Self::Glob { claim: c.claim.clone(), pattern })
        }
    }

    fn matches(&self, claims: &serde_json::Map<String, serde_json::Value>) -> bool {
        match self {
            Self::Glob { claim, pattern } => {
                let val = claim_str(claims, claim);
                pattern.matches(val)
            }
            Self::Regex { claim, re } => {
                let val = claim_str(claims, claim);
                re.is_match(val)
            }
        }
    }
}

fn detect_is_regex(pattern: &str) -> bool {
    pattern.starts_with('^')
        || pattern.ends_with('$')
        || pattern.contains("(?")
        || pattern.contains("\\d")
        || pattern.contains("\\w")
        || pattern.contains('[')
        || pattern.contains('(')
        || pattern.contains('+')
}

fn claim_str<'a>(claims: &'a serde_json::Map<String, serde_json::Value>, key: &str) -> &'a str {
    claims.get(key).and_then(|v| v.as_str()).unwrap_or("")
}

struct CompiledRule {
    static_group: Option<String>,
    group_template: Option<String>,
    role: Role,
    conditions: Vec<CompiledCondition>,
    match_mode: RuleMatch,
}

impl CompiledRule {
    fn compile(rule: &ActionsGroupRule) -> anyhow::Result<Self> {
        if rule.group.is_none() && rule.group_template.is_none() {
            anyhow::bail!("each rule must have at least one of 'group' or 'group_template'");
        }
        let role = parse_role(&rule.role);
        let conditions = rule
            .conditions
            .iter()
            .map(CompiledCondition::compile)
            .collect::<anyhow::Result<Vec<_>>>()?;
        Ok(Self {
            static_group: rule.group.clone(),
            group_template: rule.group_template.clone(),
            role,
            conditions,
            match_mode: rule.match_mode.clone(),
        })
    }

    fn evaluate(&self, claims: &serde_json::Map<String, serde_json::Value>) -> bool {
        if self.conditions.is_empty() {
            return true;
        }
        match self.match_mode {
            RuleMatch::All => self.conditions.iter().all(|c| c.matches(claims)),
            RuleMatch::Any => self.conditions.iter().any(|c| c.matches(claims)),
        }
    }

    fn collect_groups(
        &self,
        provider_name: &str,
        claims: &serde_json::Map<String, serde_json::Value>,
    ) -> Vec<String> {
        let mut groups = Vec::new();
        if let Some(g) = &self.static_group {
            groups.push(g.clone());
        }
        if let Some(t) = &self.group_template {
            groups.push(render_group_template(t, provider_name, claims));
        }
        groups
    }
}

fn parse_role(s: &str) -> Role {
    match s {
        "admin" => Role::Admin,
        "user" => Role::User,
        _ => Role::Anonymous,
    }
}

/// Render a group name template by substituting `{placeholders}` with claim values.
///
/// Special variables:
/// - `{name}`: the provider's configured name
/// - `{ref_name}`: the `ref` claim with `refs/heads/` or `refs/tags/` prefix stripped
/// - `{any_claim}`: the value of that JWT claim
///
/// Substituted values have `/` replaced with `-` so group names stay path-safe.
/// Template literal `/` separators are preserved unchanged.
fn render_group_template(
    template: &str,
    provider_name: &str,
    claims: &serde_json::Map<String, serde_json::Value>,
) -> String {
    let ref_name: String = claims
        .get("ref")
        .and_then(|v| v.as_str())
        .map(|r| {
            r.strip_prefix("refs/heads/")
                .or_else(|| r.strip_prefix("refs/tags/"))
                .unwrap_or(r)
                .replace('/', "-")
        })
        .unwrap_or_default();

    let mut result = String::with_capacity(template.len() + 16);
    let mut chars = template.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '{' {
            let mut key = String::new();
            let mut closed = false;
            for c in chars.by_ref() {
                if c == '}' {
                    closed = true;
                    break;
                }
                key.push(c);
            }
            if closed {
                let substituted = if key == "name" {
                    provider_name.replace('/', "-")
                } else if key == "ref_name" {
                    ref_name.clone()
                } else {
                    claims
                        .get(&key)
                        .and_then(|v| v.as_str())
                        .map(|s| s.replace('/', "-"))
                        .unwrap_or_else(|| format!("{{{key}}}"))
                };
                result.push_str(&substituted);
            } else {
                result.push('{');
                result.push_str(&key);
            }
        } else {
            result.push(ch);
        }
    }
    result
}

pub struct ActionsOidcAuthProvider {
    name: String,
    issuer: String,
    user_id_claim: String,
    rules: Vec<CompiledRule>,
    http: reqwest::Client,
    jwks_uri: String,
    cache: Arc<RwLock<JwksCache>>,
}

impl ActionsOidcAuthProvider {
    pub async fn new(cfg: &ActionsOidcAuthConfig) -> anyhow::Result<Self> {
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

        let rules = cfg
            .rules
            .iter()
            .map(CompiledRule::compile)
            .collect::<anyhow::Result<Vec<_>>>()?;

        Ok(Self {
            name: cfg.name.clone(),
            issuer: discovery.issuer,
            user_id_claim: cfg.user_id_claim.clone(),
            rules,
            http,
            jwks_uri: discovery.jwks_uri,
            cache: Arc::new(RwLock::new(JwksCache {
                keys,
                fetched_at: Instant::now(),
            })),
        })
    }

    async fn get_decoding_key(&self, kid: Option<&str>) -> Result<DecodingKey, CoreError> {
        {
            let cache = self.cache.read().await;
            if let Some(key) = find_key(&cache.keys, kid) {
                return Ok(key);
            }
            if cache.fetched_at.elapsed() < JWKS_MIN_REFRESH {
                return Err(CoreError::Auth("unknown JWT signing key".to_owned()));
            }
        }

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
impl AuthProvider for ActionsOidcAuthProvider {
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

        let mut matched_role = Role::Anonymous;
        let mut groups: Vec<String> = Vec::new();

        for rule in &self.rules {
            if rule.evaluate(&claims) {
                if rule.role > matched_role {
                    matched_role = rule.role.clone();
                }
                groups.extend(rule.collect_groups(&self.name, &claims));
            }
        }

        Ok(Some(Identity {
            user_id,
            role: matched_role,
            auth_provider: Some(self.name.clone()),
            groups,
        }))
    }
}

#[cfg(test)]
impl ActionsOidcAuthProvider {
    fn for_testing(name: impl Into<String>, user_id_claim: impl Into<String>, rules: Vec<CompiledRule>, jwks: JwkSet) -> Self {
        Self {
            name: name.into(),
            issuer: String::new(),
            user_id_claim: user_id_claim.into(),
            rules,
            http: reqwest::Client::new(),
            jwks_uri: String::new(),
            cache: Arc::new(RwLock::new(JwksCache {
                keys: jwks,
                fetched_at: Instant::now(),
            })),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
    use serde_json::json;
    use std::time::{SystemTime, UNIX_EPOCH};

    const TEST_EC_PRIVATE_KEY: &str = "-----BEGIN PRIVATE KEY-----\n\
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgWTFfCGljY6aw3Hrt\n\
kHmPRiazukxPLb6ilpRAewjW8nihRANCAATDskChT+Altkm9X7MI69T3IUmrQU0L\n\
950IxEzvw/x5BMEINRMrXLBJhqzO9Bm+d6JbqA21YQmd1Kt4RzLJR1W+\n\
-----END PRIVATE KEY-----";

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

    fn test_jwks() -> JwkSet {
        serde_json::from_str(TEST_JWKS_JSON).unwrap()
    }

    fn future_exp() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
            + 3600
    }

    fn past_exp() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
            - 3600
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

    fn no_rules_provider() -> ActionsOidcAuthProvider {
        ActionsOidcAuthProvider::for_testing("actions", "sub", vec![], test_jwks())
    }

    // ── Template rendering ────────────────────────────────────────────────────

    fn claims_map(v: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
        match v {
            serde_json::Value::Object(m) => m,
            _ => panic!("expected object"),
        }
    }

    #[test]
    fn template_repo_and_branch() {
        let claims = claims_map(json!({
            "repository": "batleforc/batlehub",
            "ref": "refs/heads/main"
        }));
        let out = render_group_template("{name}/{repository}/{ref_name}", "forgejo-action", &claims);
        assert_eq!(out, "forgejo-action/batleforc-batlehub/main");
    }

    #[test]
    fn template_tag_ref_name() {
        let claims = claims_map(json!({ "ref": "refs/tags/v1.2.3" }));
        let out = render_group_template("{name}/{ref_name}", "ci", &claims);
        assert_eq!(out, "ci/v1.2.3");
    }

    #[test]
    fn template_bare_ref_without_prefix() {
        let claims = claims_map(json!({ "ref": "main" }));
        let out = render_group_template("{ref_name}", "ci", &claims);
        assert_eq!(out, "main");
    }

    #[test]
    fn template_missing_claim_left_as_placeholder() {
        let claims = claims_map(json!({}));
        let out = render_group_template("{missing}", "ci", &claims);
        assert_eq!(out, "{missing}");
    }

    #[test]
    fn template_provider_name_slash_replaced() {
        let claims = claims_map(json!({}));
        let out = render_group_template("{name}", "a/b", &claims);
        assert_eq!(out, "a-b");
    }

    // ── Pattern matching ──────────────────────────────────────────────────────

    #[test]
    fn glob_matches_wildcard() {
        let c = batlehub_config::schema::Condition {
            claim: "repository".to_owned(),
            pattern: "myorg/*".to_owned(),
            match_type: ConditionMatchType::Auto,
        };
        let cc = CompiledCondition::compile(&c).unwrap();
        let claims = claims_map(json!({ "repository": "myorg/foo" }));
        assert!(cc.matches(&claims));
        let claims2 = claims_map(json!({ "repository": "other/foo" }));
        assert!(!cc.matches(&claims2));
    }

    #[test]
    fn regex_matches_tag_pattern() {
        let c = batlehub_config::schema::Condition {
            claim: "ref".to_owned(),
            pattern: "^refs/tags/v[0-9]+".to_owned(),
            match_type: ConditionMatchType::Auto,
        };
        let cc = CompiledCondition::compile(&c).unwrap();
        let claims = claims_map(json!({ "ref": "refs/tags/v1.0.0" }));
        assert!(cc.matches(&claims));
        let claims2 = claims_map(json!({ "ref": "refs/heads/main" }));
        assert!(!cc.matches(&claims2));
    }

    #[test]
    fn auto_detect_regex_on_caret() {
        assert!(detect_is_regex("^refs/tags/"));
        assert!(!detect_is_regex("refs/heads/*"));
    }

    #[test]
    fn absent_claim_does_not_match() {
        let c = batlehub_config::schema::Condition {
            claim: "missing".to_owned(),
            pattern: "something".to_owned(),
            match_type: ConditionMatchType::Glob,
        };
        let cc = CompiledCondition::compile(&c).unwrap();
        let claims = claims_map(json!({}));
        assert!(!cc.matches(&claims));
    }

    #[test]
    fn explicit_glob_with_regex_chars() {
        let c = batlehub_config::schema::Condition {
            claim: "ref".to_owned(),
            pattern: "refs/heads/main".to_owned(),
            match_type: ConditionMatchType::Glob,
        };
        let cc = CompiledCondition::compile(&c).unwrap();
        let claims = claims_map(json!({ "ref": "refs/heads/main" }));
        assert!(cc.matches(&claims));
    }

    // ── CompiledRule ──────────────────────────────────────────────────────────

    fn make_rule(
        static_group: Option<&str>,
        template: Option<&str>,
        role: Role,
        conditions: Vec<CompiledCondition>,
        match_mode: RuleMatch,
    ) -> CompiledRule {
        CompiledRule {
            static_group: static_group.map(str::to_owned),
            group_template: template.map(str::to_owned),
            role,
            conditions,
            match_mode,
        }
    }

    #[test]
    fn rule_all_requires_all_conditions() {
        let cond1 = CompiledCondition::compile(&batlehub_config::schema::Condition {
            claim: "repository_owner".to_owned(),
            pattern: "myorg".to_owned(),
            match_type: ConditionMatchType::Glob,
        }).unwrap();
        let cond2 = CompiledCondition::compile(&batlehub_config::schema::Condition {
            claim: "ref".to_owned(),
            pattern: "refs/heads/main".to_owned(),
            match_type: ConditionMatchType::Glob,
        }).unwrap();
        let rule = make_rule(Some("g"), None, Role::User, vec![cond1, cond2], RuleMatch::All);

        let both = claims_map(json!({ "repository_owner": "myorg", "ref": "refs/heads/main" }));
        assert!(rule.evaluate(&both));

        let only_one = claims_map(json!({ "repository_owner": "myorg", "ref": "refs/heads/dev" }));
        assert!(!rule.evaluate(&only_one));
    }

    #[test]
    fn rule_any_requires_one_condition() {
        let cond1 = CompiledCondition::compile(&batlehub_config::schema::Condition {
            claim: "ref_type".to_owned(),
            pattern: "branch".to_owned(),
            match_type: ConditionMatchType::Glob,
        }).unwrap();
        let cond2 = CompiledCondition::compile(&batlehub_config::schema::Condition {
            claim: "ref_type".to_owned(),
            pattern: "tag".to_owned(),
            match_type: ConditionMatchType::Glob,
        }).unwrap();
        let rule = make_rule(Some("g"), None, Role::User, vec![cond1, cond2], RuleMatch::Any);

        let branch = claims_map(json!({ "ref_type": "branch" }));
        assert!(rule.evaluate(&branch));
        let tag = claims_map(json!({ "ref_type": "tag" }));
        assert!(rule.evaluate(&tag));
        let other = claims_map(json!({ "ref_type": "other" }));
        assert!(!rule.evaluate(&other));
    }

    #[test]
    fn empty_conditions_always_match() {
        let rule = make_rule(Some("g"), None, Role::User, vec![], RuleMatch::All);
        assert!(rule.evaluate(&claims_map(json!({}))));
    }

    #[test]
    fn collect_groups_static_only() {
        let rule = make_rule(Some("my-group"), None, Role::User, vec![], RuleMatch::All);
        let groups = rule.collect_groups("prov", &claims_map(json!({})));
        assert_eq!(groups, vec!["my-group"]);
    }

    #[test]
    fn collect_groups_template_only() {
        let rule = make_rule(None, Some("{name}/{ref_name}"), Role::User, vec![], RuleMatch::All);
        let claims = claims_map(json!({ "ref": "refs/heads/main" }));
        let groups = rule.collect_groups("prov", &claims);
        assert_eq!(groups, vec!["prov/main"]);
    }

    #[test]
    fn collect_groups_both() {
        let rule = make_rule(Some("static"), Some("{name}/{ref_name}"), Role::User, vec![], RuleMatch::All);
        let claims = claims_map(json!({ "ref": "refs/heads/feat" }));
        let groups = rule.collect_groups("ci", &claims);
        assert_eq!(groups, vec!["static", "ci/feat"]);
    }

    // ── Provider authenticate ─────────────────────────────────────────────────

    #[tokio::test]
    async fn no_auth_header_returns_none() {
        let p = no_rules_provider();
        assert!(p.authenticate(&no_auth()).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn basic_auth_header_returns_none() {
        let p = no_rules_provider();
        let req = RawAuthRequest {
            headers: [("authorization".to_owned(), "Basic dXNlcjpwYXNz".to_owned())].into(),
            query_params: Default::default(),
        };
        assert!(p.authenticate(&req).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn malformed_token_returns_auth_error() {
        let p = no_rules_provider();
        let err = p.authenticate(&bearer("not.a.valid.jwt")).await.unwrap_err();
        assert!(matches!(err, CoreError::Auth(_)));
    }

    #[tokio::test]
    async fn valid_jwt_no_rules_returns_anonymous_empty_groups() {
        let p = no_rules_provider();
        let token = signed_token(
            Some("test-kid"),
            json!({ "sub": "ci-bot", "repository": "myorg/myrepo", "ref": "refs/heads/main", "exp": future_exp() }),
        );
        let id = p.authenticate(&bearer(&token)).await.unwrap().unwrap();
        assert_eq!(id.role, Role::Anonymous);
        assert!(id.groups.is_empty());
        assert_eq!(id.user_id.as_deref(), Some("ci-bot"));
        assert_eq!(id.auth_provider.as_deref(), Some("actions"));
    }

    #[tokio::test]
    async fn matching_rule_grants_group_and_role() {
        let cond = CompiledCondition::compile(&batlehub_config::schema::Condition {
            claim: "repository_owner".to_owned(),
            pattern: "myorg".to_owned(),
            match_type: ConditionMatchType::Glob,
        }).unwrap();
        let rule = make_rule(Some("ci-group"), None, Role::User, vec![cond], RuleMatch::All);
        let p = ActionsOidcAuthProvider::for_testing("actions", "sub", vec![rule], test_jwks());

        let token = signed_token(
            Some("test-kid"),
            json!({ "sub": "bot", "repository_owner": "myorg", "exp": future_exp() }),
        );
        let id = p.authenticate(&bearer(&token)).await.unwrap().unwrap();
        assert_eq!(id.role, Role::User);
        assert_eq!(id.groups, vec!["ci-group"]);
    }

    #[tokio::test]
    async fn two_matching_rules_union_groups_max_role() {
        let cond1 = CompiledCondition::compile(&batlehub_config::schema::Condition {
            claim: "repository_owner".to_owned(),
            pattern: "myorg".to_owned(),
            match_type: ConditionMatchType::Glob,
        }).unwrap();
        let cond2 = CompiledCondition::compile(&batlehub_config::schema::Condition {
            claim: "repository_owner".to_owned(),
            pattern: "myorg".to_owned(),
            match_type: ConditionMatchType::Glob,
        }).unwrap();
        let rule1 = make_rule(Some("group-a"), None, Role::User, vec![cond1], RuleMatch::All);
        let rule2 = make_rule(Some("group-b"), None, Role::Admin, vec![cond2], RuleMatch::All);
        let p = ActionsOidcAuthProvider::for_testing("actions", "sub", vec![rule1, rule2], test_jwks());

        let token = signed_token(
            Some("test-kid"),
            json!({ "sub": "bot", "repository_owner": "myorg", "exp": future_exp() }),
        );
        let id = p.authenticate(&bearer(&token)).await.unwrap().unwrap();
        assert_eq!(id.role, Role::Admin);
        assert!(id.groups.contains(&"group-a".to_owned()));
        assert!(id.groups.contains(&"group-b".to_owned()));
    }

    #[tokio::test]
    async fn template_rule_renders_dynamic_group() {
        let rule = make_rule(None, Some("{name}/{repository}/{ref_name}"), Role::User, vec![], RuleMatch::All);
        let p = ActionsOidcAuthProvider::for_testing("forgejo-action", "sub", vec![rule], test_jwks());

        let token = signed_token(
            Some("test-kid"),
            json!({
                "sub": "bot",
                "repository": "batleforc/batlehub",
                "ref": "refs/heads/main",
                "exp": future_exp()
            }),
        );
        let id = p.authenticate(&bearer(&token)).await.unwrap().unwrap();
        assert_eq!(id.groups, vec!["forgejo-action/batleforc-batlehub/main"]);
    }

    #[tokio::test]
    async fn non_matching_rule_contributes_nothing() {
        let cond = CompiledCondition::compile(&batlehub_config::schema::Condition {
            claim: "repository_owner".to_owned(),
            pattern: "other-org".to_owned(),
            match_type: ConditionMatchType::Glob,
        }).unwrap();
        let rule = make_rule(Some("group"), None, Role::Admin, vec![cond], RuleMatch::All);
        let p = ActionsOidcAuthProvider::for_testing("actions", "sub", vec![rule], test_jwks());

        let token = signed_token(
            Some("test-kid"),
            json!({ "sub": "bot", "repository_owner": "myorg", "exp": future_exp() }),
        );
        let id = p.authenticate(&bearer(&token)).await.unwrap().unwrap();
        assert_eq!(id.role, Role::Anonymous);
        assert!(id.groups.is_empty());
    }

    #[tokio::test]
    async fn expired_jwt_returns_none() {
        let p = no_rules_provider();
        let token = signed_token(
            Some("test-kid"),
            json!({ "sub": "bot", "exp": past_exp() }),
        );
        assert!(p.authenticate(&bearer(&token)).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn unknown_kid_returns_auth_error() {
        let p = no_rules_provider();
        let token = signed_token(
            Some("bad-kid"),
            json!({ "sub": "bot", "exp": future_exp() }),
        );
        assert!(matches!(p.authenticate(&bearer(&token)).await.unwrap_err(), CoreError::Auth(_)));
    }

    // ── Network bootstrap (mockito) ───────────────────────────────────────────

    fn discovery_json(base_url: &str) -> String {
        serde_json::json!({
            "issuer": base_url,
            "jwks_uri": format!("{base_url}/jwks"),
        })
        .to_string()
    }

    #[tokio::test]
    async fn new_bootstraps_from_discovery_document() {
        let mut server = mockito::Server::new_async().await;
        let base = server.url();

        let _disc = server
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

        let cfg = ActionsOidcAuthConfig {
            name: "test".to_owned(),
            issuer_url: base.clone(),
            user_id_claim: "sub".to_owned(),
            rules: vec![],
        };

        let provider = ActionsOidcAuthProvider::new(&cfg)
            .await
            .expect("provider construction failed");
        assert_eq!(provider.name(), "test");
    }
}
