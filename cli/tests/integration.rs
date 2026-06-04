/// Integration tests for `batlehub-cli`.
///
/// Each test starts a genuine in-memory batlehub server on a random port, then
/// invokes the compiled `batlehub-cli` binary via `std::process::Command` and
/// asserts on exit status and JSON output.
///
/// ## Running
///
/// ```
/// cargo test -p batlehub-cli --test integration
/// ```
use std::collections::HashMap;
use std::io::Write as _;
use std::net::TcpStream;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use actix_web::{web, App, HttpServer};
use utoipa_actix_web::AppExt;

use batlehub_adapters::{
    auth::StaticTokenAuthProvider,
    cache::InMemoryCacheStore,
    in_memory::{
        InMemoryPackageRepository, InMemoryStorageBackend, NoopArtifactMetaRepository,
        NullUserTokenRepository,
    },
    local_registry::InMemoryLocalRegistry,
    notification::InMemoryNotificationStore,
};
use batlehub_config::schema::RegistryMode;
use batlehub_core::{
    entities::{AccessEvent, PackageId, Role},
    ports::{AuthProvider, CacheStore, PackageRepository, RegistryClient},
    rules::{BlockListRule, RbacRule},
    services::{
        new_hot_lock, AdminService, HotConfig, LocalRegistryService, ProxyMetrics, ProxyService,
        RegistryPolicy,
    },
};
use batlehub_web::{
    configure_app, new_access_lock, AccessConfig, AuthMiddlewareFactory, RegistryMap,
    RegistryModeMap, UpstreamMap,
};

const AUTH_TOKEN: &str = "test-cli-token";
const REGISTRY: &str = "test-nuget";

// ── In-memory batlehub server ─────────────────────────────────────────────────

struct TestServer {
    port: u16,
    /// Exposed so tests can seed packages into the package repository directly.
    /// This is necessary because in-memory local-registry and admin-service
    /// backends are separate stores (in Postgres they share the same tables).
    repo: Arc<InMemoryPackageRepository>,
    _runtime: tokio::runtime::Runtime,
}

