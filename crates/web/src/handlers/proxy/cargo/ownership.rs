use super::{
    delete, get, put, require_cargo, web, AppError, Arc, AuthIdentity, HttpResponse,
    LocalRegistryService, RegistryMap, Responder,
};

/// `cargo owner --list`'s response, in the shape crates.io defines.
#[derive(serde::Serialize, utoipa::ToSchema)]
pub struct CargoOwnersResponse {
    pub users: Vec<CargoOwner>,
}

/// One owner. `id` is a position in this list, not a stable identifier — this
/// server keys ownership by principal, and `cargo` only ever displays the field.
#[derive(serde::Serialize, utoipa::ToSchema)]
pub struct CargoOwner {
    pub id: usize,
    pub login: String,
    pub name: String,
}

/// List owners of a crate (`cargo owner --list`).
#[utoipa::path(
    get,
    path = "/proxy/{registry}/api/v1/crates/{name}/owners",
    tag = "proxy/cargo",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("name"     = String, Path, description = "Crate name"),
    ),
    responses(
        (status = 200, description = "Owner list", body = CargoOwnersResponse),
        (status = 404, description = "Crate not found"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/api/v1/crates/{name}/owners")]
pub async fn cargo_owners(
    path: web::Path<(String, String)>,
    map: web::Data<RegistryMap>,
    local_svc: web::Data<Arc<LocalRegistryService>>,
) -> Result<impl Responder, AppError> {
    let (registry, name) = path.into_inner();
    require_cargo(&registry, &map)?;

    if let Some(ref ownership) = local_svc.ownership {
        let entries = ownership
            .list_owners(&registry, &name)
            .await
            .map_err(AppError::from)?;
        let users: Vec<CargoOwner> = entries
            .into_iter()
            .enumerate()
            .map(|(i, e)| CargoOwner {
                id: i + 1,
                login: e.principal_id.clone(),
                name: e.principal_id,
            })
            .collect();
        return Ok(HttpResponse::Ok().json(CargoOwnersResponse { users }));
    }

    // Fallback: derive from first-published version.
    let versions = local_svc
        .backend
        .get_versions(&registry, &name)
        .await
        .map_err(AppError::from)?;
    if versions.is_empty() {
        return Err(AppError::not_found(format!("crate '{name}' not found")));
    }
    let publisher = versions[0]
        .published_by
        .clone()
        .unwrap_or_else(|| "unknown".to_owned());
    Ok(HttpResponse::Ok().json(CargoOwnersResponse {
        users: vec![CargoOwner {
            id: 1,
            login: publisher.clone(),
            name: publisher,
        }],
    }))
}

// ── Ownership management (RFC 0009 §7.6) ──────────────────────────────────────
//
// `cargo owner --add` / `--remove`. Ownership was readable through this route
// and not manageable, so the only way to change it was the admin API — which is
// not what the cargo CLI calls, and not something a package's own maintainer
// necessarily has.

/// The body `cargo owner --add`/`--remove` sends.
#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct CargoOwnersRequest {
    /// Logins to add or remove. cargo sends one per invocation, but the
    /// protocol is a list.
    pub users: Vec<String>,
}

/// cargo's acknowledgement: `ok` plus a message it prints verbatim.
#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct CargoOwnersChanged {
    pub ok: bool,
    pub msg: String,
}

/// Whether `identity` may change this crate's owners.
///
/// Ownership changes are governed by ownership itself: an existing owner may
/// grant and revoke. `can_publish` is the same predicate publishing uses, and
/// deliberately returns `true` for a package with no owners yet — a crate
/// nobody owns is one anybody with `User` may claim, which is how first publish
/// works and must not be a back door for taking over an owned one.
async fn require_owner(
    local_svc: &LocalRegistryService,
    registry: &str,
    name: &str,
    identity: &batlehub_core::entities::Identity,
) -> Result<(), AppError> {
    let Some(ref ownership) = local_svc.ownership else {
        return Err(AppError::not_found(
            "ownership management is not enabled for this registry".to_owned(),
        ));
    };
    if ownership
        .can_publish(registry, name, identity)
        .await
        .map_err(AppError::from)?
    {
        return Ok(());
    }
    Err(AppError::forbidden(format!(
        "you are not an owner of '{name}'"
    )))
}

