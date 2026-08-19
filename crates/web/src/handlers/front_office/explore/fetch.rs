//! `POST /api/v1/explore/packages/{registry}/{name}/{version}/fetch`
//! (RFC 0007-bis §4.4).
//!
//! **This runs the download.** Not a warming task, not a special path — the same
//! `ProxyService::handle` a package manager's request runs, with the same
//! `ProxyRequest`, under the caller's own identity. That is the whole design,
//! and §5.3 is the section of the RFC most worth reading before changing
//! anything here.
//!
//! The single most important sentence: **this must not reuse the warming
//! service.** `WarmingService::warm_one_version_inner` calls
//! `client.fetch_artifact` directly, bypassing the rule engine, the release-age
//! gate, the block list, the licence gate, quota and the access audit — which is
//! fine, because its only caller is `require_admin`. Wiring a non-admin button
//! to it would hand every console user a way to pull bytes past every gate the
//! proxy has, and it would not look like a hole. It would look like reuse.
//!
//! What follows from being the download path, and is the point: the rules run,
//! integrity verification runs, quota is consumed, the access event is recorded
//! **with the caller as the actor**, and SBOM and README extraction run because
//! the artifact lands in storage through the ordinary path. The handler is short
//! by construction, and its length is the argument for the design.

use std::time::Instant;

use actix_web::post;
use futures::StreamExt;

use super::{web, AppError, Arc, AuthIdentity, Deserialize, IntoParams, Serialize, ToSchema};
use batlehub_core::{
    entities::{PackageId, RegistryKind, Role},
    services::{LocalRegistryService, ProxyRequest, ProxyResponse, ProxyService},
};

use crate::RegistryMap;

/// A rule refused the download. The body carries the rule's own reason.
pub const FETCH_DENIED: &str = "fetch.denied";

/// This instance already holds the version.
pub const FETCH_ALREADY_HELD: &str = "fetch.already-held";

/// The registry, or this build, does not offer the button.
pub const FETCH_UNSUPPORTED: &str = "fetch.unsupported";

/// The caller has no session. Pulling is an authenticated act (§4.1 revisited).
pub const FETCH_UNAUTHENTICATED: &str = "fetch.unauthenticated";

#[derive(Deserialize, IntoParams)]
pub struct FetchPath {
    pub registry: String,
    pub name: String,
    pub version: String,
}

#[derive(Serialize, ToSchema)]
pub struct FetchResponse {
    pub fetched: bool,
    /// What arrived, so the row can say what the wait bought.
    ///
    /// Measured against real upstreams, the median version is 0.57 MB and the
    /// largest in the sample was 41.7 MB (RFC 0007-bis §13.4) — a range wide
    /// enough that "done" on its own tells a reader nothing.
    pub size_bytes: u64,
    pub duration_ms: u64,
    /// The coordinate that was fetched, echoed so a client that fired several
    /// can match responses to rows.
    pub registry: String,
    pub name: String,
    pub version: String,
}

