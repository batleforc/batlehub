//! Registering the GPG keys a Terraform namespace's providers are signed with.
//!
//! RFC 0015 §4.2's `terraform:signing-keys:write`, and the store behind it. §13.13
//! recorded the verb as gating an action this server did not implement; what made
//! it worth implementing rather than deleting is that the *read* side was already
//! there and was serving `{"gpg_public_keys": []}` — a registry telling every
//! Terraform client there was nothing to verify a locally published provider
//! against.

use std::sync::Arc;

use actix_web::{delete, get, put, web, HttpResponse, Responder};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use batlehub_core::entities::{Action, SigningKey};
use batlehub_core::services::LocalRegistryService;

use crate::{error::AppError, extractors::AuthIdentity, handlers::schemas::OkResponse};

/// One key, in Terraform's own field names.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SigningKeyDto {
    /// The long key id, uppercase hex without `0x`.
    pub key_id: String,
    /// The armoured public key block.
    pub ascii_armor: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trust_signature: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
}

impl From<SigningKey> for SigningKeyDto {
    fn from(k: SigningKey) -> Self {
        Self {
            key_id: k.key_id,
            ascii_armor: k.ascii_armor,
            trust_signature: k.trust_signature,
            source: k.source,
            source_url: k.source_url,
        }
    }
}

impl From<SigningKeyDto> for SigningKey {
    fn from(d: SigningKeyDto) -> Self {
        Self {
            key_id: d.key_id,
            ascii_armor: d.ascii_armor,
            trust_signature: d.trust_signature,
            source: d.source,
            source_url: d.source_url,
        }
    }
}

#[derive(Serialize, ToSchema)]
pub struct SigningKeyListResponse {
    pub keys: Vec<SigningKeyDto>,
}

/// The store, or a `404` explaining that nothing can be registered without one.
///
/// `async`, and deliberately: the first version reached the lock through
/// `futures::executor::block_on` from inside an async handler, which parks the
/// executor thread on a lock another task may need to release. It happens to
/// survive a multi-threaded runtime and deadlocks a single-threaded one, which is
/// the worst combination — it passes here and hangs somewhere else.
async fn require_store(
    local_svc: &LocalRegistryService,
) -> Result<Arc<dyn batlehub_core::ports::SigningKeyPort>, AppError> {
    let port = { local_svc.hot.read().await.signing_keys.clone() };
    port.ok_or_else(|| AppError::not_found("this deployment has no signing-key store configured"))
}

/// The keys registered for a Terraform namespace.
#[utoipa::path(
    get,
    path = "/api/v1/admin/registries/{registry}/signing-keys/{namespace}",
    tag = "back-office",
    params(
        ("registry"  = String, Path, description = "Registry name"),
        ("namespace" = String, Path, description = "Terraform namespace"),
    ),
    responses(
        (status = 200, description = "Registered keys", body = SigningKeyListResponse),
        (status = 403, description = "`terraform:signing-keys:write` required"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/api/v1/admin/registries/{registry}/signing-keys/{namespace}")]
pub async fn list_signing_keys(
    path: web::Path<(String, String)>,
    identity: AuthIdentity,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    hot: web::Data<batlehub_core::services::hot_config::HotConfigLock>,
) -> Result<impl Responder, AppError> {
    let (registry, namespace) = path.into_inner();
    // The write verb gates the read too. There is no `signing-keys:read`, and
    // inventing one would be a verb for a surface that is **already public**:
    // these keys are served to every Terraform client in the download response.
    // What this endpoint adds is the administrative view, so it takes the
    // administrative verb.
    crate::handlers::back_office::require_verb(
        &identity,
        Action::TerraformSigningKeysWrite,
        Some(&registry),
        &hot,
    )
    .await?;

    let store = require_store(&local_svc).await?;
    let keys = store
        .list_signing_keys(&registry, &namespace)
        .await
        .map_err(AppError::from)?;
    Ok(web::Json(SigningKeyListResponse {
        keys: keys.into_iter().map(SigningKeyDto::from).collect(),
    }))
}

/// Register or replace a key.
#[utoipa::path(
    put,
    path = "/api/v1/admin/registries/{registry}/signing-keys/{namespace}",
    tag = "back-office",
    params(
        ("registry"  = String, Path, description = "Registry name"),
        ("namespace" = String, Path, description = "Terraform namespace"),
    ),
    request_body = SigningKeyDto,
    responses(
        (status = 200, description = "Key registered", body = OkResponse),
        (status = 400, description = "The key cannot verify anything"),
        (status = 403, description = "`terraform:signing-keys:write` required"),
    ),
    security(("bearer_token" = [])),
)]
#[put("/api/v1/admin/registries/{registry}/signing-keys/{namespace}")]
pub async fn set_signing_key(
    path: web::Path<(String, String)>,
    body: web::Json<SigningKeyDto>,
    identity: AuthIdentity,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    hot: web::Data<batlehub_core::services::hot_config::HotConfigLock>,
) -> Result<impl Responder, AppError> {
    let (registry, namespace) = path.into_inner();
    crate::handlers::back_office::require_verb(
        &identity,
        Action::TerraformSigningKeysWrite,
        Some(&registry),
        &hot,
    )
    .await?;

    let key: SigningKey = body.into_inner().into();
    // Refused at the edge rather than stored and served: a key that verifies
    // nothing is indistinguishable, from a client's side, from the empty
    // placeholder this endpoint exists to replace — except that it looks
    // configured.
    key.validate().map_err(AppError::bad_request)?;

    let store = require_store(&local_svc).await?;
    store
        .set_signing_key(&registry, &namespace, key)
        .await
        .map_err(AppError::from)?;
    Ok(HttpResponse::Ok().json(OkResponse { ok: true }))
}