/// `cargo owner --add`.
#[utoipa::path(
    put,
    path = "/proxy/{registry}/api/v1/crates/{name}/owners",
    tag = "proxy/cargo",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("name"     = String, Path, description = "Crate name"),
    ),
    request_body = CargoOwnersRequest,
    responses(
        (status = 200, description = "Owners added", body = CargoOwnersChanged),
        (status = 403, description = "Not an owner of this crate"),
        (status = 404, description = "Unknown registry, or ownership not enabled"),
    ),
    security(("bearer_token" = [])),
)]
#[put("/proxy/{registry}/api/v1/crates/{name}/owners")]
pub async fn cargo_add_owners(
    path: web::Path<(String, String)>,
    body: web::Json<CargoOwnersRequest>,
    identity: AuthIdentity,
    map: web::Data<RegistryMap>,
    local_svc: web::Data<Arc<LocalRegistryService>>,
) -> Result<impl Responder, AppError> {
    let (registry, name) = path.into_inner();
    require_cargo(&registry, &map)?;
    require_owner(&local_svc, &registry, &name, &identity.0).await?;

    let ownership = local_svc.ownership.as_ref().expect("checked above");
    let mut added = Vec::new();
    for login in &body.users {
        match ownership
            .add_owner(
                &registry,
                &name,
                batlehub_core::ports::OwnerEntry {
                    principal_type: "user".to_owned(),
                    principal_id: login.clone(),
                    role: "maintainer".to_owned(),
                    granted_by: identity.0.user_id.clone(),
                },
            )
            .await
        {
            Ok(()) => added.push(login.clone()),
            // Already an owner. cargo's own registry treats this as success,
            // and failing here would make `cargo owner --add` non-idempotent
            // for no benefit.
            Err(batlehub_core::error::CoreError::Conflict(_)) => added.push(login.clone()),
            Err(e) => return Err(AppError::from(e)),
        }
    }

    Ok(HttpResponse::Ok().json(CargoOwnersChanged {
        ok: true,
        msg: format!("added {} to owners of {name}", added.join(", ")),
    }))
}

/// `cargo owner --remove`.
#[utoipa::path(
    delete,
    path = "/proxy/{registry}/api/v1/crates/{name}/owners",
    tag = "proxy/cargo",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("name"     = String, Path, description = "Crate name"),
    ),
    request_body = CargoOwnersRequest,
    responses(
        (status = 200, description = "Owners removed", body = CargoOwnersChanged),
        (status = 403, description = "Not an owner of this crate"),
        (status = 404, description = "Unknown registry, or ownership not enabled"),
        (status = 409, description = "Removing this owner would leave the crate unowned"),
    ),
    security(("bearer_token" = [])),
)]
#[delete("/proxy/{registry}/api/v1/crates/{name}/owners")]
pub async fn cargo_remove_owners(
    path: web::Path<(String, String)>,
    body: web::Json<CargoOwnersRequest>,
    identity: AuthIdentity,
    map: web::Data<RegistryMap>,
    local_svc: web::Data<Arc<LocalRegistryService>>,
) -> Result<impl Responder, AppError> {
    let (registry, name) = path.into_inner();
    require_cargo(&registry, &map)?;
    require_owner(&local_svc, &registry, &name, &identity.0).await?;

    let ownership = local_svc.ownership.as_ref().expect("checked above");

    // A crate with no owners is a crate anyone may publish to (see
    // `require_owner`), so removing the last one is a privilege *escalation*
    // dressed as a removal. Refused rather than allowed and warned about.
    let current = ownership
        .list_owners(&registry, &name)
        .await
        .map_err(AppError::from)?;
    let remaining = current
        .iter()
        .filter(|e| !body.users.contains(&e.principal_id))
        .count();
    if remaining == 0 && !current.is_empty() {
        return Err(AppError::conflict(format!(
            "refusing to remove the last owner of '{name}': a crate with no owners \
             may be published to by anyone"
        )));
    }

    for login in &body.users {
        ownership
            .remove_owner(&registry, &name, "user", login)
            .await
            .map_err(AppError::from)?;
    }

    Ok(HttpResponse::Ok().json(CargoOwnersChanged {
        ok: true,
        msg: format!("removed {} from owners of {name}", body.users.join(", ")),
    }))
}