/// Fetch one version from upstream, as the caller.
#[utoipa::path(
    post,
    path = "/api/v1/explore/packages/{registry}/{name}/{version}/fetch",
    tag = "explore",
    params(FetchPath),
    responses(
        (status = 200, description = "The version was fetched and is now held", body = FetchResponse),
        (status = 401, description = "No session; pulling a version requires one"),
        (status = 403, description = "A rule refused it; the body carries the rule's own reason"),
        (status = 404, description = "No such registry, package or version upstream"),
        (status = 409, description = "This instance already holds the version"),
    ),
    security(("bearer_token" = [])),
)]
#[post("/api/v1/explore/packages/{registry}/{name}/{version}/fetch")]
pub async fn explore_fetch_version(
    path: web::Path<FetchPath>,
    identity: AuthIdentity,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    proxy_svc: web::Data<Arc<ProxyService>>,
    registry_map: web::Data<RegistryMap>,
) -> Result<web::Json<FetchResponse>, AppError> {
    let FetchPath {
        registry,
        name,
        version,
    } = path.into_inner();

    // `404` rather than `403` on a visibility refusal, exactly as the detail and
    // README endpoints do: a `403` confirms the package exists, which is the
    // fact a non-public package is trying not to disclose.
    if local_svc
        .check_visibility(&registry, &name, &identity)
        .await
        .is_err()
    {
        return Err(AppError::not_found(format!(
            "package '{name}' not found in registry '{registry}'"
        )));
    }

    // **A session is required to pull.**
    //
    // This endpoint used to admit an anonymous caller, on the reasoning above:
    // the fetch downloads exactly what that caller could already pull through
    // the proxy with `curl`, so it grants no read they did not have. That is
    // true about *reading* and it is not the whole question. A fetch is a write
    // to this instance — it fills the cache, spends the upstream's bandwidth and
    // ours, extracts an SBOM, and lands a row in the audit log — and it is the
    // one button in the console that does so on a page an unauthenticated reader
    // can open. Measured against the running instance before this check existed,
    // an anonymous `POST …/strip-ansi/7.2.0/fetch` came back `409 already-held`:
    // it had passed visibility, the operator's switch and the kind check, and
    // was stopped only by the artifact happening to be there already.
    //
    // The proxy path is unchanged and still serves whoever the operator's
    // `anonymous` policy allows. What is refused here is *causing this instance
    // to go and get something* without saying who you are — the audit row for
    // which would read `anonymous`.
    //
    // After the visibility check on purpose: a package this caller may not see
    // must answer `404` first, or `401` becomes an oracle for whether a private
    // package exists.
    if identity.0.role == Role::Anonymous {
        return Err(AppError::unauthorized(
            "fetching a version requires a signed-in session".to_owned(),
        )
        .coded(FETCH_UNAUTHENTICATED));
    }

    // The operator's switch. It admits nothing — the fetch is a download the
    // caller could already run with `curl` — so it exists for the operator who
    // wants the console strictly read-only, which is a legitimate posture and
    // not one the software should have to guess at (§4.1).
    let (console_fetch, kind) = {
        let hot = local_svc.hot.read().await;
        let enabled = hot
            .console_fetch
            .get(&registry)
            .copied()
            .unwrap_or(batlehub_core::services::DEFAULT_CONSOLE_FETCH);
        let kind = registry_map
            .type_of(&registry)
            .and_then(|t| t.parse::<RegistryKind>().ok());
        (enabled, kind)
    };
    if !console_fetch {
        return Err(AppError::forbidden(format!(
            "fetching from the console is disabled for registry '{registry}'"
        ))
        .coded(FETCH_UNSUPPORTED));
    }

    // "Fetch this version" has to name one thing. Maven's artifact is a set of
    // files and a Terraform provider needs an OS and an architecture, so those
    // answer with the reason rather than a disabled button with no explanation.
    // No type means no such registry here. `404` rather than a `400` about an
    // unknown kind: the caller named something that does not exist, and saying
    // so in the vocabulary of registry *types* would be an answer to a question
    // they did not ask.
    let Some(kind) = kind else {
        return Err(AppError::not_found(format!(
            "registry '{registry}' not found"
        )));
    };
    let support = kind.fetchable_by_version();
    if let Some(reason) = support.reason() {
        return Err(AppError::bad_request(reason.to_owned()).coded(FETCH_UNSUPPORTED));
    }

    let mut package_id = PackageId::new(&registry, &name, &version);
    if let Some(artifact) = support.artifact() {
        package_id = package_id.with_artifact(artifact);
    }

    // Already here? Then this would be a cache read dressed as a fetch, and the
    // row the reader is looking at is out of date rather than upstream-only.
    // `409` says which, so the console can refresh instead of reporting a
    // download that did not happen.
    //
    // The **proxy** key, not the local-publish one: they describe different
    // halves of the same catalogue, and asking the wrong one is a question
    // always answered "no".
    if proxy_svc
        .storage
        .exists(&batlehub_core::services::proxy::proxy_artifact_key(
            &package_id,
        ))
        .await
        .unwrap_or(false)
    {
        return Err(
            AppError::conflict(format!("this instance already holds {name} {version}"))
                .coded(FETCH_ALREADY_HELD),
        );
    }

    // From here it is the download. `source:read` rather than `releases:read`:
    // the button pulls **bytes**, and the permission it needs is the one a
    // package manager's download needs, not the one a metadata read needs.
    let started = Instant::now();
    let response = proxy_svc
        .handle(ProxyRequest {
            package_id,
            identity: identity.0.clone(),
            resource_type: "source:read".to_owned(),
            ip_address: None,
            user_agent: None,
        })
        .await
        .map_err(AppError::from)?;

    let stream = match response {
        // The rule's own reason, which is the same string the download would
        // have given — so the console shows the operator *why*, and the
        // `/tools/access-check` page it already links to explains the same
        // verdict (§4.4).
        ProxyResponse::Denied { reason } => {
            return Err(AppError::forbidden(reason).coded(FETCH_DENIED))
        }
        ProxyResponse::Stream(stream) => stream,
    };

    // Drained, not forwarded: this wants the side effect, not the bytes.
    // Everything that makes a download safe has already applied because it *is*
    // a download; the only difference is that nothing writes the response to a
    // socket.
    let mut stream = stream;
    let mut size_bytes: u64 = 0;
    while let Some(chunk) = stream.next().await {
        match chunk {
            Ok(bytes) => size_bytes += bytes.len() as u64,
            // A break mid-stream means the artifact did not land. Reported as an
            // error rather than as a short success, because the row would
            // otherwise claim to be held when it is not.
            Err(e) => {
                return Err(AppError::from(batlehub_core::error::CoreError::Registry(
                    format!("fetching {name} {version}: {e}"),
                )))
            }
        }
    }

    Ok(web::Json(FetchResponse {
        fetched: true,
        size_bytes,
        duration_ms: started.elapsed().as_millis() as u64,
        registry,
        name,
        version,
    }))
}
