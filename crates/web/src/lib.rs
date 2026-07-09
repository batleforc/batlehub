pub mod access;
pub mod badges;
pub mod error;
pub mod extractors;
pub mod handlers;
pub mod middleware;
pub mod services;

pub use access::{new_access_lock, AccessConfig, AccessConfigLock};

use std::collections::HashMap;
use std::sync::Arc;

use batlehub_config::schema::RegistryMode;

/// A registry-name-keyed `HashMap` behind `Arc<RwLock<>>`, reused by every
/// lookup table below so hot-reload can swap entries without restarting actix
/// workers. `Clone` shares the same lock (all clones see the same data).
///
/// The six domain types below (`RegistryMap`, `RegistryModeMap`, `RepoSignerMap`,
/// `UpstreamMap`, `VulnDbMap`, `CargoIndexMap`) each wrap one of these plus only
/// their own domain-specific accessor methods (`type_of`, `upstream_for`, …) —
/// no downstream call site needs to change, since those six public type names
/// and methods stay the same; only their formerly-duplicated lock/clone
/// boilerplate moves here.
#[derive(Clone)]
struct LockedMap<V>(Arc<std::sync::RwLock<HashMap<String, V>>>);

impl<V: Clone> LockedMap<V> {
    fn new(map: HashMap<String, V>) -> Self {
        Self(Arc::new(std::sync::RwLock::new(map)))
    }

    fn get(&self, key: &str) -> Option<V> {
        self.0
            .read()
            .expect("locked map lock poisoned")
            .get(key)
            .cloned()
    }

    fn contains(&self, key: &str) -> bool {
        self.0
            .read()
            .expect("locked map lock poisoned")
            .contains_key(key)
    }

    fn keys(&self) -> Vec<String> {
        self.0
            .read()
            .expect("locked map lock poisoned")
            .keys()
            .cloned()
            .collect()
    }

    fn insert(&self, key: String, value: V) {
        self.0
            .write()
            .expect("locked map lock poisoned")
            .insert(key, value);
    }

    /// A cloned snapshot of every `(key, value)` pair.
    fn entries(&self) -> Vec<(String, V)> {
        self.0
            .read()
            .expect("locked map lock poisoned")
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    /// Replace this map's contents with a snapshot of `other`'s, under both
    /// locks in turn. Used by the hot-reload applier to swap in a pending
    /// map's contents without replacing the `Arc` (and therefore without
    /// invalidating any clone already held by an in-flight request).
    ///
    /// This map's `replace_from` call is independent of every other hot-reload
    /// map's — see `ConfigReloadApplier::apply`'s doc comment (`services/reload/
    /// applier.rs`) for the resulting request-scoped skew window and what to do
    /// if a handler ever needs two of these maps to agree within one request.
    fn replace_from(&self, other: &Self) {
        let snapshot = other.0.read().expect("locked map lock poisoned").clone();
        *self.0.write().expect("locked map lock poisoned") = snapshot;
    }
}

impl<V: Clone> Default for LockedMap<V> {
    fn default() -> Self {
        Self::new(HashMap::new())
    }
}

impl<V: Clone> From<HashMap<String, V>> for LockedMap<V> {
    fn from(map: HashMap<String, V>) -> Self {
        Self::new(map)
    }
}

/// Maps registry name → registry type (e.g. `"github1"` → `"github"`).
#[derive(Clone, Default)]
pub struct RegistryMap(LockedMap<String>);

impl RegistryMap {
    pub fn new(map: HashMap<String, String>) -> Self {
        Self(LockedMap::new(map))
    }

    pub fn type_of(&self, name: &str) -> Option<String> {
        self.0.get(name)
    }

    pub fn is_type(&self, name: &str, expected: &str) -> bool {
        self.type_of(name).as_deref() == Some(expected)
    }

    pub fn contains(&self, name: &str) -> bool {
        self.0.contains(name)
    }

    pub fn keys(&self) -> Vec<String> {
        self.0.keys()
    }

    /// A cloned snapshot of every `(registry name, registry type)` pair.
    pub fn entries(&self) -> Vec<(String, String)> {
        self.0.entries()
    }

    /// Registry names with the given type.
    pub fn names_of_type(&self, registry_type: &str) -> Vec<String> {
        self.entries()
            .into_iter()
            .filter(|(_, t)| t == registry_type)
            .map(|(n, _)| n)
            .collect()
    }

