pub mod error;
pub mod extractors;
pub mod handlers;
pub mod middleware;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use proxy_cache_core::entities::{Identity, Role};

/// Maps each role (and each dynamic group) to the set of registry names it can access.
///
/// Role inheritance: user inherits anonymous, admin inherits both.
/// Groups are additive — a user sees the union of their role's registries and each group's registries.
#[derive(Clone)]
pub struct AccessConfig {
    pub anonymous: HashSet<String>,
    pub user: HashSet<String>,
    pub admin: HashSet<String>,
    /// Dynamic group → registry names. Populated from `[registries.rbac.groups]`.
    pub groups: HashMap<String, HashSet<String>>,
}

impl AccessConfig {
    pub fn accessible_registries(&self, role: &Role) -> &HashSet<String> {
        match role {
            Role::Admin => &self.admin,
            Role::User => &self.user,
            Role::Anonymous => &self.anonymous,
        }
    }

    /// Returns the union of registries accessible via the caller's role and group memberships.
    /// Supports wildcard keys: `"*:team-a"` in the groups map matches `"oidc1:team-a"`,
    /// `"oidc2:team-a"`, `"kubernetes:team-a"`, etc.
    pub fn accessible_registries_for(&self, identity: &Identity) -> HashSet<String> {
        let mut result = self.accessible_registries(&identity.role).clone();
        for group in &identity.groups {
            // Exact match
            if let Some(registries) = self.groups.get(group) {
                result.extend(registries.iter().cloned());
            }
            // Wildcard match: "*:local-name" covers any provider prefix
            if let Some(colon) = group.find(':') {
                let wildcard = format!("*:{}", &group[colon + 1..]);
                if let Some(registries) = self.groups.get(&wildcard) {
                    result.extend(registries.iter().cloned());
                }
            }
        }
        result
    }

    pub fn has_registry_access(&self, identity: &Identity) -> bool {
        !self.accessible_registries_for(identity).is_empty()
    }
}

#[cfg(test)]
mod access_config_tests {
    use super::*;
    use proxy_cache_core::entities::Identity;

    fn make_config() -> AccessConfig {
        AccessConfig {
            anonymous: ["public"].iter().map(|s| s.to_string()).collect(),
            user: ["public", "user-only"].iter().map(|s| s.to_string()).collect(),
            admin: ["public", "user-only", "admin-only"].iter().map(|s| s.to_string()).collect(),
            groups: [
                ("team-a".to_owned(), ["group-a-reg"].iter().map(|s| s.to_string()).collect()),
                ("team-b".to_owned(), ["group-b-reg", "public"].iter().map(|s| s.to_string()).collect()),
            ]
            .into_iter()
            .collect(),
        }
    }

    fn identity(role: Role, groups: Vec<&str>) -> Identity {
        Identity {
            user_id: None,
            role,
            auth_provider: None,
            groups: groups.into_iter().map(str::to_owned).collect(),
        }
    }

    #[test]
    fn role_only_access_unchanged() {
        let cfg = make_config();
        let id = identity(Role::User, vec![]);
        let accessible = cfg.accessible_registries_for(&id);
        assert!(accessible.contains("public"));
        assert!(accessible.contains("user-only"));
        assert!(!accessible.contains("admin-only"));
        assert!(!accessible.contains("group-a-reg"));
    }

    #[test]
    fn group_membership_adds_group_registries() {
        let cfg = make_config();
        let id = identity(Role::Anonymous, vec!["team-a"]);
        let accessible = cfg.accessible_registries_for(&id);
        assert!(accessible.contains("group-a-reg"), "team-a should see group-a-reg");
        assert!(accessible.contains("public"), "anonymous role still applies");
        assert!(!accessible.contains("group-b-reg"), "team-a should not see group-b-reg");
    }

    #[test]
    fn multiple_groups_union() {
        let cfg = make_config();
        let id = identity(Role::Anonymous, vec!["team-a", "team-b"]);
        let accessible = cfg.accessible_registries_for(&id);
        assert!(accessible.contains("group-a-reg"));
        assert!(accessible.contains("group-b-reg"));
        assert!(accessible.contains("public"));
    }

    #[test]
    fn has_registry_access_via_group_only() {
        let _ = make_config(); // ensure it compiles
        // No role-based registries for anonymous, but group-a-reg is accessible via team-a.
        let anon_cfg = AccessConfig {
            anonymous: [].iter().cloned().collect(),
            user: [].iter().cloned().collect(),
            admin: [].iter().cloned().collect(),
            groups: [("team-a".to_owned(), ["group-a-reg".to_string()].into_iter().collect())]
                .into_iter()
                .collect(),
        };
        let id = identity(Role::Anonymous, vec!["team-a"]);
        assert!(anon_cfg.has_registry_access(&id));
    }