impl TestServer {
    fn start() -> Self {
        let registry_map = RegistryMap::from(
            [(REGISTRY.to_owned(), "nuget".to_owned())]
                .into_iter()
                .collect::<HashMap<String, String>>(),
        );
        let registry_names: Vec<String> = registry_map.keys();

        let repo = InMemoryPackageRepository::new();
        let storage = InMemoryStorageBackend::new();
        let cache: Arc<dyn CacheStore> = Arc::new(InMemoryCacheStore::new());

        let policies: HashMap<String, Arc<RegistryPolicy>> = registry_names
            .iter()
            .map(|name| {
                let perms = HashMap::from([
                    (Role::Anonymous, vec!["*".to_owned()]),
                    (Role::User, vec!["*".to_owned()]),
                    (Role::Admin, vec!["*".to_owned()]),
                ]);
                (
                    name.clone(),
                    Arc::new(RegistryPolicy {
                        metadata_ttl: None,
                        firewall_only: false,
                        serve_stale_metadata: false,
                        artifact_ttl: None,
                        rules: vec![
                            Box::new(RbacRule::new(perms)),
                            Box::new(BlockListRule::new(repo.clone())),
                        ],
                    }),
                )
            })
            .collect();

        let local_svc = Arc::new(LocalRegistryService {
            backend: Arc::new(InMemoryLocalRegistry::new()),
            storage: storage.clone(),
            hot: new_hot_lock(HotConfig {
                registries: HashMap::new(),
                policies: HashMap::new(),
                versioning: HashMap::new(),
                signing: HashMap::new(),
                sbom: HashMap::new(),
                beta_channel: HashMap::new(),
                max_artifact_size_bytes: None,
            }),
            quota: None,
            ownership: None,
            team_namespace: None,
            sbom: None,
            explore_cache: None,
        });

        let registries: HashMap<String, Arc<dyn RegistryClient>> = HashMap::new();
        let proxy_svc = Arc::new(ProxyService {
            hot: new_hot_lock(HotConfig {
                registries,
                policies,
                versioning: HashMap::new(),
                signing: HashMap::new(),
                sbom: HashMap::new(),
                beta_channel: HashMap::new(),
                max_artifact_size_bytes: None,
            }),
            storage,
            cache,
            repo: repo.clone(),
            artifact_meta: NoopArtifactMetaRepository::arc(),
            metrics: Arc::new(ProxyMetrics::new(&[])),
            sbom: None,
        });

        let admin_svc = Arc::new(AdminService::new(repo.clone()));
        let token_repo = NullUserTokenRepository::arc();

        let access_config = new_access_lock(AccessConfig {
            anonymous: registry_names.iter().cloned().collect(),
            user: registry_names.iter().cloned().collect(),
            admin: registry_names.iter().cloned().collect(),
            groups: HashMap::new(),
            explore_anonymous: std::collections::HashSet::new(),
            explore_user: std::collections::HashSet::new(),
            explore_admin: std::collections::HashSet::new(),
        });

        let auth_providers: Vec<Arc<dyn AuthProvider>> =
            vec![Arc::new(StaticTokenAuthProvider::new([(
                AUTH_TOKEN.to_owned(),
                Some("test-user".to_owned()),
                Role::Admin,
            )]))];

        let mode_map = RegistryModeMap::from(
            registry_names
                .iter()
                .map(|n| (n.clone(), RegistryMode::Local))
                .collect::<HashMap<_, _>>(),
        );

        let configure = configure_app(
            proxy_svc,
            admin_svc,
            token_repo,
            None,
            access_config,
            registry_map,
            UpstreamMap::default(),
            vec![],
            HashMap::new(),
            Arc::new(ProxyMetrics::new(&[])),
            None,
            None,
            None,
            Arc::new(InMemoryNotificationStore::new()),
            None,
        );

        let rt = tokio::runtime::Runtime::new().unwrap();
        let port = rt.block_on(async {
            let local_svc = local_svc.clone();
            let mode_map = mode_map.clone();
            let configure = configure.clone();
            let auth_providers = auth_providers.clone();
            let cargo_indexes = batlehub_web::CargoIndexMap::default();

            let server = HttpServer::new(move || {
                let (app, _) = App::new()
                    .into_utoipa_app()
                    .configure(configure.clone())
                    .split_for_parts();
                app.app_data(web::Data::new(cargo_indexes.clone()))
                    .app_data(web::Data::new(local_svc.clone()))
                    .app_data(web::Data::new(mode_map.clone()))
                    .wrap(AuthMiddlewareFactory::new(auth_providers.clone()))
            })
            .bind("127.0.0.1:0")
            .expect("bind to random port");

            let port = server.addrs()[0].port();
            tokio::spawn(server.run());
            port
        });

        assert!(
            wait_for_port(port, Duration::from_secs(10)),
            "test server did not start on port {port}"
        );

        Self {
            port,
            repo,
            _runtime: rt,
        }
    }

    fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// Seed a package entry directly into the `PackageRepository` that the admin
    /// service queries. Required because the in-memory local-registry backend and
    /// the package repository are separate stores (unlike PostgreSQL, where they
    /// share the same tables).
    fn seed_package(&self, name: &str, version: &str) {
        let pkg = PackageId {
            registry: REGISTRY.to_owned(),
            name: name.to_owned(),
            version: version.to_owned(),
            artifact: None,
        };
        let event = AccessEvent::allowed_download(pkg, Some("test-user".to_owned()), Role::Admin);
        let repo = self.repo.clone();
        // Run in a fresh single-threaded runtime so we don't nest runtimes.
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(async move { repo.record_access(event).await.unwrap() });
    }
}

fn wait_for_port(port: u16, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return true;
        }
        thread::sleep(Duration::from_millis(300));
    }
    false
}

// ── Test helpers ──────────────────────────────────────────────────────────────

