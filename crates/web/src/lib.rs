pub mod badges;
pub mod error;
pub mod extractors;
pub mod handlers;
pub mod middleware;
pub mod services;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use batlehub_config::schema::RegistryMode;
use batlehub_core::entities::{Identity, Role};

/// Shared, hot-reloadable access config — updated atomically on config reload.
pub type AccessConfigLock = Arc<tokio::sync::RwLock<AccessConfig>>;

/// Convenience constructor for `AccessConfigLock`.
pub fn new_access_lock(cfg: AccessConfig) -> AccessConfigLock {
    Arc::new(tokio::sync::RwLock::new(cfg))
}

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
    /// Registries where each role can browse/search in the package explorer.
    /// Always a subset of the corresponding proxy-access set.
    pub explore_anonymous: HashSet<String>,
    pub explore_user: HashSet<String>,
    pub explore_admin: HashSet<String>,
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
            if let Some((_, local_name)) = group.split_once(':') {
                let wildcard = format!("*:{local_name}");
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

    fn explore_registries(&self, role: &Role) -> &HashSet<String> {
        match role {
            Role::Admin => &self.explore_admin,
            Role::User => &self.explore_user,
            Role::Anonymous => &self.explore_anonymous,
        }
    }

    /// Returns the set of registries the caller can browse/search in the package explorer.
    /// Groups inherit their proxy access for explore (no separate group-level explore restriction).
    pub fn explore_accessible_registries_for(&self, identity: &Identity) -> HashSet<String> {
        let proxy = self.accessible_registries_for(identity);
        let explore = self.explore_registries(&identity.role);
        proxy.intersection(explore).cloned().collect()
    }
}

#[cfg(test)]
mod access_config_tests {
    use super::*;
    use batlehub_core::entities::Identity;

