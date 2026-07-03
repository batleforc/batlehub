use super::{
    get, require_cargo, serve_local_or_proxy_artifact, web, AppError, Arc, AuthIdentity,
    CargoIndexMap, CoreError, HttpRequest, HttpResponse, LocalOrProxyArtifactOpts,
    LocalRegistryService, ProxyService, RegistryMap, RegistryMode, RegistryModeMap, Responder,
};

/// Cargo sparse registry `config.json`.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/registry/config.json",
    tag = "proxy/cargo",
    params(("registry" = String, Path, description = "Registry name")),
    responses(
        (status = 200, description = "Sparse registry configuration"),
        (status = 404, description = "No cargo registry configured"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/registry/config.json")]
pub async fn cargo_registry_config(
    path: web::Path<String>,
    indexes: web::Data<CargoIndexMap>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
    req: HttpRequest,
) -> HttpResponse {
    let registry = path.into_inner();
    if !map.is_type(&registry, "cargo") {
        return HttpResponse::NotFound().body(format!("unknown cargo registry '{registry}'"));
    }

    let mode = mode_map.get(&registry);

    // Proxy and Hybrid modes require a configured upstream index.
    if matches!(mode, RegistryMode::Proxy | RegistryMode::Hybrid)
        && indexes.get(&registry).is_none()
    {
        return HttpResponse::NotFound().body("no cargo index configured");
    }

    let (scheme, host) = {
        let info = req.connection_info();
        (info.scheme().to_owned(), info.host().to_owned())
    };
    let dl = format!("{scheme}://{host}/proxy/{registry}/{{crate}}/{{version}}/download");
    let mut resp = serde_json::json!({ "dl": dl });

    // Expose the publish API URL for local and hybrid registries.
    if matches!(mode, RegistryMode::Local | RegistryMode::Hybrid) {
        resp["api"] = serde_json::Value::String(format!("{scheme}://{host}/proxy/{registry}"));
    }

    HttpResponse::Ok()
        .content_type("application/json")
        .json(resp)
}

/// Cargo sparse registry index entries.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/registry/{path}",
    tag = "proxy/cargo",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("path"     = String, Path, description = "Crate index path, e.g. se/rd/serde"),
    ),
    responses(
        (status = 200, description = "Sparse index entry (newline-delimited JSON)"),
        (status = 404, description = "Crate not found in index"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/registry/{path:.*}")]
pub async fn cargo_registry_index(
    path: web::Path<(String, String)>,
    indexes: web::Data<CargoIndexMap>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    identity: AuthIdentity,
) -> HttpResponse {
    let (registry, index_path) = path.into_inner();
    if !map.is_type(&registry, "cargo") {
        return HttpResponse::NotFound().body(format!("unknown cargo registry '{registry}'"));
    }

    let mode = mode_map.get(&registry);

    match mode {
        RegistryMode::Local => {
            serve_local_index(&local_svc, &registry, &index_path, &identity).await
        }
        RegistryMode::Hybrid => {
            let local = serve_local_index(&local_svc, &registry, &index_path, &identity).await;
            if local.status() != actix_web::http::StatusCode::NOT_FOUND {
                return local;
            }
            proxy_upstream_index(&indexes, &registry, &index_path).await
        }
        RegistryMode::Proxy => proxy_upstream_index(&indexes, &registry, &index_path).await,
    }
}

async fn serve_local_index(
    local_svc: &LocalRegistryService,
    registry: &str,
    index_path: &str,
    identity: &batlehub_core::entities::Identity,
) -> HttpResponse {
    // The Cargo sparse index path format is "{prefix1}/{prefix2}/{name}" for
    // names ≥ 3 chars, or "{len}/{name}" for 1–2 char names.
    // `splitn(3, '/')` captures everything after the prefix segments as the
    // final component, which preserves slashes in package names (e.g. a
    // name like "scope/pkg" decoded from "scope%2Fpkg" in the URL remains
    // intact as "scope/pkg" rather than being truncated to "pkg").
    let name = index_path.splitn(3, '/').last().unwrap_or(index_path);
    match local_svc.get_index(registry, name, identity).await {
        Ok(content) => HttpResponse::Ok()
            .content_type("text/plain; charset=utf-8")
            .body(content),
        Err(CoreError::NotFound(_)) => {
            HttpResponse::NotFound().body(format!("crate '{name}' not found in local registry"))
        }
        Err(CoreError::AccessDenied(msg)) => HttpResponse::Forbidden().body(msg),
        Err(e) => {
            tracing::error!(error = %e, "local index lookup failed");
            HttpResponse::InternalServerError().body(e.to_string())
        }
    }
}

async fn proxy_upstream_index(
    indexes: &CargoIndexMap,
    registry: &str,
    index_path: &str,
) -> HttpResponse {
    let Some(index) = indexes.get(registry) else {
        return HttpResponse::NotFound().body("no cargo registry configured");
    };
    let url = format!("{}/{}", index.index_url.trim_end_matches('/'), index_path);
    tracing::debug!(url = %url, "fetching cargo sparse index entry");
    let resp = match index.http.get(&url).send().await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(url = %url, error = %e, "cargo index fetch failed");
            return HttpResponse::BadGateway().body(e.to_string());
        }
    };
    let status = actix_web::http::StatusCode::from_u16(resp.status().as_u16())
        .unwrap_or(actix_web::http::StatusCode::INTERNAL_SERVER_ERROR);
    match resp.bytes().await {
        Ok(bytes) => HttpResponse::build(status)
            .content_type("text/plain; charset=utf-8")
            .body(bytes),
        Err(e) => HttpResponse::BadGateway().body(e.to_string()),
    }
}

/// Download a `.crate` file for a specific version.
#[utoipa::path(
    get,
    path = "/proxy/{registry}/{name}/{version}/download",
    tag = "proxy/cargo",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("name"     = String, Path, description = "Crate name"),
        ("version"  = String, Path, description = "Version"),
    ),
    responses(
        (status = 200, description = ".crate file stream"),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Unknown registry"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/proxy/{registry}/{name}/{version}/download")]
pub async fn download_crate(
    path: web::Path<(String, String, String)>,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
) -> Result<impl Responder, AppError> {
    let (registry, name, version) = path.into_inner();
    require_cargo(&registry, &map)?;

    serve_local_or_proxy_artifact(
        svc,
        local_svc,
        &mode_map,
        &registry,
        &name,
        &version,
        identity,
        LocalOrProxyArtifactOpts {
            artifact_suffix: "dl",
            local_content_type: "application/octet-stream",
            proxy_content_type: None,
            resource_type: batlehub_core::rules::resource_type::SOURCE_READ,
            check_prerelease: true,
            append_signature: true,
        },
    )
    .await
}