/// Run the CLI binary with env-injected server/token and return (success, stdout, stderr).
fn cli_cmd(args: &[&str], server: &str, token: &str) -> (bool, String, String) {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_batlehub-cli"))
        .args(args)
        .env("BATLEHUB_SERVER", server)
        .env("BATLEHUB_TOKEN", token)
        .env("HOME", "/tmp")
        .env("XDG_CONFIG_HOME", "/tmp/.xdg-batlehub-test")
        .output()
        .expect("failed to run batlehub-cli binary");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// Build a minimal `.nupkg` ZIP containing only a `.nuspec` file.
fn make_nupkg(id: &str, version: &str) -> Vec<u8> {
    let nuspec = format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://schemas.microsoft.com/packaging/2013/05/nuspec.xsd">
  <metadata>
    <id>{id}</id>
    <version>{version}</version>
    <description>Integration test package</description>
    <authors>TestAuthor</authors>
  </metadata>
</package>"#
    );
    let mut buf = Vec::new();
    {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
        let opts = zip::write::SimpleFileOptions::default();
        zip.start_file(format!("{id}.nuspec"), opts).unwrap();
        zip.write_all(nuspec.as_bytes()).unwrap();
        zip.finish().unwrap();
    }
    buf
}

/// Publish a minimal NuGet package directly via HTTP (not via CLI) to the local
/// registry endpoint. Used for `publish_nuget_via_cli` which tests the CLI publish
/// path itself. Note: this does NOT populate `InMemoryPackageRepository`; use
/// `TestServer::seed_package` if you need the package to appear in `package list`.
fn http_publish_nuget(base_url: &str, registry: &str, name: &str, version: &str) {
    let nupkg = make_nupkg(name, version);
    let boundary = "batlehub_test_boundary";
    let mut body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"package\"; \
         filename=\"package.nupkg\"\r\nContent-Type: application/octet-stream\r\n\r\n"
    )
    .into_bytes();
    body.extend_from_slice(&nupkg);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

    let resp = reqwest::blocking::Client::new()
        .put(format!(
            "{base_url}/proxy/{registry}/nuget/api/v2/package"
        ))
        .header("Authorization", format!("Bearer {AUTH_TOKEN}"))
        .header(
            "Content-Type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(body)
        .send()
        .expect("HTTP publish request failed");

    assert!(
        resp.status().is_success(),
        "HTTP publish returned {}: {}",
        resp.status(),
        resp.text().unwrap_or_default()
    );
}

// ── Tests: registry ───────────────────────────────────────────────────────────

#[test]
fn registry_list_json() {
    let srv = TestServer::start();
    let (ok, stdout, _stderr) =
        cli_cmd(&["registry", "list", "--json"], &srv.base_url(), AUTH_TOKEN);
    assert!(ok, "registry list should succeed");
    let arr: Vec<serde_json::Value> = serde_json::from_str(&stdout).expect("valid JSON array");
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["name"], REGISTRY);
    assert_eq!(arr[0]["type"], "nuget");
    assert_eq!(arr[0]["mode"], "local");
}

#[test]
fn registry_info_found() {
    let srv = TestServer::start();
    let (ok, stdout, _stderr) = cli_cmd(
        &["registry", "info", REGISTRY, "--json"],
        &srv.base_url(),
        AUTH_TOKEN,
    );
    assert!(ok, "registry info for known registry should succeed");
    let val: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert_eq!(val["name"], REGISTRY);
    assert_eq!(val["type"], "nuget");
    assert_eq!(val["mode"], "local");
}

#[test]
fn registry_info_not_found() {
    let srv = TestServer::start();
    let (ok, _stdout, stderr) = cli_cmd(
        &["registry", "info", "no-such-registry", "--json"],
        &srv.base_url(),
        AUTH_TOKEN,
    );
    assert!(!ok, "registry info for unknown registry should fail");
    assert!(
        stderr.to_lowercase().contains("not found"),
        "stderr should mention 'not found', got: {stderr}"
    );
}

// ── Tests: auth ───────────────────────────────────────────────────────────────

#[test]
fn auth_whoami_authenticated() {
    let srv = TestServer::start();
    let (ok, stdout, _stderr) =
        cli_cmd(&["auth", "whoami", "--json"], &srv.base_url(), AUTH_TOKEN);
    assert!(ok, "auth whoami should succeed with valid token");
    let val: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert_eq!(val["user_id"], "test-user");
    assert_eq!(val["role"], "admin");
}