    /// Replace this map's contents with `other`'s (called by the hot-reload applier).
    pub fn replace_from(&self, other: &Self) {
        self.0.replace_from(&other.0);
    }
}

impl From<HashMap<String, String>> for RegistryMap {
    fn from(map: HashMap<String, String>) -> Self {
        Self::new(map)
    }
}

/// Maps registry name → configured `RegistryMode` (proxy / local / hybrid).
#[derive(Clone, Default)]
pub struct RegistryModeMap(LockedMap<RegistryMode>);

impl RegistryModeMap {
    pub fn new(map: HashMap<String, RegistryMode>) -> Self {
        Self(LockedMap::new(map))
    }

    pub fn get(&self, name: &str) -> RegistryMode {
        self.0.get(name).unwrap_or_default()
    }

    pub fn insert(&self, name: String, mode: RegistryMode) {
        self.0.insert(name, mode);
    }

    /// Replace this map's contents with `other`'s (called by the hot-reload applier).
    pub fn replace_from(&self, other: &Self) {
        self.0.replace_from(&other.0);
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
#[derive(Clone, Default)]
pub struct RepoSignerMap(LockedMap<Arc<batlehub_adapters::repo::OpenPgpSigner>>);

impl RepoSignerMap {
    pub fn get(&self, name: &str) -> Option<Arc<batlehub_adapters::repo::OpenPgpSigner>> {
        self.0.get(name)
    }

    /// Replace this map's contents with `other`'s (called by the hot-reload applier).
    pub fn replace_from(&self, other: &Self) {
        self.0.replace_from(&other.0);
    }
}

impl From<HashMap<String, Arc<batlehub_adapters::repo::OpenPgpSigner>>> for RepoSignerMap {
    fn from(map: HashMap<String, Arc<batlehub_adapters::repo::OpenPgpSigner>>) -> Self {
        Self(LockedMap::new(map))
    }
}

/// Maps npm/terraform/pypi/conda registry name → first upstream base URL (for audit pass-through).
#[derive(Clone, Default)]
pub struct UpstreamMap(LockedMap<String>);

impl UpstreamMap {
    pub fn new(map: HashMap<String, String>) -> Self {
        Self(LockedMap::new(map))
    }

    pub fn upstream_for(&self, name: &str) -> Option<String> {
        self.0.get(name)
    }

    /// Replace this map's contents with `other`'s (called by the hot-reload applier).
    pub fn replace_from(&self, other: &Self) {
        self.0.replace_from(&other.0);
    }
}

impl From<HashMap<String, String>> for UpstreamMap {
    fn from(map: HashMap<String, String>) -> Self {
        Self::new(map)
    }
}

/// Maps a `goproxy` registry name → the base URL of its upstream Go Vulnerability
/// Database (default `https://vuln.go.dev`). Registries absent from the map have
/// the vuln DB passthrough disabled (`vuln_db_url = ""`).
///
/// Holds a shared `reqwest::Client` so all registries reuse one connection pool.
#[derive(Clone)]
pub struct VulnDbMap {
    pub http: reqwest::Client,
    urls: LockedMap<String>,
}

impl VulnDbMap {
    pub fn new(urls: HashMap<String, String>) -> Self {
        let http = reqwest::Client::builder()
            .user_agent("batlehub/0.1")
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()
            .expect("building vuln DB HTTP client");
        Self {
            http,
            urls: LockedMap::new(urls),
        }
    }

    pub fn url_for(&self, registry: &str) -> Option<String> {
        self.urls.get(registry)
    }

    /// Replace the URL map in place (called by the hot-reload applier).
    pub fn update(&self, urls: HashMap<String, String>) {
        *self.urls.0.write().expect("locked map lock poisoned") = urls;
    }

    /// Replace this map's contents with `other`'s (called by the hot-reload applier).
    pub fn replace_from(&self, other: &Self) {
        self.urls.replace_from(&other.urls);
    }
}

impl Default for VulnDbMap {
    fn default() -> Self {
        Self::new(HashMap::new())
    }
}

/// Maps Cargo registry name → [`CargoIndexProxy`] (sparse-index HTTP client + URL).
#[derive(Clone, Default)]
pub struct CargoIndexMap(LockedMap<CargoIndexProxy>);

impl CargoIndexMap {
    pub fn new(map: HashMap<String, CargoIndexProxy>) -> Self {
        Self(LockedMap::new(map))
    }

    /// Clone the proxy for the given registry name, if configured.
    pub fn get(&self, name: &str) -> Option<CargoIndexProxy> {
        self.0.get(name)
    }

    /// Replace this map's contents with `other`'s (called by the hot-reload applier).
    pub fn replace_from(&self, other: &Self) {
        self.0.replace_from(&other.0);
    }
}

impl From<HashMap<String, CargoIndexProxy>> for CargoIndexMap {
    fn from(map: HashMap<String, CargoIndexProxy>) -> Self {
        Self::new(map)
    }
}

use actix_web::web;
use handlers::back_office::ops::eviction::EvictionServiceMap;
use handlers::back_office::ops::warming::WarmingServiceMap;
use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa::OpenApi;
use utoipa_actix_web::{service_config::ServiceConfig as UtoipaServiceConfig, AppExt};
use utoipa_scalar::{Scalar, Servable as _};

use sqlx::PgPool;

use batlehub_adapters::auth::OidcSsoFlow;
use batlehub_core::{
    ports::{StorageAdminRepository, UserTokenRepository},
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
pub use middleware::UserBlockMiddlewareFactory;

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
            access_check::admin_access_check,
            audit::{audit_log, export_audit_log, purge_audit_log},
            bulk::{
                bulk_delete, bulk_unyank, bulk_yank as bulk_yank_handler, deprecate, relist,
                undeprecate, unlist,
            },
            config::{
                apply_pending_reload, clear_banner, discard_pending_reload, get_config_content,
                get_pending_reload, list_config_changes, load_config_from_content, reload_config,
                set_banner, validate_config_content,
            },
            explore::invalidate_explore_cache,
            governance::{
                beta_channel::{add_beta_member, list_beta_members, remove_beta_member},
                ownership::{add_package_owner, list_package_owners, remove_package_owner},
                team_namespaces::{
                    claim_namespace, list_namespaces, my_namespace_packages, my_namespaces,
                    release_namespace,
                },
                user_block::{block_user, list_blocked_users, unblock_user},
            },
            health::{clear_registry_cache, registry_health},
            notification::{
                create_subscription, delete_subscription, get_subscription,
                list_notification_channels, list_subscriptions, test_subscription,
                update_subscription,
            },
            ops::{
                eviction::{delete_cached_artifact, evict_registry},
                ip_blocks::{block_ip, list_blocked_ips, unblock_ip},
                quota::{
                    get_quota_for_user, list_quota, list_quota_for_registry, reset_quota_for_user,
                },
                warming::{get_warming_status, warm_registry},
            },
            packages::{
                block_package, bulk_block_packages, bulk_delete_packages, bulk_unblock_packages,
                delete_package, invalidate_package, list_packages as admin_list_packages,
                package_detail, unblock_package,
            },
            sbom::{export_org_sbom, get_artifact_sbom},
            stats::admin_stats,
            visibility::{get_package_visibility, set_package_visibility},
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
                composer_dist, composer_p2_metadata, composer_packages_json,
                composer_security_advisories, composer_upload, composer_yank,
            },
            // Register most-specific patterns first so actix-web resolves correctly:
            // cargo api/v1 (literal "api" segment) > cargo index (literal "registry" segment) >
            // github (owner/repo/verb) > cargo download (literal "download") >
            // maven (literal "maven2" segment) >
            // openvsx vsix (literal "vsix") > npm audit bulk/quick > npm tarball >
            // shared version metadata > shared packument
            // nuget: vuln page/index > registration > flat > search
            // composer: upload/yank > advisories (literal "api") > p2 > dist > packages.json
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
            goproxy::{
                goproxy_file, goproxy_latest, goproxy_list, goproxy_publish, goproxy_vuln_entry,
                goproxy_vuln_index, goproxy_vuln_query,
            },
            jetbrains::jetbrains_get,
            maven::{maven_get, maven_put},
            npm::{
                audit_bulk, audit_quick, download_tarball as npm_download_tarball, get_packument,
                get_version, npm_publish,
            },
            nuget::{
                nuget_flat_download, nuget_flat_versions, nuget_publish, nuget_registration,
                nuget_search, nuget_service_index, nuget_vuln_index, nuget_vuln_page, nuget_yank,
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
                terraform_module_artifact, terraform_module_download, terraform_module_unyank,
                terraform_module_upload, terraform_module_versions, terraform_module_yank,
                terraform_provider_artifact, terraform_provider_binary_upload,
                terraform_provider_download, terraform_provider_unyank, terraform_provider_upload,
                terraform_provider_versions, terraform_provider_yank,
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
    // Vuln DB passthrough: literal /v1/ paths registered before the module wildcard routes.
    cfg.service(goproxy_vuln_index); // GET …/v1/index.json
    cfg.service(goproxy_vuln_entry); // GET …/v1/ID/{id}.json
    cfg.service(goproxy_vuln_query); // POST …/v1/query
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
    cfg.service(nuget_vuln_page); // GET .../v3/vulnerabilities/page/{page}
    cfg.service(nuget_vuln_index); // GET .../v3/vulnerabilities/index.json
    cfg.service(nuget_registration); // GET .../v3/registration5/{id}/index.json
    cfg.service(nuget_flat_versions); // GET .../v3/flat/{id}/index.json
    cfg.service(nuget_search); // GET .../v3/query
    cfg.service(nuget_flat_download); // GET .../v3/flat/{id}/{version}/{filename}
                                      // Terraform modules — longer paths first (unyank > yank > artifact > upload > download > versions)
    cfg.service(terraform_module_unyank); // POST …/versions/{ver}/unyank
    cfg.service(terraform_module_yank); // DELETE …/versions/{ver}
    cfg.service(terraform_module_artifact); // GET …/{ver}/artifact
    cfg.service(terraform_module_upload); // POST …/{ver}
    cfg.service(terraform_module_download); // GET …/{ver}/download
    cfg.service(terraform_module_versions); // GET …/versions
                                            // Terraform providers — binary PUT/GET before download, unyank/yank before upload/versions
    cfg.service(terraform_provider_unyank); // POST …/versions/{ver}/unyank
    cfg.service(terraform_provider_yank); // DELETE …/versions/{ver}
    cfg.service(terraform_provider_binary_upload); // PUT …/{ver}/artifact/{os}/{arch}
    cfg.service(terraform_provider_artifact); // GET …/{ver}/artifact/{os}/{arch}
    cfg.service(terraform_provider_download); // GET …/{ver}/download/{os}/{arch}
    cfg.service(terraform_provider_upload); // POST …/versions (write)
    cfg.service(terraform_provider_versions); // GET …/versions
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
    cfg.service(composer_security_advisories); // GET …/api/security-advisories/
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
    // npm audit pass-through (literal "/-/npm/v1/audit/{quick,bulk}" paths — bulk before quick)
    cfg.service(audit_bulk);
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
    cfg.service(delete_package);
    cfg.service(bulk_delete_packages);
    cfg.service(bulk_block_packages);
    cfg.service(bulk_unblock_packages);
    cfg.service(invalidate_package);
    cfg.service(registry_health);
    cfg.service(clear_registry_cache);
    cfg.service(export_audit_log); // specific path before parameterised handlers
    cfg.service(audit_log);
    cfg.service(purge_audit_log);
    cfg.service(get_warming_status);
    cfg.service(warm_registry);
    cfg.service(evict_registry);
    cfg.service(delete_cached_artifact);
    cfg.service(admin_stats);
    cfg.service(admin_access_check);
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
    // User block admin (specific /blocked list before parameterised /{user_id}/block)
    cfg.service(list_blocked_users);
    cfg.service(block_user);
    cfg.service(unblock_user);
    // Config reload admin (pending/apply before pending/delete — more specific first)
    cfg.service(reload_config);
    cfg.service(apply_pending_reload);
    cfg.service(get_pending_reload);
    cfg.service(discard_pending_reload);
    cfg.service(list_config_changes);
    // Config content (editor) endpoints
    cfg.service(get_config_content);
    cfg.service(validate_config_content);
    cfg.service(load_config_from_content);
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
    storage_admin_repo: Option<Arc<dyn StorageAdminRepository>>,
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
        if let Some(ref r) = storage_admin_repo {
            cfg.app_data(web::Data::new(r.clone()));
        }
        collect_routes(cfg);
    }
}