    fn make_config() -> AccessConfig {
        let regs: HashSet<String> = [
            "public",
            "user-only",
            "admin-only",
            "group-a-reg",
            "group-b-reg",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        AccessConfig {
            anonymous: ["public"].iter().map(|s| s.to_string()).collect(),
            user: ["public", "user-only"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            admin: ["public", "user-only", "admin-only"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            groups: [
                (
                    "team-a".to_owned(),
                    ["group-a-reg"].iter().map(|s| s.to_string()).collect(),
                ),
                (
                    "team-b".to_owned(),
                    ["group-b-reg", "public"]
                        .iter()
                        .map(|s| s.to_string())
                        .collect(),
                ),
            ]
            .into_iter()
            .collect(),
            explore_anonymous: regs.clone(),
            explore_user: regs.clone(),
            explore_admin: regs,
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
        assert!(
            accessible.contains("group-a-reg"),
            "team-a should see group-a-reg"
        );
        assert!(
            accessible.contains("public"),
            "anonymous role still applies"
        );
        assert!(
            !accessible.contains("group-b-reg"),
            "team-a should not see group-b-reg"
        );
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
        // No role-based registries for anonymous, but group-a-reg is accessible via team-a.
        let anon_cfg = AccessConfig {
            anonymous: [].iter().cloned().collect(),
            user: [].iter().cloned().collect(),
            admin: [].iter().cloned().collect(),
            groups: [(
                "team-a".to_owned(),
                ["group-a-reg".to_string()].into_iter().collect(),
            )]
            .into_iter()
            .collect(),
            explore_anonymous: HashSet::new(),
            explore_user: HashSet::new(),
            explore_admin: HashSet::new(),
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
        let all: HashSet<String> = ["all-reg", "shared-reg", "oidc2-reg"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        AccessConfig {
            anonymous: HashSet::new(),
            user: HashSet::new(),
            admin: ["all-reg".to_owned()].into_iter().collect(),
            groups: [
                // Wildcard: any provider's "team-a" gets "shared-reg"
                (
                    "*:team-a".to_owned(),
                    ["shared-reg".to_owned()].into_iter().collect(),
                ),
                // Exact: only oidc2's "team-b" gets "oidc2-reg"
                (
                    "oidc2:team-b".to_owned(),
                    ["oidc2-reg".to_owned()].into_iter().collect(),
                ),
            ]
            .into_iter()
            .collect(),
            explore_anonymous: all.clone(),
            explore_user: all.clone(),
            explore_admin: all,
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
        assert!(
            !accessible.contains("oidc2-reg"),
            "oidc1:team-b should not match oidc2:team-b"
        );
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
        assert!(
            !accessible.contains("shared-reg"),
            "bare group name should not match wildcards"
        );
    }

    #[test]
    fn multi_provider_user_gets_union_via_wildcard() {
        let cfg = make_wildcard_config();
        let id = identity(Role::Anonymous, vec!["oidc1:team-a", "oidc2:team-b"]);
        let accessible = cfg.accessible_registries_for(&id);
        assert!(
            accessible.contains("shared-reg"),
            "wildcard match for oidc1:team-a"
        );
        assert!(
            accessible.contains("oidc2-reg"),
            "exact match for oidc2:team-b"
        );
    }
}

/// Maps registry name → registry type (e.g. `"github1"` → `"github"`).
///
/// The inner `HashMap` is behind `Arc<RwLock<>>` so the hot-reload path can swap
/// registry metadata without restarting actix workers. `Clone` shares the same lock.
#[derive(Clone)]
pub struct RegistryMap(pub Arc<std::sync::RwLock<HashMap<String, String>>>);

impl RegistryMap {
    pub fn new(map: HashMap<String, String>) -> Self {
        Self(Arc::new(std::sync::RwLock::new(map)))
    }

    pub fn type_of(&self, name: &str) -> Option<String> {
        self.0
            .read()
            .expect("registry map lock poisoned")
            .get(name)
            .cloned()
    }

    pub fn is_type(&self, name: &str, expected: &str) -> bool {
        self.type_of(name).as_deref() == Some(expected)
    }

    pub fn contains(&self, name: &str) -> bool {
        self.0
            .read()
            .expect("registry map lock poisoned")
            .contains_key(name)
    }

    pub fn keys(&self) -> Vec<String> {
        self.0
            .read()
            .expect("registry map lock poisoned")
            .keys()
            .cloned()
            .collect()
    }

    /// Registry names with the given type.
    pub fn names_of_type(&self, registry_type: &str) -> Vec<String> {
        self.0
            .read()
            .expect("registry map lock poisoned")
            .iter()
            .filter(|(_, t)| t.as_str() == registry_type)
            .map(|(n, _)| n.clone())
            .collect()
    }
}

impl From<HashMap<String, String>> for RegistryMap {
    fn from(map: HashMap<String, String>) -> Self {
        Self::new(map)
    }
}

/// Maps registry name → configured `RegistryMode` (proxy / local / hybrid).
///
/// Inner `HashMap` is behind `Arc<RwLock<>>` for the same hot-reload reason as [`RegistryMap`].
#[derive(Clone)]
pub struct RegistryModeMap(pub Arc<std::sync::RwLock<HashMap<String, RegistryMode>>>);

impl RegistryModeMap {
    pub fn new(map: HashMap<String, RegistryMode>) -> Self {
        Self(Arc::new(std::sync::RwLock::new(map)))
    }

    pub fn get(&self, name: &str) -> RegistryMode {
        self.0
            .read()
            .expect("registry mode map lock poisoned")
            .get(name)
            .cloned()
            .unwrap_or_default()
    }

    pub fn insert(&self, name: String, mode: RegistryMode) {
        self.0
            .write()
            .expect("registry mode map lock poisoned")
            .insert(name, mode);
    }
}

impl Default for RegistryModeMap {
    fn default() -> Self {
        Self::new(HashMap::new())
    }
}

impl From<HashMap<String, RegistryMode>> for RegistryModeMap {
    fn from(map: HashMap<String, RegistryMode>) -> Self {
        Self::new(map)
    }
}

/// Maps a `deb`/`rpm` registry name → its repository-metadata signing key, when
/// configured. Registries absent from the map host **unsigned** repositories
/// (clients must use `[trusted=yes]` / `gpgcheck=0`).
///
/// Inner `HashMap` is behind `Arc<RwLock<>>` for the same hot-reload reason as [`RegistryMap`].
#[derive(Clone, Default)]
pub struct RepoSignerMap(
    pub Arc<std::sync::RwLock<HashMap<String, Arc<batlehub_adapters::repo::OpenPgpSigner>>>>,
);

impl RepoSignerMap {
    pub fn get(&self, name: &str) -> Option<Arc<batlehub_adapters::repo::OpenPgpSigner>> {
        self.0
            .read()
            .expect("repo signer map lock poisoned")
            .get(name)
            .cloned()
    }
}

impl From<HashMap<String, Arc<batlehub_adapters::repo::OpenPgpSigner>>> for RepoSignerMap {
    fn from(map: HashMap<String, Arc<batlehub_adapters::repo::OpenPgpSigner>>) -> Self {
        Self(Arc::new(std::sync::RwLock::new(map)))
    }
}

/// Maps npm/terraform/pypi/conda registry name → first upstream base URL (for audit pass-through).
///
/// Inner `HashMap` is behind `Arc<RwLock<>>` for the same hot-reload reason as [`RegistryMap`].
#[derive(Clone)]
pub struct UpstreamMap(pub Arc<std::sync::RwLock<HashMap<String, String>>>);

impl UpstreamMap {
    pub fn new(map: HashMap<String, String>) -> Self {
        Self(Arc::new(std::sync::RwLock::new(map)))
    }

    pub fn upstream_for(&self, name: &str) -> Option<String> {
        self.0
            .read()
            .expect("upstream map lock poisoned")
            .get(name)
            .cloned()
    }
}

impl Default for UpstreamMap {
    fn default() -> Self {
        Self::new(HashMap::new())
    }
}

impl From<HashMap<String, String>> for UpstreamMap {
    fn from(map: HashMap<String, String>) -> Self {
        Self::new(map)
    }
}

/// Maps Cargo registry name → [`CargoIndexProxy`] (sparse-index HTTP client + URL).
///
/// Inner `HashMap` is behind `Arc<RwLock<>>` so hot-reload can swap proxy settings
/// without restarting actix workers. Clone shares the same lock.
#[derive(Clone, Default)]
pub struct CargoIndexMap(pub Arc<std::sync::RwLock<HashMap<String, CargoIndexProxy>>>);

impl CargoIndexMap {
    pub fn new(map: HashMap<String, CargoIndexProxy>) -> Self {
        Self(Arc::new(std::sync::RwLock::new(map)))
    }

    /// Clone the proxy for the given registry name, if configured.
    pub fn get(&self, name: &str) -> Option<CargoIndexProxy> {
        self.0
            .read()
            .expect("cargo index map lock poisoned")
            .get(name)
            .cloned()
    }
}

impl From<HashMap<String, CargoIndexProxy>> for CargoIndexMap {
    fn from(map: HashMap<String, CargoIndexProxy>) -> Self {
        Self::new(map)
    }
}

use actix_web::web;
use handlers::back_office::eviction::EvictionServiceMap;
use handlers::back_office::warming::WarmingServiceMap;
use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa::OpenApi;
use utoipa_actix_web::{service_config::ServiceConfig as UtoipaServiceConfig, AppExt};
use utoipa_scalar::{Scalar, Servable as _};

use sqlx::PgPool;

use batlehub_adapters::auth::OidcSsoFlow;
use batlehub_core::{
    ports::UserTokenRepository,
    services::{AdminService, ProxyMetrics, ProxyService, SbomService},
};
use metrics_exporter_prometheus::PrometheusHandle;

pub use handlers::front_office::cli_download::CliBinaryPath;
pub use handlers::healthz::healthz;
pub use handlers::metrics::prometheus_metrics;
pub use handlers::proxy::cargo::CargoIndexProxy;
pub use middleware::AuthMiddlewareFactory;
pub use middleware::IpBlockMiddlewareFactory;
pub use middleware::RateLimitMiddlewareFactory;
pub use middleware::RateLimitService;

#[derive(OpenApi)]
#[openapi(
    tags(
        (name = "proxy/github",   description = "GitHub proxy — releases, assets, tarballs, raw files (also serves Forgejo/Gitea registries, which share this URL scheme)"),
        (name = "proxy/gitlab",   description = "GitLab proxy — releases, release link assets, and source archives"),
        (name = "proxy/deb",      description = "Debian APT repository — proxy + local hosting (Packages/Release generation, Ed25519 OpenPGP signing)"),
        (name = "proxy/rpm",      description = "RPM/YUM repository — proxy + local hosting (repodata generation, Ed25519 OpenPGP signing)"),
        (name = "proxy/pacman",   description = "Arch Linux pacman repository — proxy + local hosting (.pkg.tar.zst, repo DB generation, Ed25519 OpenPGP signing)"),
        (name = "proxy/npm",      description = "npm proxy — packuments, version metadata, tarballs"),
        (name = "proxy/cargo",    description = "Cargo proxy — sparse index, crate metadata, .crate downloads"),
        (name = "proxy/openvsx",  description = "OpenVSX proxy — VS Code extension metadata and VSIX packages"),
        (name = "proxy/goproxy",    description = "Go module proxy — version info, go.mod, and zip downloads"),
        (name = "proxy/terraform",  description = "Terraform registry — provider and module proxy, private module/provider publishing"),
        (name = "proxy/rubygems",   description = "RubyGems registry — gem downloads, version listing, and private gem publishing"),
        (name = "proxy/pypi",       description = "PyPI registry — simple index proxy with URL rewriting, wheel/sdist downloads, and twine-compatible publish"),
        (name = "proxy/conda",      description = "Conda channel proxy — repodata.json, package downloads, and private channel publishing"),
        (name = "proxy/nuget",      description = "NuGet registry — service index, flat container, registration metadata, .nupkg download, and private package publishing"),
        (name = "front-office",     description = "User-facing package information"),
        (name = "explore",          description = "Package explorer — browse and search across registries"),
        (name = "back-office",    description = "Admin management (requires Admin role)"),
        (name = "notifications",  description = "Inbound webhook receiver — accepts events from external systems"),
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
            beta_channel::{add_beta_member, list_beta_members, remove_beta_member},
            bulk::{
                bulk_delete, bulk_unyank, bulk_yank as bulk_yank_handler, deprecate, relist,
                undeprecate, unlist,
            },
            config::{
                apply_pending_reload, clear_banner, discard_pending_reload, get_pending_reload,
                list_config_changes, reload_config, set_banner,
            },
            eviction::evict_registry,
            explore::invalidate_explore_cache,
            health::{clear_registry_cache, registry_health},
            ip_blocks::{block_ip, list_blocked_ips, unblock_ip},
            notification::{
                create_subscription, delete_subscription, get_subscription,
                list_notification_channels, list_subscriptions, test_subscription,
                update_subscription,
            },
            ownership::{add_package_owner, list_package_owners, remove_package_owner},
            packages::{
                block_package, bulk_block_packages, bulk_unblock_packages, invalidate_package,
                list_packages as admin_list_packages, package_detail, unblock_package,
            },
            quota::{
                get_quota_for_user, list_quota, list_quota_for_registry, reset_quota_for_user,
            },
            sbom::{export_org_sbom, get_artifact_sbom},
            stats::admin_stats,
            team_namespaces::{
                claim_namespace, list_namespaces, my_namespace_packages, my_namespaces,
                release_namespace,
            },
            visibility::{get_package_visibility, set_package_visibility},
            warming::warm_registry,
        },
        front_office::{
            banner::get_banner,
            cli_download::download_cli,
            explore::{
                explore_package_detail, explore_packages, explore_registry_stats,
                explore_upstream_search,
            },
            me::me,
            packages::{check_access, list_packages},
            registries::list_registries,
        },
        inbound_webhook::{list_inbound_events, receive_inbound_webhook},
        proxy::{
            cargo::{
                cargo_owners, cargo_publish, cargo_registry_config, cargo_registry_index,
                cargo_unyank, cargo_yank, download_crate,
            },
            composer::{
                composer_dist, composer_p2_metadata, composer_packages_json, composer_upload,
                composer_yank,
            },
            // Register most-specific patterns first so actix-web resolves correctly:
            // cargo api/v1 (literal "api" segment) > cargo index (literal "registry" segment) >
            // github (owner/repo/verb) > cargo download (literal "download") >
            // maven (literal "maven2" segment) >
            // openvsx vsix (literal "vsix") > npm audit (literal "/-/npm/v1/audit/quick") >
            // npm tarball (literal "tarball") > shared version metadata > shared packument
            // composer: upload/yank (literal "api") > p2 (literal "p2") > dist > packages.json
            conda::{conda_current_repodata, conda_file_download, conda_publish, conda_repodata},
            forgejo::fj_packages,
            github::{
                download_asset, download_asset_by_name, download_raw, download_tarball,
                download_zipball, get_release, list_releases,
            },
            gitlab::{
                gl_download_archive, gl_download_link, gl_download_raw, gl_get_release,
                gl_list_releases, gl_packages,
            },
            goproxy::{goproxy_file, goproxy_latest, goproxy_list, goproxy_publish},
            jetbrains::jetbrains_get,
            maven::{maven_get, maven_put},
            npm::{
                audit_quick, download_tarball as npm_download_tarball, get_packument, get_version,
                npm_publish,
            },
            nuget::{
                nuget_flat_download, nuget_flat_versions, nuget_publish, nuget_registration,
                nuget_search, nuget_service_index, nuget_yank,
            },
            openvsx::{download_vsix, vsix_publish},
            pypi::{pypi_file_download, pypi_publish, pypi_simple_package, pypi_simple_root},
            repo::{
                deb_get, pacman_get,
                publish::{deb_publish, pacman_publish, rpm_publish},
                rpm_get,
            },
            rubygems::{
                gem_download, gem_gemspec, gem_info, gem_publish, gem_specs_full, gem_specs_latest,
                gem_specs_prerelease, gem_unyank, gem_versions, gem_yank,
            },
            terraform::{
                tf_module_artifact, tf_module_download, tf_module_unyank, tf_module_upload,
                tf_module_versions, tf_module_yank, tf_provider_artifact,
                tf_provider_binary_upload, tf_provider_download, tf_provider_unyank,
                tf_provider_upload, tf_provider_versions, tf_provider_yank,
            },
        },
    };

    cfg.service(list_oidc_providers);
    cfg.service(oidc_login);
    cfg.service(oidc_callback);
    cfg.service(oidc_refresh);
    cfg.service(create_token);
    cfg.service(list_tokens);
    cfg.service(revoke_token);
    // Cargo publish API (literal "api/v1" sub-path — most specific, must precede download)
    cfg.service(cargo_publish);
    cfg.service(cargo_yank);
    cfg.service(cargo_unyank);
    cfg.service(cargo_owners);
    // Cargo index (literal "registry" sub-path)
    cfg.service(cargo_registry_config);
    cfg.service(cargo_registry_index);
    // Forgejo/GitLab package registries: literal `api/…` prefix — register before
    // the GitHub `{owner}/{repo}` routes so it isn't captured as owner="api".
    cfg.service(fj_packages); // GET …/api/packages/{path}  (Forgejo/Gitea)
    cfg.service(gl_packages); // GET …/api/v4/{path}         (GitLab)
                              // GitHub (owner/repo structure, multi-segment) — also serves Forgejo releases.
    cfg.service(list_releases);
    cfg.service(get_release);
    cfg.service(download_asset_by_name);
    cfg.service(download_asset);
    cfg.service(download_tarball);
    cfg.service(download_zipball);
    cfg.service(download_raw);
    // GitLab (distinct `/-/` delimiter; most-specific first)
    cfg.service(gl_download_link); // …/-/releases/{tag}/downloads/{name}
    cfg.service(gl_get_release); // …/-/releases/{tag}
    cfg.service(gl_list_releases); // …/-/releases
    cfg.service(gl_download_archive); // …/-/archive/{tag}/{filename}
    cfg.service(gl_download_raw); // …/-/raw/{ref}/{path}
                                  // Deb / RPM repositories: publish (PUT) before the catch-all read (GET).
    cfg.service(deb_publish); // PUT …/deb/pool/{dist}/{component}/upload
    cfg.service(rpm_publish); // PUT …/rpm/upload
    cfg.service(deb_get); // GET …/deb/{path}
    cfg.service(rpm_get); // GET …/rpm/{path}
    cfg.service(pacman_publish); // PUT …/pacman/upload
    cfg.service(pacman_get); // GET …/pacman/{path}
    cfg.service(jetbrains_get); // GET …/jetbrains/{path} (proxy-only cache)
                                // Cargo download (literal "download" suffix)
    cfg.service(download_crate);
    // Go module proxy (multi-segment module paths — must precede generic packument routes)
    // PUT goproxy_publish must come before GET goproxy_file (same path pattern, different method)
    cfg.service(goproxy_publish);
    cfg.service(goproxy_latest);
    cfg.service(goproxy_list);
    cfg.service(goproxy_file);
    // Maven — PUT before GET (same path pattern, different method)
    cfg.service(maven_put);
    cfg.service(maven_get);
    // NuGet: publish (PUT) and yank (DELETE) before read routes; literal paths before wildcards
    cfg.service(nuget_publish); // PUT  .../api/v2/package
    cfg.service(nuget_yank); // DELETE .../v2/package/{id}/{version}
    cfg.service(nuget_service_index); // GET .../v3/index.json
    cfg.service(nuget_registration); // GET .../v3/registration5/{id}/index.json
    cfg.service(nuget_flat_versions); // GET .../v3/flat/{id}/index.json
    cfg.service(nuget_search); // GET .../v3/query
    cfg.service(nuget_flat_download); // GET .../v3/flat/{id}/{version}/{filename}
                                      // Terraform modules — longer paths first (unyank > yank > artifact > upload > download > versions)
    cfg.service(tf_module_unyank); // POST …/versions/{ver}/unyank
    cfg.service(tf_module_yank); // DELETE …/versions/{ver}
    cfg.service(tf_module_artifact); // GET …/{ver}/artifact
    cfg.service(tf_module_upload); // POST …/{ver}
    cfg.service(tf_module_download); // GET …/{ver}/download
    cfg.service(tf_module_versions); // GET …/versions
                                     // Terraform providers — binary PUT/GET before download, unyank/yank before upload/versions
    cfg.service(tf_provider_unyank); // POST …/versions/{ver}/unyank
    cfg.service(tf_provider_yank); // DELETE …/versions/{ver}
    cfg.service(tf_provider_binary_upload); // PUT …/{ver}/artifact/{os}/{arch}
    cfg.service(tf_provider_artifact); // GET …/{ver}/artifact/{os}/{arch}
    cfg.service(tf_provider_download); // GET …/{ver}/download/{os}/{arch}
    cfg.service(tf_provider_upload); // POST …/versions (write)
    cfg.service(tf_provider_versions); // GET …/versions
                                       // RubyGems — yank/unyank/publish before download (same /api/v1/gems prefix, different methods)
    cfg.service(gem_yank);
    cfg.service(gem_unyank);
    cfg.service(gem_publish);
    // gemspec (literal "quick/Marshal.4.8") before generic gem download
    cfg.service(gem_gemspec);
    cfg.service(gem_download);
    cfg.service(gem_info);
    cfg.service(gem_versions);
    cfg.service(gem_specs_full);
    cfg.service(gem_specs_latest);
    cfg.service(gem_specs_prerelease);
    // Composer: literal "api" routes before "p2" before "dist" before "packages.json"
    cfg.service(composer_upload); // POST …/api/upload
    cfg.service(composer_yank); // DELETE …/api/packages/{vendor}/{package}/versions/{version}
    cfg.service(composer_p2_metadata); // GET …/p2/{path:.*}
    cfg.service(composer_dist); // GET …/dist/{vendor}/{package}/{version}
    cfg.service(composer_packages_json); // GET …/packages.json
                                         // PyPI: publish (POST /legacy/) before simple package (GET /simple/{pkg}/) before root (GET /simple/) before file download
    cfg.service(pypi_publish); // POST …/legacy/
    cfg.service(pypi_simple_package); // GET …/simple/{package}/
    cfg.service(pypi_simple_root); // GET …/simple/
    cfg.service(pypi_file_download); // GET …/packages/{filename}
                                     // Conda: literal repodata routes before wildcard file download; publish (POST) before GET
    cfg.service(conda_publish); // POST …/{platform}/
    cfg.service(conda_repodata); // GET …/{platform}/repodata.json
    cfg.service(conda_current_repodata); // GET …/{platform}/current_repodata.json
    cfg.service(conda_file_download); // GET …/{platform}/{filename}
                                      // OpenVSX/VSCode VSIX publish (PUT) and download (GET) — same path, different method
    cfg.service(vsix_publish);
    cfg.service(download_vsix);
    // npm audit pass-through (literal "/-/npm/v1/audit/quick" path)
    cfg.service(audit_quick);
    // npm tarball (literal "tarball" suffix)
    cfg.service(npm_download_tarball);
    // npm publish (PUT same path as packument — different method, registered before GET)
    cfg.service(npm_publish);
    // Shared npm/cargo: version metadata then packument (more specific first)
    cfg.service(get_version);
    cfg.service(get_packument);
    cfg.service(me);
    cfg.service(download_cli);
    cfg.service(list_registries);
    // Explore: detail path before list (more specific first); upstream before list
    cfg.service(explore_package_detail);
    cfg.service(explore_upstream_search);
    cfg.service(explore_packages);
    cfg.service(explore_registry_stats);
    cfg.service(list_packages);
    cfg.service(check_access);
    cfg.service(invalidate_explore_cache);
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
    cfg.service(warm_registry);
    cfg.service(evict_registry);
    cfg.service(admin_stats);
    // Quota admin (specific user route before registry-level route)
    cfg.service(reset_quota_for_user);
    cfg.service(get_quota_for_user);
    cfg.service(list_quota_for_registry);
    cfg.service(list_quota);
    // Ownership admin
    cfg.service(list_package_owners);
    cfg.service(add_package_owner);
    cfg.service(remove_package_owner);
    // Package visibility admin (wildcard {name:.*} — registered after literal-suffix /owners routes)
    cfg.service(get_package_visibility);
    cfg.service(set_package_visibility);
    // Team namespace admin
    cfg.service(list_namespaces);
    cfg.service(claim_namespace);
    cfg.service(release_namespace); // wildcard {prefix:.*}
                                    // Team namespace user-facing
    cfg.service(my_namespaces);
    cfg.service(my_namespace_packages); // wildcard {prefix:.*}
                                        // Bulk operations admin
    cfg.service(bulk_yank_handler);
    cfg.service(bulk_unyank);
    cfg.service(bulk_delete);
    // Deprecation & unlisting admin (single version)
    cfg.service(deprecate);
    cfg.service(undeprecate);
    cfg.service(unlist);
    cfg.service(relist);
    // Beta channel admin
    cfg.service(list_beta_members);
    cfg.service(add_beta_member);
    cfg.service(remove_beta_member);
    // IP block admin
    cfg.service(list_blocked_ips);
    cfg.service(block_ip);
    cfg.service(unblock_ip);
    // Config reload admin (pending/apply before pending/delete — more specific first)
    cfg.service(reload_config);
    cfg.service(apply_pending_reload);
    cfg.service(get_pending_reload);
    cfg.service(discard_pending_reload);
    cfg.service(list_config_changes);
    // Banner admin + public
    cfg.service(set_banner);
    cfg.service(clear_banner);
    cfg.service(get_banner);
    // SBOM: export (literal "export") before per-artifact (parameterised path)
    cfg.service(export_org_sbom);
    cfg.service(get_artifact_sbom);
    // Notifications admin (subscriptions/{id}/test before subscriptions/{id} — more specific first)
    cfg.service(list_notification_channels);
    cfg.service(list_subscriptions);
    cfg.service(create_subscription);
    cfg.service(test_subscription);
    cfg.service(get_subscription);
    cfg.service(update_subscription);
    cfg.service(delete_subscription);
    cfg.service(list_inbound_events);
    // Inbound webhooks (public-facing — no admin auth required)
    cfg.service(receive_inbound_webhook);
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

/// Return a Scalar API docs service using the provided OpenAPI spec.
pub fn scalar(openapi: utoipa::openapi::OpenApi) -> Scalar<utoipa::openapi::OpenApi> {
    Scalar::with_url("/scalar", openapi)
}

/// Configure all application routes on a `UtoipaApp`.
///
/// Static file serving (SPA fallback) is intentionally excluded — register it on
/// the plain `actix_web::App` returned by `split_for_parts()` after this configure
/// call, so that `actix_files::Files` (which is not an `OpenApiFactory`) does not
/// interfere with path collection.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
pub fn configure_app(
    proxy_svc: Arc<ProxyService>,
    admin_svc: Arc<AdminService>,
    token_repo: Arc<dyn UserTokenRepository>,
    pool: Option<PgPool>,
    access_config: Arc<tokio::sync::RwLock<AccessConfig>>,
    registry_map: RegistryMap,
    upstream_map: UpstreamMap,
    oidc_sso_flows: Vec<OidcSsoFlow>,
    warming_map: WarmingServiceMap,
    eviction_map: EvictionServiceMap,
    proxy_metrics: Arc<ProxyMetrics>,
    prometheus_handle: Option<PrometheusHandle>,
    sbom_svc: Option<Arc<SbomService>>,
    notification_svc: Option<Arc<services::NotificationService>>,
    notification_store: Arc<dyn batlehub_core::ports::NotificationPort + 'static>,
    notifications_config: Option<batlehub_config::schema::NotificationsConfig>,
) -> impl Fn(&mut UtoipaServiceConfig) + Clone + 'static {
    let audit_client = reqwest::Client::builder()
        .user_agent("batlehub/0.1")
        .build()
        .expect("audit HTTP client");
    move |cfg| {
        cfg.app_data(web::Data::new(proxy_svc.clone()));
        cfg.app_data(web::Data::new(admin_svc.clone()));
        cfg.app_data(web::Data::new(token_repo.clone()));
        cfg.app_data(web::Data::new(Arc::clone(&access_config)));
        cfg.app_data(web::Data::new(registry_map.clone()));
        cfg.app_data(web::Data::new(upstream_map.clone()));
        cfg.app_data(web::Data::new(audit_client.clone()));
        cfg.app_data(web::Data::new(oidc_sso_flows.clone()));
        cfg.app_data(web::Data::new(warming_map.clone()));
        cfg.app_data(web::Data::new(eviction_map.clone()));
        cfg.app_data(web::Data::new(proxy_metrics.clone()));
        if let Some(ref h) = prometheus_handle {
            cfg.app_data(web::Data::new(h.clone()));
        }
        if let Some(ref p) = pool {
            cfg.app_data(web::Data::new(p.clone()));
        }
        if let Some(ref s) = sbom_svc {
            cfg.app_data(web::Data::new(s.clone()));
        }
        // Always register as Option so handlers can extract without a 500 when disabled.
        cfg.app_data(web::Data::new(notification_svc.clone()));
        cfg.app_data(web::Data::new(notification_store.clone()));
        cfg.app_data(web::Data::new(notifications_config.clone()));
        collect_routes(cfg);
    }
}