#[test]
fn auth_whoami_anonymous() {
    let srv = TestServer::start();
    // Empty token → anonymous
    let (ok, stdout, _stderr) = cli_cmd(&["auth", "whoami", "--json"], &srv.base_url(), "");
    assert!(ok, "auth whoami without token should succeed (anonymous)");
    let val: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert_eq!(val["role"], "anonymous");
}

// ── Tests: package ────────────────────────────────────────────────────────────

#[test]
fn package_list_empty() {
    let srv = TestServer::start();
    let (ok, stdout, _stderr) = cli_cmd(
        &["package", "list", "--registry", REGISTRY, "--json"],
        &srv.base_url(),
        AUTH_TOKEN,
    );
    assert!(ok, "package list on empty registry should succeed");
    let arr: Vec<serde_json::Value> = serde_json::from_str(&stdout).expect("valid JSON array");
    assert!(arr.is_empty(), "expected empty list, got {arr:?}");
}

#[test]
fn package_list_after_seed() {
    let srv = TestServer::start();
    srv.seed_package("MyLib", "1.0.0");

    let (ok, stdout, _stderr) = cli_cmd(
        &["package", "list", "--registry", REGISTRY, "--json"],
        &srv.base_url(),
        AUTH_TOKEN,
    );
    assert!(ok, "package list should succeed");
    let arr: Vec<serde_json::Value> = serde_json::from_str(&stdout).expect("valid JSON array");
    assert_eq!(arr.len(), 1, "expected one package");
    assert_eq!(
        arr[0]["name"].as_str().unwrap_or("").to_lowercase(),
        "mylib"
    );
    assert_eq!(arr[0]["version"], "1.0.0");
    assert_eq!(arr[0]["registry"], REGISTRY);
}

#[test]
fn package_versions_after_seed() {
    let srv = TestServer::start();
    srv.seed_package("SomeLib", "2.3.4");
    srv.seed_package("SomeLib", "2.3.5");

    let (ok, stdout, _stderr) = cli_cmd(
        &["package", "versions", REGISTRY, "SomeLib", "--json"],
        &srv.base_url(),
        AUTH_TOKEN,
    );
    assert!(ok, "package versions should succeed");
    let arr: Vec<serde_json::Value> = serde_json::from_str(&stdout).expect("valid JSON array");
    assert!(
        arr.iter().any(|v| v["version"] == "2.3.4"),
        "expected version 2.3.4 in {arr:?}"
    );
    assert!(
        arr.iter().any(|v| v["version"] == "2.3.5"),
        "expected version 2.3.5 in {arr:?}"
    );
}

// ── Tests: version lifecycle ──────────────────────────────────────────────────
//
// Yank/unyank/delete operate on InMemoryLocalRegistry (the local-registry backend).
// Verification is done via the NuGet flat-index endpoint which also reads from
// InMemoryLocalRegistry. We don't use `package list` here because that queries
// InMemoryPackageRepository, a separate in-memory store (in Postgres they share
// the same tables).

