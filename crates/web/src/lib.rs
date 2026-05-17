pub mod error;
pub mod extractors;
pub mod handlers;
pub mod middleware;

use std::sync::Arc;

use actix_web::web;
use utoipa::OpenApi;
use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa_actix_web::{AppExt, service_config::ServiceConfig as UtoipaServiceConfig};
use utoipa_swagger_ui::SwaggerUi;

use sqlx::PgPool;

use proxy_cache_core::{
    ports::UserTokenRepository,
    services::{AdminService, ProxyService},
};

pub use handlers::proxy::cargo::CargoIndexProxy;
pub use middleware::AuthMiddlewareFactory;

#[derive(OpenApi)]
#[openapi(
    tags(
        (name = "proxy", description = "Registry proxy — forward requests to upstream registries with caching and RBAC"),
        (name = "front-office", description = "User-facing package information"),
        (name = "back-office", description = "Admin management (requires Admin role)"),
    ),
    modifiers(&SecurityAddon),
)]
pub struct ApiDoc;

struct SecurityAddon;

impl utoipa::Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "bearer_token",
            SecurityScheme::Http(
                HttpBuilder::new()
                    .scheme(HttpAuthScheme::Bearer)
                    .bearer_format("token")
                    .build(),
            ),
        );
    }
}

fn collect_routes(cfg: &mut UtoipaServiceConfig) {
    use handlers::{
        auth::{
            oidc::{oidc_callback, oidc_login, oidc_refresh},
            tokens::{create_token, list_tokens, revoke_token},
        },
        back_office::{
            audit::audit_log,
            packages::{block_package, list_packages as admin_list_packages, package_detail, unblock_package},
        },
        front_office::{
            me::me,
            packages::{check_access, list_packages},
        },
        proxy::{
            cargo::{cargo_registry_config, cargo_registry_index, download_crate, get_crate, get_version as cargo_get_version},
            github::{download_asset, download_asset_by_name, download_raw, download_tarball, download_zipball, get_release, list_releases},
            npm::{download_tarball as npm_download_tarball, get_packument, get_version as npm_get_version},
        },
    };

    cfg.service(oidc_login);
    cfg.service(oidc_callback);
    cfg.service(oidc_refresh);
    cfg.service(create_token);
    cfg.service(list_tokens);
    cfg.service(revoke_token);
    cfg.service(list_releases);
    cfg.service(get_release);
    cfg.service(download_asset);
    cfg.service(download_asset_by_name);
    cfg.service(download_tarball);
    cfg.service(download_zipball);
    cfg.service(download_raw);
    cfg.service(get_packument);
    cfg.service(npm_get_version);
    cfg.service(npm_download_tarball);
    cfg.service(cargo_registry_config);
    cfg.service(cargo_registry_index);
    cfg.service(get_crate);
    cfg.service(cargo_get_version);
    cfg.service(download_crate);
    cfg.service(me);
    cfg.service(list_packages);
    cfg.service(check_access);
    cfg.service(admin_list_packages);
    cfg.service(package_detail);
    cfg.service(block_package);
    cfg.service(unblock_package);
    cfg.service(audit_log);
}

/// Return the raw OpenAPI JSON spec (auto-collected from route registrations).
pub fn openapi_spec() -> utoipa::openapi::OpenApi {
    let (_, openapi) = actix_web::App::new()
        .into_utoipa_app()
        .openapi(ApiDoc::openapi())
        .configure(collect_routes)
        .split_for_parts();
    openapi
}

/// Return a SwaggerUi service using the provided OpenAPI spec.
pub fn swagger_ui(openapi: utoipa::openapi::OpenApi) -> SwaggerUi {
    SwaggerUi::new("/swagger-ui/{_:.*}").url("/api/openapi.json", openapi)
}

/// Configure all application routes on a `UtoipaApp`.
///
/// Static file serving (SPA fallback) is intentionally excluded — register it on
/// the plain `actix_web::App` returned by `split_for_parts()` after this configure
/// call, so that `actix_files::Files` (which is not an `OpenApiFactory`) does not
/// interfere with path collection.
pub fn configure_app(
    proxy_svc: Arc<ProxyService>,
    admin_svc: Arc<AdminService>,
    token_repo: Arc<dyn UserTokenRepository>,
    pool: Option<PgPool>,
) -> impl Fn(&mut UtoipaServiceConfig) + Clone + 'static {
    move |cfg| {
        cfg.app_data(web::Data::new(proxy_svc.clone()));
        cfg.app_data(web::Data::new(admin_svc.clone()));
        cfg.app_data(web::Data::new(token_repo.clone()));
        if let Some(ref p) = pool {
            cfg.app_data(web::Data::new(p.clone()));
        }
        collect_routes(cfg);
    }
}