    #[test]
    fn has_registry_access_false_without_role_or_group_match() {
        let cfg = make_config();
        let id = identity(Role::Anonymous, vec!["team-c"]);
        let accessible = cfg.accessible_registries_for(&id);
        assert!(accessible.contains("public"));
        assert!(!accessible.contains("group-a-reg"));
        assert!(!accessible.contains("group-b-reg"));
    }

    #[test]
    fn group_overlap_with_role_no_duplicates() {
        // team-b includes "public", which anonymous role already grants — no issue.
        let cfg = make_config();
        let id = identity(Role::Anonymous, vec!["team-b"]);
        let accessible = cfg.accessible_registries_for(&id);
        assert_eq!(accessible.iter().filter(|r| *r == "public").count(), 1);
        assert!(accessible.contains("group-b-reg"));
    }

    fn make_wildcard_config() -> AccessConfig {
        AccessConfig {
            anonymous: HashSet::new(),
            user: HashSet::new(),
            admin: ["all-reg".to_owned()].into_iter().collect(),
            groups: [
                // Wildcard: any provider's "team-a" gets "shared-reg"
                ("*:team-a".to_owned(), ["shared-reg".to_owned()].into_iter().collect()),
                // Exact: only oidc2's "team-b" gets "oidc2-reg"
                ("oidc2:team-b".to_owned(), ["oidc2-reg".to_owned()].into_iter().collect()),
            ]
            .into_iter()
            .collect(),
        }
    }

    #[test]
    fn wildcard_matches_any_provider_prefix() {
        let cfg = make_wildcard_config();
        for group in &["oidc1:team-a", "oidc2:team-a", "kubernetes:team-a"] {
            let id = identity(Role::Anonymous, vec![group]);
            let accessible = cfg.accessible_registries_for(&id);
            assert!(
                accessible.contains("shared-reg"),
                "{group} should match *:team-a and access shared-reg"
            );
        }
    }

    #[test]
    fn exact_entry_not_matched_by_wrong_provider() {
        let cfg = make_wildcard_config();
        let id = identity(Role::Anonymous, vec!["oidc1:team-b"]);
        let accessible = cfg.accessible_registries_for(&id);
        assert!(!accessible.contains("oidc2-reg"), "oidc1:team-b should not match oidc2:team-b");
    }

    #[test]
    fn exact_entry_matched_by_correct_provider() {
        let cfg = make_wildcard_config();
        let id = identity(Role::Anonymous, vec!["oidc2:team-b"]);
        let accessible = cfg.accessible_registries_for(&id);
        assert!(accessible.contains("oidc2-reg"));
    }

    #[test]
    fn group_without_colon_skips_wildcard_lookup_safely() {
        let cfg = make_wildcard_config();
        let id = identity(Role::Anonymous, vec!["raw-group"]);
        let accessible = cfg.accessible_registries_for(&id);
        assert!(!accessible.contains("shared-reg"), "bare group name should not match wildcards");
    }

    #[test]
    fn multi_provider_user_gets_union_via_wildcard() {
        let cfg = make_wildcard_config();
        let id = identity(Role::Anonymous, vec!["oidc1:team-a", "oidc2:team-b"]);
        let accessible = cfg.accessible_registries_for(&id);
        assert!(accessible.contains("shared-reg"), "wildcard match for oidc1:team-a");
        assert!(accessible.contains("oidc2-reg"), "exact match for oidc2:team-b");
    }
}

/// Maps registry name → registry type (e.g. `"github1"` → `"github"`).
#[derive(Clone)]
pub struct RegistryMap(pub HashMap<String, String>);

impl RegistryMap {
    pub fn type_of(&self, name: &str) -> Option<&str> {
        self.0.get(name).map(String::as_str)
    }

    pub fn is_type(&self, name: &str, expected: &str) -> bool {
        self.type_of(name) == Some(expected)
    }

    /// Registry names with the given type, in insertion order.
    pub fn names_of_type<'a>(&'a self, registry_type: &'a str) -> impl Iterator<Item = &'a str> {
        self.0
            .iter()
            .filter(move |(_, t)| t.as_str() == registry_type)
            .map(|(n, _)| n.as_str())
    }
}

use actix_web::web;
use utoipa::OpenApi;
use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa_actix_web::{AppExt, service_config::ServiceConfig as UtoipaServiceConfig};
use utoipa_swagger_ui::SwaggerUi;

use sqlx::PgPool;

use proxy_cache_adapters::auth::OidcSsoFlow;
use proxy_cache_core::{
    ports::UserTokenRepository,
    services::{AdminService, ProxyService},
};