/// Remove a key by id.
#[utoipa::path(
    delete,
    path = "/api/v1/admin/registries/{registry}/signing-keys/{namespace}/{key_id}",
    tag = "back-office",
    params(
        ("registry"  = String, Path, description = "Registry name"),
        ("namespace" = String, Path, description = "Terraform namespace"),
        ("key_id"    = String, Path, description = "The key id to remove"),
    ),
    responses(
        (status = 200, description = "Key removed, or was not registered", body = OkResponse),
        (status = 403, description = "`terraform:signing-keys:write` required"),
    ),
    security(("bearer_token" = [])),
)]
#[delete("/api/v1/admin/registries/{registry}/signing-keys/{namespace}/{key_id}")]
pub async fn delete_signing_key(
    path: web::Path<(String, String, String)>,
    identity: AuthIdentity,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    hot: web::Data<batlehub_core::services::hot_config::HotConfigLock>,
) -> Result<impl Responder, AppError> {
    let (registry, namespace, key_id) = path.into_inner();
    crate::handlers::back_office::require_verb(
        &identity,
        Action::TerraformSigningKeysWrite,
        Some(&registry),
        &hot,
    )
    .await?;

    let store = require_store(&local_svc).await?;
    store
        .delete_signing_key(&registry, &namespace, &key_id)
        .await
        .map_err(AppError::from)?;
    Ok(HttpResponse::Ok().json(OkResponse { ok: true }))
}

// ── RFC 0015 §4.2 — `jetbrains:channel:assign` ───────────────────────────────
//
// Filed beside the signing keys because they are the same kind of thing: the two
// ecosystem verbs whose actions this server can perform, landing together.

/// The channel to move a build to.
#[derive(Debug, Deserialize, ToSchema)]
pub struct AssignChannelRequest {
    /// The release channel. The **empty string is Stable**, which is
    /// JetBrains' own convention and not a missing value — `eco_jetbrains.rs`
    /// reads it the same way when selecting builds.
    pub channel: String,
}

/// Move a published plugin build to a release channel.
#[utoipa::path(
    put,
    path = "/api/v1/admin/registries/{registry}/plugins/{plugin}/{version}/channel",
    tag = "back-office",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("plugin"   = String, Path, description = "Plugin id"),
        ("version"  = String, Path, description = "Plugin version"),
    ),
    request_body = AssignChannelRequest,
    responses(
        (status = 200, description = "Channel assigned, or already that channel", body = OkResponse),
        (status = 403, description = "`jetbrains:channel:assign` required"),
        (status = 404, description = "Unknown registry, or not in local/hybrid mode"),
    ),
    security(("bearer_token" = [])),
)]
#[put("/api/v1/admin/registries/{registry}/plugins/{plugin}/{version}/channel")]
pub async fn assign_plugin_channel(
    path: web::Path<(String, String, String)>,
    body: web::Json<AssignChannelRequest>,
    identity: AuthIdentity,
    local_svc: web::Data<Arc<LocalRegistryService>>,
) -> Result<impl Responder, AppError> {
    let (registry, plugin, version) = path.into_inner();
    // The verb is checked in the service, beside the ownership and namespace
    // gates the other lifecycle mutations pass — not here. A handler that
    // repeated it would be the second place the question is answered, which is
    // the shape §5.0 exists to remove.
    let changed = local_svc
        .assign_channel(&registry, &plugin, &version, &body.channel, &identity.0)
        .await
        .map_err(AppError::from)?;
    let _ = changed;
    Ok(HttpResponse::Ok().json(OkResponse { ok: true }))
}
