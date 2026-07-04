use actix_web::{get, web, Responder};
use serde::Serialize;
use utoipa::ToSchema;

use crate::{extractors::AuthIdentity, RegistryMap, RegistryModeMap};

#[derive(Serialize, ToSchema)]
pub struct RegistryInfo {
    pub name: String,
    #[serde(rename = "type")]
    pub registry_type: String,
    pub mode: String,
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
    modes: web::Data<RegistryModeMap>,
    access: web::Data<crate::AccessConfigLock>,
    identity: AuthIdentity,
) -> impl Responder {
    let accessible = access.read().await.accessible_registries_for(&identity);
    let mut registries: Vec<RegistryInfo> = map
        .entries()
        .into_iter()
        .filter(|(name, _)| accessible.contains(name.as_str()))
        .map(|(name, registry_type)| RegistryInfo {
            mode: match modes.get(&name) {
                batlehub_config::schema::RegistryMode::Proxy => "proxy",
                batlehub_config::schema::RegistryMode::Local => "local",
                batlehub_config::schema::RegistryMode::Hybrid => "hybrid",
            }
            .to_string(),
            name,
            registry_type,
        })
        .collect();
    registries.sort_by(|a, b| a.name.cmp(&b.name));
    web::Json(registries)
}