pub use handlers::proxy::cargo::CargoIndexProxy;
pub use middleware::AuthMiddlewareFactory;

#[derive(OpenApi)]
#[openapi(
    tags(
        (name = "proxy/github",   description = "GitHub proxy — releases, assets, tarballs, raw files"),
        (name = "proxy/npm",      description = "npm proxy — packuments, version metadata, tarballs"),
        (name = "proxy/cargo",    description = "Cargo proxy — sparse index, crate metadata, .crate downloads"),
        (name = "proxy/openvsx",  description = "OpenVSX proxy — VS Code extension metadata and VSIX packages"),
        (name = "proxy/goproxy",  description = "Go module proxy — version info, go.mod, and zip downloads"),
        (name = "front-office",   description = "User-facing package information"),
        (name = "back-office",    description = "Admin management (requires Admin role)"),
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
            oidc::{list_oidc_providers, oidc_callback, oidc_login, oidc_refresh},
            tokens::{create_token, list_tokens, revoke_token},
        },
        back_office::{
            audit::audit_log,
            health::{clear_registry_cache, registry_health},
            packages::{block_package, bulk_block_packages, bulk_unblock_packages, invalidate_package, list_packages as admin_list_packages, package_detail, unblock_package},
        },
        front_office::{
            me::me,
            packages::{check_access, list_packages},
            registries::list_registries,
        },
        proxy::{
            // Register most-specific patterns first so actix-web resolves correctly:
            // cargo index (literal "registry" segment) > github (owner/repo/verb) >
            // cargo download (literal "download") > openvsx vsix (literal "vsix") >
            // npm tarball (literal "tarball") > shared version metadata > shared packument
            cargo::{cargo_registry_config, cargo_registry_index, download_crate},
            github::{download_asset, download_asset_by_name, download_raw, download_tarball, download_zipball, get_release, list_releases},
            npm::{download_tarball as npm_download_tarball, get_packument, get_version},
            openvsx::download_vsix,
            goproxy::{goproxy_file, goproxy_latest, goproxy_list},
        },
    };

    cfg.service(list_oidc_providers);
    cfg.service(oidc_login);
    cfg.service(oidc_callback);
    cfg.service(oidc_refresh);
    cfg.service(create_token);
    cfg.service(list_tokens);
    cfg.service(revoke_token);
    // Cargo index (most specific — literal "registry" sub-path)
    cfg.service(cargo_registry_config);
    cfg.service(cargo_registry_index);
    // GitHub (owner/repo structure, multi-segment)
    cfg.service(list_releases);
    cfg.service(get_release);
    cfg.service(download_asset_by_name);
    cfg.service(download_asset);
    cfg.service(download_tarball);
    cfg.service(download_zipball);
    cfg.service(download_raw);
    // Cargo download (literal "download" suffix)
    cfg.service(download_crate);
    // Go module proxy (multi-segment module paths — must precede generic packument routes)
    cfg.service(goproxy_latest);
    cfg.service(goproxy_list);
    cfg.service(goproxy_file);
    // OpenVSX VSIX download (literal "vsix" suffix)
    cfg.service(download_vsix);
    // npm tarball (literal "tarball" suffix)
    cfg.service(npm_download_tarball);
    // Shared npm/cargo: version metadata then packument (more specific first)
    cfg.service(get_version);
    cfg.service(get_packument);
    cfg.service(me);
    cfg.service(list_registries);
    cfg.service(list_packages);
    cfg.service(check_access);
    cfg.service(admin_list_packages);
    cfg.service(package_detail);
    cfg.service(block_package);
    cfg.service(unblock_package);
    cfg.service(bulk_block_packages);
    cfg.service(bulk_unblock_packages);
    cfg.service(invalidate_package);
    cfg.service(registry_health);
    cfg.service(clear_registry_cache);
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
    access_config: AccessConfig,
    registry_map: RegistryMap,
    oidc_sso_flows: Vec<OidcSsoFlow>,
) -> impl Fn(&mut UtoipaServiceConfig) + Clone + 'static {
    move |cfg| {
        cfg.app_data(web::Data::new(proxy_svc.clone()));
        cfg.app_data(web::Data::new(admin_svc.clone()));
        cfg.app_data(web::Data::new(token_repo.clone()));
        cfg.app_data(web::Data::new(access_config.clone()));
        cfg.app_data(web::Data::new(registry_map.clone()));
        cfg.app_data(web::Data::new(oidc_sso_flows.clone()));
        if let Some(ref p) = pool {
            cfg.app_data(web::Data::new(p.clone()));
        }
        collect_routes(cfg);
    }
}