fn nuget_flat_versions(base_url: &str, registry: &str, id_lower: &str) -> Vec<String> {
    let resp = reqwest::blocking::Client::new()
        .get(format!(
            "{base_url}/proxy/{registry}/nuget/v3/flat/{id_lower}/index.json"
        ))
        .header("Authorization", format!("Bearer {AUTH_TOKEN}"))
        .send()
        .expect("GET flat index");
    if !resp.status().is_success() {
        return vec![];
    }
    let body: serde_json::Value = resp.json().expect("flat index JSON");
    body["versions"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect()
}

#[test]
fn version_yank_and_unyank() {
    let srv = TestServer::start();
    http_publish_nuget(&srv.base_url(), REGISTRY, "YankLib", "0.1.0");

    // Confirm visible before yank
    assert!(
        nuget_flat_versions(&srv.base_url(), REGISTRY, "yanklib").contains(&"0.1.0".to_owned()),
        "version should be in flat index before yank"
    );

    // Yank
    let (ok, _stdout, stderr) = cli_cmd(
        &["version", "yank", REGISTRY, "YankLib", "0.1.0"],
        &srv.base_url(),
        AUTH_TOKEN,
    );
    assert!(ok, "version yank should succeed; stderr: {stderr}");

    // Yanked versions are filtered from the flat index
    assert!(
        !nuget_flat_versions(&srv.base_url(), REGISTRY, "yanklib").contains(&"0.1.0".to_owned()),
        "yanked version should not appear in flat index"
    );

    // Unyank
    let (ok, _stdout, stderr) = cli_cmd(
        &["version", "unyank", REGISTRY, "YankLib", "0.1.0"],
        &srv.base_url(),
        AUTH_TOKEN,
    );
    assert!(ok, "version unyank should succeed; stderr: {stderr}");

    // Unyanked version reappears in flat index
    assert!(
        nuget_flat_versions(&srv.base_url(), REGISTRY, "yanklib").contains(&"0.1.0".to_owned()),
        "unyanked version should reappear in flat index"
    );
}

#[test]
fn version_delete() {
    let srv = TestServer::start();
    http_publish_nuget(&srv.base_url(), REGISTRY, "DeleteLib", "3.0.0");

    // Confirm visible before delete
    assert!(
        nuget_flat_versions(&srv.base_url(), REGISTRY, "deletelib").contains(&"3.0.0".to_owned()),
        "version should be in flat index before delete"
    );

    let (ok, _stdout, stderr) = cli_cmd(
        &[
            "version",
            "delete",
            REGISTRY,
            "DeleteLib",
            "3.0.0",
            "--yes",
        ],
        &srv.base_url(),
        AUTH_TOKEN,
    );
    assert!(ok, "version delete --yes should succeed; stderr: {stderr}");

    // Deleted version no longer in flat index
    assert!(
        !nuget_flat_versions(&srv.base_url(), REGISTRY, "deletelib").contains(&"3.0.0".to_owned()),
        "deleted version should not appear in flat index"
    );
}

// ── Tests: publish ────────────────────────────────────────────────────────────

#[test]
fn publish_nuget_via_cli() {
    let srv = TestServer::start();
    let tmp = tempfile::tempdir().unwrap();
    let nupkg_path = tmp.path().join("TestPkg.1.2.3.nupkg");
    std::fs::write(&nupkg_path, make_nupkg("TestPkg", "1.2.3")).unwrap();

    let (ok, stdout, stderr) = cli_cmd(
        &[
            "publish",
            nupkg_path.to_str().unwrap(),
            "--registry",
            REGISTRY,
        ],
        &srv.base_url(),
        AUTH_TOKEN,
    );
    assert!(ok, "publish should succeed; stderr: {stderr}");
    assert!(
        stdout.contains("Published successfully"),
        "expected success message in stdout, got: {stdout}"
    );
}

/// Verify that the NuGet package uploaded via CLI is accessible via the proxy
/// flat-index endpoint (the local registry's own query path, not the admin list).
#[test]
fn publish_nuget_package_accessible_via_proxy() {
    let srv = TestServer::start();
    http_publish_nuget(&srv.base_url(), REGISTRY, "PubLib", "4.0.0");

    // The NuGet flat index returns the list of available versions.
    let resp = reqwest::blocking::Client::new()
        .get(format!(
            "{}/proxy/{REGISTRY}/nuget/v3/flat/publib/index.json",
            srv.base_url()
        ))
        .header("Authorization", format!("Bearer {AUTH_TOKEN}"))
        .send()
        .expect("GET flat index");
    assert!(
        resp.status().is_success(),
        "flat index returned {}",
        resp.status()
    );
    let body: serde_json::Value = resp.json().expect("valid JSON from flat index");
    let versions = body["versions"].as_array().expect("versions array");
    assert!(
        versions.iter().any(|v| v == "4.0.0"),
        "expected 4.0.0 in flat index versions: {versions:?}"
    );
}
