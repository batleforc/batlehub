pub mod error;
pub mod extractors;
pub mod handlers;
pub mod middleware;

use std::sync::Arc;

use actix_web::web;
use utoipa::OpenApi;
use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa_swagger_ui::SwaggerUi;

use proxy_cache_core::services::{AdminService, ProxyService};

pub use middleware::AuthMiddlewareFactory;

#[derive(OpenApi)]
#[openapi(
    paths(
        handlers::proxy::github::list_releases,
        handlers::proxy::github::get_release,
        handlers::proxy::github::download_asset,
        handlers::proxy::github::download_tarball,
        handlers::proxy::npm::get_packument,
        handlers::proxy::npm::get_version,
        handlers::proxy::npm::download_tarball,
        handlers::proxy::cargo::get_crate,
        handlers::proxy::cargo::get_version,
        handlers::proxy::cargo::download_crate,
        handlers::front_office::me::me,
        handlers::front_office::packages::list_packages,
        handlers::front_office::packages::check_access,
        handlers::back_office::packages::list_packages,
        handlers::back_office::packages::block_package,
        handlers::back_office::packages::unblock_package,
        handlers::back_office::audit::audit_log,
    ),
    components(schemas(
        proxy_cache_core::entities::Role,
        handlers::front_office::me::MeResponse,
        handlers::front_office::packages::PackageListResponse,
        handlers::front_office::packages::PackageSummaryDto,
        handlers::front_office::packages::PackageStatusDto,
        handlers::front_office::packages::AccessCheckResponse,
        handlers::front_office::packages::PackageIdentifierDto,
        handlers::back_office::packages::BlockRequest,
        handlers::back_office::packages::UnblockRequest,
        handlers::back_office::packages::ActionResponse,
    )),
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

/// Return the raw OpenAPI JSON spec.
pub fn openapi_spec() -> utoipa::openapi::OpenApi {
    ApiDoc::openapi()
}

/// Return the Swagger UI service mounted at `/swagger-ui/` and `/api/openapi.json`.
pub fn swagger_ui() -> SwaggerUi {
    SwaggerUi::new("/swagger-ui/{_:.*}").url("/api/openapi.json", ApiDoc::openapi())
}

/// Configure all routes on an `actix_web::App`.
pub fn configure_app(
    proxy_svc: Arc<ProxyService>,
    admin_svc: Arc<AdminService>,
    static_dir: Option<String>,
) -> impl Fn(&mut web::ServiceConfig) + Clone + 'static {
    use handlers::{
        back_office::{audit::audit_log, packages::{block_package, list_packages as admin_list_packages, unblock_package}},
        front_office::{me::me, packages::{check_access, list_packages}},
        proxy::{
            cargo::{download_crate, get_crate, get_version as cargo_get_version},
            github::{download_asset, download_tarball, get_release, list_releases},
            npm::{download_tarball as npm_download_tarball, get_packument, get_version as npm_get_version},
        },
    };

    move |cfg: &mut web::ServiceConfig| {
        // Shared state
        cfg.app_data(web::Data::new(proxy_svc.clone()));
        cfg.app_data(web::Data::new(admin_svc.clone()));

        // GitHub proxy
        cfg.service(list_releases);
        cfg.service(get_release);
        cfg.service(download_asset);
        cfg.service(download_tarball);

        // npm proxy
        cfg.service(get_packument);
        cfg.service(npm_get_version);
        cfg.service(npm_download_tarball);

        // Cargo proxy
        cfg.service(get_crate);
        cfg.service(cargo_get_version);
        cfg.service(download_crate);

        // Front office
        cfg.service(me);
        cfg.service(list_packages);
        cfg.service(check_access);

        // Back office
        cfg.service(admin_list_packages);
        cfg.service(block_package);
        cfg.service(unblock_package);
        cfg.service(audit_log);

        // Static SPA files (must come last so API routes take precedence)
        if let Some(ref dir) = static_dir {
            cfg.service(
                actix_files::Files::new("/", dir)
                    .index_file("index.html")
                    .use_last_modified(true),
            );
        }
    }
}
