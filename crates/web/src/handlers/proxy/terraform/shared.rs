use super::{
    dispatch_notification, proxy_stream, require_local_mode, require_registry_type, web, AppError,
    Arc, AuthIdentity, HttpRequest, HttpResponse, LocalRegistryService, NotificationEventType,
    NotificationService, PackageId, ProxyService, RegistryMap, RegistryMode, RegistryModeMap,
};

pub fn base_url_from_req(req: &HttpRequest) -> String {
    let info = req.connection_info();
    format!("{}://{}", info.scheme(), info.host())
}

/// Shared yank/unyank flow for Terraform modules and providers: validates the
/// registry/mode, performs the (un)yank, dispatches the notification, and builds
/// the JSON response message. `display_name` is the human-readable identifier used
/// in the response message (e.g. `"module {namespace}/{name}/{provider}"` or
/// `"provider {namespace}/{ptype}"`).
#[allow(clippy::too_many_arguments)]
pub async fn terraform_set_yanked(
    registry: &str,
    map: &RegistryMap,
    mode_map: &RegistryModeMap,
    pkg_name: &str,
    version: &str,
    display_name: &str,
    identity: &AuthIdentity,
    local_svc: &Arc<LocalRegistryService>,
    notification_svc: &web::Data<Option<Arc<NotificationService>>>,
    yanked: bool,
) -> Result<HttpResponse, AppError> {
    require_registry_type(registry, "terraform", map)?;
    require_local_mode(registry, mode_map)?;

    let actor = identity.0.user_id.clone().unwrap_or_default();
    let (event_type, verb) = if yanked {
        local_svc
            .yank(registry, pkg_name, version, &identity.0)
            .await
            .map_err(AppError::from)?;
        (NotificationEventType::PackageYanked, "yanked")
    } else {
        local_svc
            .unyank(registry, pkg_name, version, &identity.0)
            .await
            .map_err(AppError::from)?;
        (NotificationEventType::PackageUnyanked, "unyanked")
    };

    dispatch_notification(
        notification_svc,
        event_type,
        registry,
        pkg_name,
        Some(version.to_owned()),
        &actor,
    );

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "message": format!("{verb} {display_name}@{version}")
    })))
}

/// Shared versions-listing flow for Terraform modules and providers: if `local_result`
/// is `Some`, it's the already-awaited local/hybrid lookup; on `NotFound` in hybrid
/// mode (or `None`, i.e. proxy mode), falls through to streaming the upstream response.
pub async fn terraform_versions_response(
    registry: &str,
    pkg_name: String,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    mode: RegistryMode,
    local_result: Option<Result<serde_json::Value, batlehub_core::error::CoreError>>,
) -> Result<HttpResponse, AppError> {
    if let Some(result) = local_result {
        match result {
            Ok(json) => return Ok(HttpResponse::Ok().json(json)),
            Err(batlehub_core::error::CoreError::NotFound(_)) if mode == RegistryMode::Hybrid => {}
            Err(batlehub_core::error::CoreError::NotFound(msg)) => {
                return Err(AppError::not_found(msg))
            }
            Err(e) => return Err(AppError::from(e)),
        }
    }

    let pkg = PackageId::new(registry, pkg_name, "versions");
    proxy_stream(
        svc,
        pkg,
        identity,
        batlehub_core::rules::resource_type::RELEASES_READ,
        Some("application/json"),
    )
    .await
}
