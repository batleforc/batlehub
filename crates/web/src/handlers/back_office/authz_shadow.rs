//! `GET /api/v1/admin/authz/shadow` — what shadow mode would have refused.
//!
//! RFC 0015 §4.7 asks for three records of every would-have-been: a structured
//! log line, a `batlehub_policy_dryrun_total` counter labelled by policy and
//! node, and *"an admin endpoint listing recent would-have-beens so the console
//! can show them"*. This is the third.
//!
//! # Why this endpoint exists at all
//!
//! `grants.dry_run` is the most useful setting in RFC 0015 and the most
//! dangerous: a request that would be refused is **served**. It is what makes
//! §10's migration survivable in practice — enable the new model in shadow,
//! watch a week of real traffic, then enforce — and it is also, if forgotten, an
//! authorization bypass configured on purpose.
//!
//! A shadow with nothing to read is only the dangerous half. The whole reason to
//! run one is to answer *"what breaks if I enforce?"*, and that question has an
//! answer only if the would-have-beens are somewhere an operator will look.

use std::sync::Arc;

use actix_web::{get, web, HttpResponse, Responder};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use batlehub_core::services::{shadow::ShadowLog, ProxyService};

use crate::{error::AppError, extractors::AuthIdentity};

/// One request shadow mode served that enforcement would have refused.
#[derive(Debug, Serialize, ToSchema)]
pub struct ShadowedDenialDto {
    pub at: DateTime<Utc>,
    pub registry: String,
    pub package: String,
    pub version: String,
    /// The verb the caller did not hold.
    pub action: String,
    /// The subject, in the spelling a grant would be written in — so it can be
    /// pasted into the block that would fix this.
    pub subject: String,
    /// The node whose shadow served the request.
    pub node: String,
    pub shadow_until: NaiveDate,
}

/// One node's shadow, summarised.
///
/// The shape the question actually has. *"Can I enforce this namespace yet?"* is
/// answered by which verbs are still missing and for whom, not by a list of
/// requests — and a busy registry produces thousands of the latter for a handful
/// of the former.
#[derive(Debug, Serialize, ToSchema)]
pub struct ShadowSummaryDto {
    pub node: String,
    pub shadow_until: NaiveDate,
    /// How many requests this node's shadow has served.
    pub count: u64,
    /// The distinct verbs that were missing, in the order first seen.
    pub actions: Vec<String>,
    /// The distinct subjects that lacked them.
    pub subjects: Vec<String>,
    pub last_seen: DateTime<Utc>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ShadowResponse {
    /// Per node, busiest first — an operator triaging a migration reads
    /// top-down.
    pub by_node: Vec<ShadowSummaryDto>,
    /// The individual entries, newest first.
    pub recent: Vec<ShadowedDenialDto>,
    /// How many entries the buffer keeps at most.
    ///
    /// Reported rather than assumed, because `recent.len() == kept` is the one
    /// state where an operator must not read the list as complete — a bounded
    /// buffer that looked exhaustive would understate what a shadow is serving,
    /// which is the wrong direction for this page.
    pub kept: usize,
    /// `true` when nothing is being shadowed at all — no node in the loaded
    /// config carries a `grants_shadow` block.
    ///
    /// Distinguishes "the shadow is quiet" from "there is no shadow", which look
    /// identical in an empty list and mean opposite things: the first says
    /// enforcing is safe, the second says nothing was measured.
    pub no_shadow_configured: bool,
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct ShadowQuery {
    /// How many individual entries to return. Defaults to 100.
    #[serde(default)]
    pub limit: Option<usize>,
}

/// What shadow mode has served that enforcement would have refused.
#[utoipa::path(
    get,
    path = "/api/v1/admin/authz/shadow",
    tag = "back-office",
    params(ShadowQuery),
    responses(
        (status = 200, description = "Recent would-have-beens, and a summary per node", body = ShadowResponse),
        (status = 403, description = "`authz:read` required"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/api/v1/admin/authz/shadow")]
pub async fn authz_shadow(
    query: web::Query<ShadowQuery>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    hot: web::Data<batlehub_core::services::hot_config::HotConfigLock>,
) -> Result<impl Responder, AppError> {
    crate::handlers::back_office::require_verb(
        &identity,
        batlehub_core::entities::Action::AuthzRead,
        None,
        &hot,
    )
    .await?;

    let (log, no_shadow_configured) = {
        let hot = svc.hot.read().await;
        // Whether *any* loaded node carries a shadow. Read from the resolved
        // hierarchy rather than from the config file, so it answers for what is
        // actually running after a reload rather than for what is on disk.
        let configured = hot.grants.values().any(|g| {
            g.registry.dry_run.is_some() || g.namespaces.iter().any(|(_, n)| n.dry_run.is_some())
        });
        (hot.shadow_log.clone(), !configured)
    };

    let Some(log) = log else {
        // No buffer wired: the shadow still serves what it would refuse — that
        // is what the operator configured — but there is nothing recorded to
        // show. Answering an empty list without saying so would report a quiet
        // shadow, which is the opposite of the truth.
        return Ok(HttpResponse::Ok().json(ShadowResponse {
            by_node: Vec::new(),
            recent: Vec::new(),
            kept: 0,
            no_shadow_configured,
        }));
    };

    let limit = query.limit.unwrap_or(100).min(500);
    Ok(HttpResponse::Ok().json(build_response(&log, limit, no_shadow_configured).await))
}

async fn build_response(
    log: &Arc<ShadowLog>,
    limit: usize,
    no_shadow_configured: bool,
) -> ShadowResponse {
    let by_node = log
        .by_node()
        .await
        .into_iter()
        .map(|s| ShadowSummaryDto {
            node: s.node,
            shadow_until: s.shadow_until,
            count: s.count,
            actions: s.actions,
            subjects: s.subjects,
            last_seen: s.last_seen,
        })
        .collect();
    let recent = log
        .recent(limit)
        .await
        .into_iter()
        .map(|d| ShadowedDenialDto {
            at: d.at,
            registry: d.registry,
            package: d.package,
            version: d.version,
            action: d.action,
            subject: d.subject,
            node: d.node,
            shadow_until: d.shadow_until,
        })
        .collect::<Vec<_>>();
    ShadowResponse {
        by_node,
        kept: log.capacity(),
        recent,
        no_shadow_configured,
    }
}
