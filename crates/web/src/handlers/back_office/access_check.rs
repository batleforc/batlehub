use std::sync::Arc;

use actix_web::{post, web, Responder};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use batlehub_core::{
    entities::{Identity, PackageId, PackageMetadata, Role},
    ports::{IpBlockStore, UserBlockRepository},
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
    /// Simulated client address, checked against the IP block list.
    ///
    /// Optional, and its absence is reported rather than assumed: a simulation
    /// with no address cannot answer for the IP block layer, and answering
    /// "allow" because none was supplied is the same defect this endpoint had
    /// one level down (RFC 0004-bis B4). See `covers` on the response.
    pub client_ip: Option<String>,
}

/// Which enforcement layers the answer actually accounts for.
///
/// The simulator used to evaluate `policy.rules` and nothing else, so an admin
/// who blocked `alice` on `/admin/security/blocks` and simulated `alice` on the
/// next tab was told **allow** — the page whose entire purpose is "would this
/// identity be allowed" contradicted by the section it lives in.
///
/// Stating coverage is part of the fix rather than a nicety: two of the three
/// layers are only checkable when the caller supplies the input they key on, so
/// a bare `allow` is ambiguous between "nothing denies this" and "nothing I
/// looked at denies this".
#[derive(Serialize, ToSchema)]
pub struct SimulationCoverage {
    /// Always true — the registry's rule chain is evaluated in full.
    pub rules: bool,
    /// True when `user_id` was supplied, so the account block list was consulted.
    pub account_blocks: bool,
    /// True when `client_ip` was supplied, so the IP block list was consulted.
    pub ip_blocks: bool,
}

/// What denied the request, when something did.
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum BlockedBy {
    /// The account is on the manual user block list.
    Account,
    /// The address is on the IP block list.
    Ip,
    /// A registry rule denied it.
    Rule,
}

#[derive(Serialize, ToSchema)]
pub struct AccessSimulationResponse {
    /// "allow" or "deny".
    pub decision: String,
    /// Present when decision is "deny".
    pub reason: Option<String>,
    /// Name of the rule that triggered the deny, if any.
    pub rule_matched: Option<String>,
    /// Which layer denied, when one did.
    pub blocked_by: Option<BlockedBy>,
    /// The layers this answer accounts for.
    pub covers: SimulationCoverage,
}

fn parse_role(s: Option<&str>) -> Result<Role, AppError> {
    match s {
        Some(value) => value
            .parse::<Role>()
            .map_err(|_| AppError::bad_request("role must be 'anonymous', 'user', or 'admin'")),
        None => Ok(Role::Anonymous),
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
        (status = 200, description = "Simulation result", body = AccessSimulationResponse),
        (status = 403, description = "Admin role required"),
        (status = 404, description = "Registry not configured"),
    ),
    security(("bearer_token" = [])),
)]
#[post("/api/v1/admin/access-check")]
pub async fn admin_access_check(
    identity: AuthIdentity,
    proxy_svc: web::Data<Arc<ProxyService>>,
    user_blocks: web::Data<Arc<dyn UserBlockRepository>>,
    ip_blocks: web::Data<Arc<dyn IpBlockStore>>,
    body: web::Json<AccessCheckRequest>,
) -> Result<impl Responder, AppError> {
    require_admin(&identity)?;

    let policy = {
        let hot = proxy_svc.hot.read().await;
        hot.policies.get(body.registry.as_str()).cloned()
    };

    let policy = policy.ok_or_else(|| AppError::not_found("registry not configured"))?;

    let covers = SimulationCoverage {
        rules: true,
        account_blocks: body.user_id.is_some(),
        ip_blocks: body.client_ip.is_some(),
    };

    /* The block layers first, and in this order, because that is where a real
    request meets them: `IpBlockMiddleware` and `UserBlockMiddleware` both
    reject *before* any rule is evaluated, so a simulator that starts at the
    rule chain is answering a different question than the one it was asked.
    Ordering matters for `blocked_by` too — an address on the IP list never
    reaches the account check in production, so it must not here either. */
    if let Some(ip) = body.client_ip.as_deref() {
        if let Some(unblock_at) = ip_blocks.blocked_until(ip).await? {
            return Ok(web::Json(AccessSimulationResponse {
                decision: "deny".to_owned(),
                reason: Some(format!("IP {ip} is blocked until {unblock_at}")),
                rule_matched: None,
                blocked_by: Some(BlockedBy::Ip),
                covers,
            }));
        }
    }

    if let Some(user_id) = body.user_id.as_deref() {
        if user_blocks.is_blocked(user_id).await? {
            return Ok(web::Json(AccessSimulationResponse {
                decision: "deny".to_owned(),
                reason: Some(format!("account '{user_id}' is blocked")),
                rule_matched: None,
                blocked_by: Some(BlockedBy::Account),
                covers,
            }));
        }
    }

    let sim_identity = Identity {
        user_id: body.user_id.clone(),
        role: parse_role(body.role.as_deref())?,
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
        RuleDecision::Allow => AccessSimulationResponse {
            decision: "allow".to_owned(),
            reason: None,
            rule_matched,
            blocked_by: None,
            covers,
        },
        RuleDecision::Deny { reason } => AccessSimulationResponse {
            decision: "deny".to_owned(),
            reason: Some(reason),
            rule_matched,
            blocked_by: Some(BlockedBy::Rule),
            covers,
        },
        RuleDecision::RequireRole { minimum } => AccessSimulationResponse {
            decision: "deny".to_owned(),
            reason: Some(format!("requires role '{minimum}' or higher")),
            rule_matched,
            blocked_by: Some(BlockedBy::Rule),
            covers,
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
