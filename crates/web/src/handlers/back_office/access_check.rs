use std::sync::Arc;

use actix_web::{post, web, Responder};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use batlehub_core::{
    entities::{Identity, PackageId, PackageMetadata, Role},
    rules::{Rule, RuleContext, RuleDecision},
    services::ProxyService,
};

use super::require_admin;
use crate::{error::AppError, extractors::AuthIdentity};

#[derive(Deserialize, ToSchema)]
pub struct AccessCheckRequest {
    pub registry: String,
    pub package_name: String,
    pub version: String,
    pub resource_type: String,
    /// Simulated user id (optional).
    pub user_id: Option<String>,
    /// Simulated role: "anonymous", "user", or "admin". Defaults to "anonymous".
    pub role: Option<String>,
    /// Simulated OIDC groups the identity belongs to.
    #[serde(default)]
    pub groups: Vec<String>,
}

#[derive(Serialize, ToSchema)]
pub struct AccessCheckResponse {
    /// "allow" or "deny".
    pub decision: String,
    /// Present when decision is "deny".
    pub reason: Option<String>,
    /// Name of the rule that triggered the deny, if any.
    pub rule_matched: Option<String>,
}

fn parse_role(s: Option<&str>) -> Role {
    match s {
        Some("admin") => Role::Admin,
        Some("user") => Role::User,
        _ => Role::Anonymous,
    }
}

/// Simulate whether a given identity would be allowed to perform an operation
/// against a registry's policy without issuing a real request (admin).
#[utoipa::path(
    post,
    path = "/api/v1/admin/access-check",
    tag = "back-office",
    request_body = AccessCheckRequest,
    responses(
        (status = 200, description = "Simulation result", body = AccessCheckResponse),
        (status = 403, description = "Admin role required"),
        (status = 404, description = "Registry not configured"),
    ),
    security(("bearer_token" = [])),
)]
#[post("/api/v1/admin/access-check")]
pub async fn admin_access_check(
    identity: AuthIdentity,
    proxy_svc: web::Data<Arc<ProxyService>>,
    body: web::Json<AccessCheckRequest>,
) -> Result<impl Responder, AppError> {
    require_admin(&identity)?;

    let policy = {
        let hot = proxy_svc.hot.read().await;
        hot.policies.get(body.registry.as_str()).cloned()
    };

    let policy = policy.ok_or_else(|| AppError::not_found("registry not configured"))?;

    let sim_identity = Identity {
        user_id: body.user_id.clone(),
        role: parse_role(body.role.as_deref()),
        auth_provider: None,
        groups: body.groups.clone(),
    };

    let package_id = PackageId::new(&body.registry, &body.package_name, &body.version);
    let metadata = PackageMetadata {
        id: package_id,
        published_at: None,
        download_url: None,
        checksum: None,
        is_signed: None,
        extra: serde_json::Value::Null,
        cache_control: None,
    };

    let ctx = RuleContext {
        identity: &sim_identity,
        package: &metadata,
        resource_type: &body.resource_type,
        cache_entry: None,
        requested_version: Some(&body.version),
    };

    let (decision, rule_matched) = evaluate_and_trace(&policy.rules, &ctx).await;

    let response = match decision {
        RuleDecision::Allow => AccessCheckResponse {
            decision: "allow".to_owned(),
            reason: None,
            rule_matched,
        },
        RuleDecision::Deny { reason } => AccessCheckResponse {
            decision: "deny".to_owned(),
            reason: Some(reason),
            rule_matched,
        },
        RuleDecision::RequireRole { minimum } => AccessCheckResponse {
            decision: "deny".to_owned(),
            reason: Some(format!("requires role '{minimum}' or higher")),
            rule_matched,
        },
    };

    Ok(web::Json(response))
}

/// Run rules in order, returning the first deny and the name of the rule that caused it.
async fn evaluate_and_trace(
    rules: &[Box<dyn Rule>],
    ctx: &RuleContext<'_>,
) -> (RuleDecision, Option<String>) {
    for rule in rules {
        let decision = rule.evaluate(ctx).await.resolve(ctx.identity);
        if decision.is_deny() {
            return (decision, Some(rule.name().to_owned()));
        }
    }
    (RuleDecision::Allow, None)
}
