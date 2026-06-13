use super::{
    get, require_cargo, web, AppError, Arc, HttpResponse, LocalRegistryService, RegistryMap,
    Responder,
};

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
        (status = 200, description = "Owner list"),
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
        let users: Vec<_> = entries
            .into_iter()
            .enumerate()
            .map(|(i, e)| serde_json::json!({ "id": i + 1, "login": e.principal_id, "name": e.principal_id }))
            .collect();
        return Ok(HttpResponse::Ok().json(serde_json::json!({ "users": users })));
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
    Ok(HttpResponse::Ok().json(serde_json::json!({
        "users": [{ "id": 1, "login": publisher, "name": publisher }]
    })))
}
