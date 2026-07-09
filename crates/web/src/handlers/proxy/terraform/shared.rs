use super::{
    dispatch_notification, proxy_stream, require_local_mode, require_registry_type, web, AppError,
    Arc, AuthIdentity, HttpRequest, HttpResponse, LocalRegistryService, NotificationEventType,
    NotificationService, PackageId, ProxyService, RegistryMap, RegistryMode, RegistryModeMap,
};

pub fn base_url_from_req(req: &HttpRequest) -> String {
    let info = req.connection_info();
    format!("{}://{}", info.scheme(), info.host())
}

/// The data describing a single Terraform yank/unyank request — everything
/// [`terraform_set_yanked`] needs about *what* is being (un)yanked, grouped so
/// the function's other params stay limited to identity/service handles
/// (mirrors `common.rs`'s `LocalOrProxyArtifactOpts` split for the analogous
/// artifact-serving cluster).
pub struct TerraformYankRequest<'a> {
    pub registry: &'a str,
    pub map: &'a RegistryMap,
    pub mode_map: &'a RegistryModeMap,
    pub pkg_name: &'a str,
    pub version: &'a str,
    /// Human-readable identifier used in the response message, e.g.
    /// `"module {namespace}/{name}/{provider}"` or `"provider {namespace}/{ptype}"`.
    pub display_name: &'a str,
    pub yanked: bool,
}

/// Shared yank/unyank flow for Terraform modules and providers: validates the
/// registry/mode, performs the (un)yank, dispatches the notification, and builds
/// the JSON response message.
pub async fn terraform_set_yanked(
    req: TerraformYankRequest<'_>,
    identity: &AuthIdentity,
    local_svc: &Arc<LocalRegistryService>,
    notification_svc: &web::Data<Option<Arc<NotificationService>>>,
) -> Result<HttpResponse, AppError> {
    require_registry_type(req.registry, "terraform", req.map)?;
    require_local_mode(req.registry, req.mode_map)?;

    let actor = identity.0.user_id.clone().unwrap_or_default();
    let (event_type, verb) = if req.yanked {
        local_svc
            .yank(req.registry, req.pkg_name, req.version, &identity.0)
            .await
            .map_err(AppError::from)?;
        (NotificationEventType::PackageYanked, "yanked")
    } else {
        local_svc
            .unyank(req.registry, req.pkg_name, req.version, &identity.0)
            .await
            .map_err(AppError::from)?;
        (NotificationEventType::PackageUnyanked, "unyanked")
    };

    dispatch_notification(
        notification_svc,
        event_type,
        req.registry,
        req.pkg_name,
        Some(req.version.to_owned()),
        &actor,
    );

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "message": format!("{verb} {}@{}", req.display_name, req.version)
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
