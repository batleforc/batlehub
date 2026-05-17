use actix_web::{Responder, get, web};
use serde::Serialize;
use utoipa::ToSchema;

use crate::{AccessConfig, RegistryMap, extractors::AuthIdentity};

#[derive(Serialize, ToSchema)]
pub struct RegistryInfo {
    pub name: String,
    #[serde(rename = "type")]
    pub registry_type: String,
}

/// List configured registries visible to the current user.
#[utoipa::path(
    get,
    path = "/api/v1/registries",
    tag = "front-office",
    responses(
        (status = 200, description = "List of accessible registries", body = Vec<RegistryInfo>),
    ),
    security(("bearer_token" = [])),
)]
#[get("/api/v1/registries")]
pub async fn list_registries(
    map: web::Data<RegistryMap>,
    access: web::Data<AccessConfig>,
    identity: AuthIdentity,
) -> impl Responder {
    let accessible = access.accessible_registries_for(&identity);
    let mut registries: Vec<RegistryInfo> = map
        .0
        .iter()
        .filter(|(name, _)| accessible.contains(name.as_str()))
        .map(|(name, registry_type)| RegistryInfo {
            name: name.clone(),
            registry_type: registry_type.clone(),
        })
        .collect();
    registries.sort_by(|a, b| a.name.cmp(&b.name));
    web::Json(registries)
}
