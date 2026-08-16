//! `GET /api/v1/admin/subjects` — the identities this instance has seen.
//!
//! RFC 0004-bis A8. Four console fields ask an operator to type a subject —
//! the audit-log filter, the access-check simulator, a namespace claim's owner,
//! a beta-channel membership — and nothing could offer one, because
//! `/api/v1/admin/users/blocked` lists only the *blocked*.
//!
//! That absence is the only one in RFC 0004-bis's gap table that is currently
//! *invisible*: a subject field with no source does not error, it returns an
//! empty result set that reads as an answer. An operator filtering the audit
//! log for `alice` on an instance that stores `oidc:alice` gets an empty table,
//! which looks exactly like "this user did nothing" — on the surface whose
//! entire purpose is establishing what someone did.
//!
//! **Not a user directory.** This product does not have one and must not grow
//! one here. The answer is scoped to identities the instance has actually
//! observed: subjects in the access log, owners on namespace claims, and
//! accounts on the block list. An identity that has never touched this instance
//! is correctly absent — and a field that suggests must still accept a value
//! the server has not seen, which is §6.2's first non-negotiable behaviour.

use std::collections::BTreeSet;
use std::sync::Arc;

use actix_web::{get, web, Responder};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use batlehub_core::{
    ports::{TeamNamespacePort, UserBlockRepository},
    services::AdminService,
};

use super::super::require_admin;
use crate::{error::AppError, extractors::AuthIdentity, RegistryMap};

/// Ceiling on how many subjects one response may carry.
///
/// A suggesting field shows a handful; anything past that is a list nobody
/// reads and a query nobody meant to run. `q` is how you narrow it.
const MAX_LIMIT: u64 = 100;
const DEFAULT_LIMIT: u64 = 20;

#[derive(Debug, Deserialize, IntoParams)]
pub struct SubjectsQuery {
    /// Case-insensitive substring. `alice` matches `oidc:alice`, which is the
    /// mismatch this endpoint exists to make survivable.
    pub q: Option<String>,
    pub limit: Option<u64>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SubjectDto {
    /// The identity as this instance stores it — `oidc:alice`, not `alice`.
    pub user_id: String,
    /// Where it was seen. A subject seen in more than one place lists all of
    /// them, so an operator can tell a namespace owner who has never pulled
    /// anything from an account that is only in the audit log.
    pub sources: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SubjectsResponse {
    pub items: Vec<SubjectDto>,
    /// True when the limit truncated the answer, so a field can say "keep
    /// typing" rather than implying it has shown everything.
    pub truncated: bool,
}

/// List identities this instance has seen, for fields that name a subject.
#[utoipa::path(
    get,
    path = "/api/v1/admin/subjects",
    tag = "back-office",
    params(SubjectsQuery),
    responses(
        (status = 200, description = "Known subjects", body = SubjectsResponse),
        (status = 403, description = "Admin role required"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/api/v1/admin/subjects")]
pub async fn list_subjects(
    query: web::Query<SubjectsQuery>,
    identity: AuthIdentity,
    admin_svc: web::Data<Arc<AdminService>>,
    blocks: web::Data<Arc<dyn UserBlockRepository>>,
    // Optional: an instance with no team-namespace store simply contributes no
    // owners. A hard dependency would make this endpoint 500 rather than answer
    // with the two sources it does have — which is worse than a shorter list,
    // on the endpoint whose whole purpose is not returning a misleading empty.
    namespaces: Option<web::Data<Arc<dyn TeamNamespacePort>>>,
    registry_map: web::Data<RegistryMap>,
) -> Result<impl Responder, AppError> {
    require_admin(&identity)?;

    let limit = query.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let needle = query.q.as_deref().map(str::to_lowercase);
    let matches = |id: &str| {
        needle
            .as_ref()
            .is_none_or(|n| id.to_lowercase().contains(n))
    };

    // Ordered map: the audit log's ordering (most recent first) is the useful
    // one, so it is inserted first and the other two sources only *add*
    // sources to an existing entry or append behind it.
    let mut order: Vec<String> = Vec::new();
    let mut sources: std::collections::HashMap<String, BTreeSet<&'static str>> =
        std::collections::HashMap::new();
    let mut add = |id: String, source: &'static str| {
        let entry = sources.entry(id.clone()).or_default();
        if entry.is_empty() {
            order.push(id);
        }
        entry.insert(source);
    };

    // The audit log, over-fetched: the three sources are merged and then cut to
    // `limit`, so taking exactly `limit` from the largest one would let a
    // namespace owner be pushed out by audit subjects that are about to be
    // deduplicated against it anyway.
    for user_id in admin_svc
        .repo
        .distinct_event_subjects(query.q.as_deref(), limit * 2)
        .await
        .map_err(AppError::from)?
    {
        add(user_id, "audit");
    }

    // Namespace owners. `list_namespaces` is per-registry, and a claim list is
    // a handful of rows per registry, so this is bounded by the registry count.
    if let Some(namespaces) = namespaces {
        let claim_lists =
            futures::future::join_all(registry_map.entries().into_iter().map(|(name, _)| {
                let store = Arc::clone(&namespaces);
                async move { store.list_namespaces(&name).await }
            }))
            .await;
        for claimed_by in claim_lists
            .into_iter()
            .flat_map(|r| r.unwrap_or_default())
            .filter_map(|ns| ns.claimed_by)
        {
            if matches(&claimed_by) {
                add(claimed_by, "namespace");
            }
        }
    }

    // Blocked accounts. Already listed by `/admin/users/blocked`, and included
    // here so a simulator's subject field can offer the account an operator is
    // most likely about to check.
    for block in blocks.list().await.map_err(AppError::from)?.into_iter() {
        if matches(&block.user_id) {
            add(block.user_id, "blocked");
        }
    }

    let truncated = order.len() as u64 > limit;
    let items = order
        .into_iter()
        .take(limit as usize)
        .map(|user_id| SubjectDto {
            sources: sources
                .get(&user_id)
                .map(|s| s.iter().map(|s| (*s).to_owned()).collect())
                .unwrap_or_default(),
            user_id,
        })
        .collect();

    Ok(web::Json(SubjectsResponse { items, truncated }))
}
