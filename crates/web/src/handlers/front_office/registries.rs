use actix_web::{get, web, Responder};
use serde::Serialize;
use utoipa::ToSchema;

use crate::{extractors::AuthIdentity, RegistryHostMap, RegistryMap, RegistryModeMap};

#[derive(Serialize, ToSchema)]
pub struct RegistryInfo {
    pub name: String,
    #[serde(rename = "type")]
    pub registry_type: String,
    pub mode: String,
    /// The registry's own hostname-rooted URL (`https://npm.acme.io`) when one is
    /// configured, otherwise `null` — clients then fall back to
    /// `{origin}/proxy/{name}`.
    ///
    /// Shown to anonymous callers too. The endpoint already filters by
    /// `accessible_registries_for`, so a caller only ever sees registries they
    /// can already reach; withholding the host from them would break the Setup
    /// Guide while hiding a name that is in public DNS anyway.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub public_url: Option<String>,
    /// The upstream this registry proxies (`https://registry.npmjs.org`), or
    /// `null` for a local-only registry that has none.
    ///
    /// The design proof's catalog caption names it — *"hybrid ·
    /// registry.npmjs.org · 1,284 packages · 6.1 GB cached"* — because "which
    /// upstream is behind this" is the first thing an operator asks of a
    /// registry they did not configure themselves.
    ///
    /// Visible to anonymous callers on the same terms as `public_url` above,
    /// and for the same reason: this endpoint already filters by
    /// `accessible_registries_for`, so a caller only ever sees registries they
    /// can already pull through — and pulling through one reveals the upstream
    /// anyway, in the artifacts it returns.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upstream: Option<String>,
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
    hosts: Option<web::Data<RegistryHostMap>>,
    upstreams: Option<web::Data<crate::UpstreamMap>>,
    access: web::Data<crate::AccessConfigLock>,
    identity: AuthIdentity,
) -> impl Responder {
    let accessible = access.read().await.accessible_registries_for(&identity);
    let mut registries: Vec<RegistryInfo> = map
        .entries()
        .into_iter()
        .filter(|(name, _)| accessible.contains(name.as_str()))
        .map(|(name, registry_type)| RegistryInfo {
            mode: modes.get(&name).as_str().to_owned(),
            public_url: hosts.as_ref().and_then(|h| h.public_url_for(&name)),
            upstream: upstreams.as_ref().and_then(|u| u.upstream_for(&name)),
            name,
            registry_type,
        })
        .collect();
    registries.sort_by(|a, b| a.name.cmp(&b.name));
    web::Json(registries)
}
