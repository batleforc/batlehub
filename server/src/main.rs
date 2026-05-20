use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use actix_cors::Cors;
use actix_web::{http, web, App, HttpServer};
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{trace as sdktrace, Resource};
use tracing::info;
use tracing_actix_web::{DefaultRootSpanBuilder, RootSpanBuilder, TracingLogger};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};
use utoipa::OpenApi as _;
use utoipa_actix_web::AppExt;

use batlehub_adapters::{
    auth::{
        KubernetesAuthProvider, OidcAuthProvider, OidcSsoFlow, StaticTokenAuthProvider,
        UserTokenAuthProvider,
    },
    db::PgPackageRepository,
    registry::{
        CargoRegistryClient, FanoutRegistryClient, GoProxyRegistryClient, GithubRegistryClient,
        NpmRegistryClient, OpenVsxRegistryClient, VsCodeMarketplaceRegistryClient,
        UpstreamHttpOptions,
    },
    storage::{FilesystemStorageBackend, StorageRouter},
};
use batlehub_config::{
    load,
    schema::{AuthConfig, OtelConfig, RegistryConfig, RuleConfig, StorageBackendConfig, StoragesConfig, UpstreamAuthConfig},
};
use batlehub_core::{
    entities::Role,
    ports::{AuthProvider, CacheStore, InMemoryCacheStore, UserTokenRepository},
    rules::{BlockListRule, DenyLatestRule, RbacRule, ReleaseAgeGateRule},
    services::{AdminService, ProxyService, RegistryPolicy},
};
use batlehub_web::{configure_app, openapi_spec, AccessConfig, ApiDoc, CargoIndexProxy, RegistryMap, UpstreamMap};

// ── Tracing span builder ──────────────────────────────────────────────────────

/// Custom root span builder that separates upstream/client errors from backend faults:
/// - 4xx → `INFO`  "upstream/client error (not a backend fault)"
/// - 5xx → `WARN`  "backend error"
struct BatleHubSpanBuilder;

impl RootSpanBuilder for BatleHubSpanBuilder {
    fn on_request_start(request: &actix_web::dev::ServiceRequest) -> tracing::Span {
        tracing_actix_web::root_span!(level = tracing::Level::INFO, request)
    }

    fn on_request_end<B: actix_web::body::MessageBody>(
        span: tracing::Span,
        outcome: &Result<actix_web::dev::ServiceResponse<B>, actix_web::Error>,
    ) {
        let status = match outcome {
            Ok(resp) => resp.status(),
            Err(err) => err.as_response_error().status_code(),
        };
        if status.is_client_error() {
            tracing::info!(
                http.status_code = status.as_u16(),
                "upstream/client error (not a backend fault)"
            );
        } else if status.is_server_error() {
            tracing::warn!(http.status_code = status.as_u16(), "backend error");
        }
        DefaultRootSpanBuilder::on_request_end(span, outcome);
    }
}

// ── CLI ───────────────────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(
    name = "batlehub",
    about = "BatleHub — smart artifact hub for package registries"
)]
struct Cli {
    #[arg(short, long, default_value = "config.toml")]
    config: String,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Print the OpenAPI spec to stdout and exit (for frontend code generation).
    DumpSpec,
}

// ── Entry point ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Handle subcommands before loading config or initialising anything heavy.
    if let Some(Command::DumpSpec) = cli.command {
        let spec = openapi_spec();
        println!("{}", spec.to_pretty_json().expect("serialize openapi spec"));
        return Ok(());
    }

    let config =
        load(&cli.config).with_context(|| format!("loading config from '{}'", cli.config))?;

    // ── Tracing ───────────────────────────────────────────────────────────────
    let _tracer_provider = init_tracing(config.otel.as_ref());

    tracing::info!(config = %cli.config, "batlehub starting");

    // ── Database ──────────────────────────────────────────────────────────────
    let repo = Arc::new(
        PgPackageRepository::new(&config.database.url)
            .await
            .context("connecting to database")?,
    );
    repo.run_migrations().await.context("running migrations")?;

    // ── Storage ───────────────────────────────────────────────────────────────
    let storage: Arc<dyn batlehub_core::ports::StorageBackend> = match &config.storage {
        StoragesConfig::Single(backend_cfg) => {
            // Wrap in StorageRouter so artifact_storage is always tracked in the DB,
            // enabling the health endpoint to report accurate artifact counts and sizes.
            let backend = build_single_backend(backend_cfg).await?;
            let mut backends = HashMap::new();
            backends.insert("default".to_string(), backend);
            Arc::new(StorageRouter::new(
                backends,
                "default".to_string(),
                HashMap::new(),
                repo.pool(),
            ))
        }
        StoragesConfig::Multi(multi) => {
            let mut backends = HashMap::new();
            for named in &multi.backends {
                let backend = build_single_backend(&named.config).await?;
                backends.insert(named.name.clone(), backend);
            }
            if !backends.contains_key(&multi.default) {
                anyhow::bail!(
                    "storage default '{}' does not match any backend name in [[storage.backends]]",
                    multi.default
                );
            }
            let registry_assignments: HashMap<String, String> = config
                .registries
                .iter()
                .filter_map(|r| r.storage.as_ref().map(|s| (r.name.clone(), s.clone())))
                .collect();
            Arc::new(StorageRouter::new(
                backends,
                multi.default.clone(),
                registry_assignments,
                repo.pool(),
            ))
        }
    };

    // ── Auth providers ────────────────────────────────────────────────────────
    let mut auth_providers: Vec<Arc<dyn AuthProvider>> = Vec::new();
    let mut oidc_sso_flows: Vec<OidcSsoFlow> = Vec::new();

    for auth_cfg in &config.auth {
        match auth_cfg {
            AuthConfig::Token(tok) => {
                let entries = tok.tokens.iter().map(|t| {
                    let role = parse_role(&t.role);
                    (t.value.clone(), t.user_id.clone(), role)
                });
                auth_providers.push(Arc::new(StaticTokenAuthProvider::new(entries)));
                info!("configured static token auth provider");
            }
            AuthConfig::Oidc(oidc_cfg) => {
                match OidcAuthProvider::new(oidc_cfg).await {
                    Ok(provider) => {
                        if let Some(flow) = provider.sso_flow().cloned() {
                            oidc_sso_flows.push(flow);
                        }
                        auth_providers.push(Arc::new(provider));
                        tracing::info!(issuer = %oidc_cfg.issuer_url, "OIDC auth provider ready");
                    }
                    Err(e) => {
                        // Non-fatal: server starts without OIDC. The /auth/oidc/login
                        // endpoint will return 503 until the provider becomes reachable
                        // and the server is restarted.
                        tracing::warn!(
                            issuer = %oidc_cfg.issuer_url,
                            error = %e,
                            "OIDC provider unreachable at startup — continuing without it"
                        );
                    }
                }
            }
            AuthConfig::Kubernetes(k8s_cfg) => {
                let provider = KubernetesAuthProvider::new(k8s_cfg)
                    .await
                    .context("initialising Kubernetes auth provider")?;
                auth_providers.push(Arc::new(provider));
                info!(
                    "configured Kubernetes auth provider for service account '{}'",
                    k8s_cfg.audiences.join(", ")
                );
            }
        }
    }

    // Add user-token provider (after OIDC so JWTs are validated first)
    let token_repo = repo.clone() as Arc<dyn UserTokenRepository>;
    auth_providers.push(Arc::new(UserTokenAuthProvider::new(token_repo.clone())));
    info!("configured user-token auth provider");

    // ── Registries + policies ─────────────────────────────────────────────────
    let cache: Arc<dyn CacheStore> = Arc::new(InMemoryCacheStore::new());
    let mut registry_clients: HashMap<String, Arc<dyn batlehub_core::ports::RegistryClient>> =
        HashMap::new();
    let mut policies: HashMap<String, RegistryPolicy> = HashMap::new();
    let mut cargo_indexes: HashMap<String, CargoIndexProxy> = HashMap::new();
    let mut registry_type_map: HashMap<String, String> = HashMap::new();
    let mut npm_upstream_map: HashMap<String, String> = HashMap::new();

    for reg in &config.registries {
        let client = build_registry_client(reg)
            .with_context(|| format!("building registry client for '{}'", reg.name))?;
        registry_clients.insert(reg.name.clone(), client);

        let policy = build_policy(
            reg,
            repo.clone() as Arc<dyn batlehub_core::ports::PackageRepository>,
        );
        policies.insert(reg.name.clone(), policy);

        registry_type_map.insert(reg.name.clone(), reg.registry_type.clone());

        if reg.registry_type == "npm" {
            let first_url = if reg.upstreams.is_empty() {
                "https://registry.npmjs.org".to_owned()
            } else {
                reg.upstreams[0].clone()
            };
            npm_upstream_map.insert(reg.name.clone(), first_url);
        }

        if reg.registry_type == "cargo" {
            let index = build_cargo_index(reg)
                .with_context(|| format!("building cargo index client for '{}'", reg.name))?;
            cargo_indexes.insert(reg.name.clone(), index);
        }
    }

    let upstream_map = UpstreamMap(npm_upstream_map);

    // ── Services ──────────────────────────────────────────────────────────────
    let proxy_svc = Arc::new(ProxyService {
        registries: registry_clients,
        storage: storage.clone(),
        cache: cache.clone(),
        repo: repo.clone() as Arc<dyn batlehub_core::ports::PackageRepository>,
        policies,
        max_artifact_size_bytes: None,
    });

    let admin_svc = Arc::new(AdminService::new(
        repo.clone() as Arc<dyn batlehub_core::ports::PackageRepository>
    ));

    // ── Access config ─────────────────────────────────────────────────────────
    // Respects role inheritance: user inherits anonymous, admin inherits both.
    // Dynamic groups are additive on top of role-based access.
    let mut group_access: HashMap<String, HashSet<String>> = HashMap::new();
    for r in &config.registries {
        for group_name in r.rbac.groups.keys() {
            group_access.entry(group_name.clone()).or_default().insert(r.name.clone());
        }
    }

    let access_config = AccessConfig {
        anonymous: config.registries.iter()
            .filter(|r| !r.rbac.anonymous.is_empty())
            .map(|r| r.name.clone())
            .collect(),
        user: config.registries.iter()
            .filter(|r| !r.rbac.anonymous.is_empty() || !r.rbac.user.is_empty())
            .map(|r| r.name.clone())
            .collect(),
        admin: config.registries.iter()
            .filter(|r| !r.rbac.anonymous.is_empty() || !r.rbac.user.is_empty() || !r.rbac.admin.is_empty())
            .map(|r| r.name.clone())
            .collect(),
        groups: group_access,
    };

    let registry_map = RegistryMap(registry_type_map);

    // ── HTTP server ───────────────────────────────────────────────────────────
    let bind_addr = format!("{}:{}", config.server.host, config.server.port);
    let static_dir = config.server.static_dir.clone();
    let cors_allowed_origins = config.server.cors_allowed_origins.clone().unwrap_or_default();
    let db_pool = repo.pool();

    tracing::info!(addr = %bind_addr, "listening");

    HttpServer::new(move || {
        let configure = configure_app(
            proxy_svc.clone(),
            admin_svc.clone(),
            token_repo.clone(),
            Some(db_pool.clone()),
            access_config.clone(),
            registry_map.clone(),
            upstream_map.clone(),
            oidc_sso_flows.clone(),
        );
        let static_dir_inner = static_dir.clone();
        let cargo_indexes_inner = cargo_indexes.clone();

        let (app, openapi) = App::new()
            .into_utoipa_app()
            .openapi(ApiDoc::openapi())
            .configure(configure)
            .split_for_parts();

        // Register all cargo sparse index proxies (keyed by registry name).
        let app = app.app_data(web::Data::new(cargo_indexes_inner));

        let cors_base = Cors::default()
            .allowed_methods(vec!["GET", "POST", "HEAD", "OPTIONS", "DELETE"])
            .allowed_headers(vec![
                http::header::AUTHORIZATION,
                http::header::CONTENT_TYPE,
                http::header::ACCEPT,
            ])
            .max_age(3600);
        let cors = if cors_allowed_origins.is_empty() {
            cors_base.allow_any_origin()
        } else {
            cors_allowed_origins.iter().fold(cors_base, |c, origin| c.allowed_origin(origin))
        };

        app.wrap(TracingLogger::<BatleHubSpanBuilder>::new())
            .wrap(batlehub_web::AuthMiddlewareFactory::new(
                auth_providers.clone(),
            ))
            .wrap(cors)
            .service(batlehub_web::swagger_ui(openapi))
            .configure(move |cfg| {
                if let Some(ref dir) = static_dir_inner {
                    cfg.service(
                        actix_files::Files::new("/", dir)
                            .index_file("index.html")
                            .use_last_modified(true),
                    );
                }
            })
    })
    .bind(&bind_addr)
    .with_context(|| format!("binding to {bind_addr}"))?
    .run()
    .await
    .context("HTTP server error")?;

    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

async fn build_single_backend(
    cfg: &StorageBackendConfig,
) -> Result<Arc<dyn batlehub_core::ports::StorageBackend>> {
    match cfg {
        StorageBackendConfig::Filesystem(fs) => {
            let backend = FilesystemStorageBackend::new(&fs.path)
                .await
                .with_context(|| format!("initialising filesystem storage at '{}'", fs.path))?;
            Ok(Arc::new(backend))
        }
        StorageBackendConfig::S3(_s3) => {
            #[cfg(feature = "storage-s3")]
            {
                use batlehub_adapters::storage::S3StorageBackend;
                let backend = S3StorageBackend::new(_s3)
                    .await
                    .with_context(|| format!("initialising S3 storage for bucket '{}'", _s3.bucket))?;
                return Ok(Arc::new(backend));
            }
            #[cfg(not(feature = "storage-s3"))]
            anyhow::bail!("S3 storage requires the 'storage-s3' feature flag at compile time");
        }
    }
}

fn parse_role(s: &str) -> Role {
    match s {
        "admin" => Role::Admin,
        "user" => Role::User,
        _ => Role::Anonymous,
    }
}

fn upstream_options(reg: &RegistryConfig) -> UpstreamHttpOptions {
    let (bearer_token, basic_auth, custom_header) = match &reg.upstream_auth {
        Some(UpstreamAuthConfig::Bearer(b)) => (Some(b.token.clone()), None, None),
        Some(UpstreamAuthConfig::Basic(b)) => (None, Some((b.username.clone(), b.password.clone())), None),
        Some(UpstreamAuthConfig::Header(h)) => (None, None, Some((h.name.clone(), h.value.clone()))),
        None => (None, None, None),
    };
    UpstreamHttpOptions {
        bearer_token,
        basic_auth,
        custom_header,
        ca_cert_path: reg.tls.as_ref().and_then(|t| t.ca_cert_path.clone()),
    }
}

fn build_cargo_index(reg: &RegistryConfig) -> anyhow::Result<CargoIndexProxy> {
    let index_url = if let Some(ref url) = reg.index_url {
        url.clone()
    } else {
        let upstream = reg.upstreams.first().map(|s| s.as_str()).unwrap_or("https://crates.io");
        if upstream.contains("crates.io") {
            "https://index.crates.io".to_owned()
        } else {
            upstream.to_owned()
        }
    };
    let opts = upstream_options(reg);
    let http = batlehub_adapters::registry::apply_upstream_options(
        reqwest::Client::builder().user_agent("batlehub/0.1"),
        &opts,
    )?;
    tracing::info!(index_url = %index_url, "cargo sparse index proxy configured");
    Ok(CargoIndexProxy { http, index_url })
}

fn build_registry_client(reg: &RegistryConfig) -> anyhow::Result<Arc<dyn batlehub_core::ports::RegistryClient>> {
    fn resolve_urls(configured: &[String], default: &str) -> Vec<String> {
        if configured.is_empty() {
            vec![default.to_owned()]
        } else {
            configured.to_vec()
        }
    }

    fn make_one(
        registry_type: &str,
        url: &str,
        opts: &UpstreamHttpOptions,
    ) -> anyhow::Result<Arc<dyn batlehub_core::ports::RegistryClient>> {
        let client: Arc<dyn batlehub_core::ports::RegistryClient> = match registry_type {
            "github" => Arc::new(GithubRegistryClient::new(url, opts)?),
            "npm" => Arc::new(NpmRegistryClient::new(url, opts)?),
            "cargo" => Arc::new(CargoRegistryClient::new(url, opts)?),
            "openvsx" => Arc::new(OpenVsxRegistryClient::new(url, opts)?),
            "goproxy" => Arc::new(GoProxyRegistryClient::new(url, opts)?),
            "vscode-marketplace" => Arc::new(VsCodeMarketplaceRegistryClient::new(url, opts)?),
            other => anyhow::bail!("registry type '{other}' is configured but no adapter is compiled in"),
        };
        Ok(client)
    }

    let opts = upstream_options(reg);

    let urls = match reg.registry_type.as_str() {
        "github" => resolve_urls(&reg.upstreams, "https://api.github.com"),
        "npm" => resolve_urls(&reg.upstreams, "https://registry.npmjs.org"),
        "cargo" => resolve_urls(&reg.upstreams, "https://crates.io"),
        "openvsx" => resolve_urls(&reg.upstreams, "https://open-vsx.org"),
        "goproxy" => resolve_urls(&reg.upstreams, "https://proxy.golang.org"),
        "vscode-marketplace" => resolve_urls(&reg.upstreams, "https://marketplace.visualstudio.com"),
        other => anyhow::bail!("registry type '{other}' is configured but no adapter is compiled in"),
    };

    if urls.len() == 1 {
        make_one(&reg.registry_type, &urls[0], &opts)
    } else {
        let clients = urls
            .iter()
            .map(|u| make_one(&reg.registry_type, u, &opts))
            .collect::<anyhow::Result<Vec<_>>>()?;
        Ok(Arc::new(FanoutRegistryClient::new(&reg.registry_type, clients)))
    }
}

fn build_policy(
    reg: &RegistryConfig,
    repo: Arc<dyn batlehub_core::ports::PackageRepository>,
) -> RegistryPolicy {
    let mut rules: Vec<Box<dyn batlehub_core::rules::Rule>> = Vec::new();

    // 1. RBAC rule (always first)
    let rbac_perms = HashMap::from([
        (Role::Anonymous, reg.rbac.anonymous.clone()),
        (Role::User, reg.rbac.user.clone()),
        (Role::Admin, reg.rbac.admin.clone()),
    ]);
    rules.push(Box::new(
        RbacRule::new(rbac_perms).with_groups(reg.rbac.groups.clone()),
    ));

    // 2. Block list rule (always second)
    rules.push(Box::new(BlockListRule::new(repo)));

    // 3. Optional registry-specific rules from config
    for rule_cfg in &reg.rules {
        match rule_cfg {
            RuleConfig::ReleaseAgeGate(cfg) => {
                let bypass: Vec<Role> = cfg.bypass_roles.iter().map(|r| parse_role(r)).collect();
                rules.push(Box::new(ReleaseAgeGateRule::new(
                    Duration::from_secs(cfg.min_age_secs),
                    bypass,
                )));
            }
            RuleConfig::RequireSignedRelease(cfg) => {
                if cfg.enabled {
                    tracing::warn!(
                        "require_signed_release rule is configured but not yet implemented"
                    );
                }
            }
            RuleConfig::DenyLatest(cfg) => {
                let bypass: Vec<Role> = cfg.bypass_roles.iter().map(|r| parse_role(r)).collect();
                rules.push(Box::new(DenyLatestRule::new(bypass)));
            }
        }
    }

    RegistryPolicy {
        metadata_ttl: Some(Duration::from_secs(reg.cache.metadata_ttl_secs)),
        firewall_only: reg.firewall_only,
        rules,
    }
}

/// Initialise tracing.  Returns the `TracerProvider` when OTLP is configured
/// so the caller can keep it alive for the process lifetime and flush on exit.
fn init_tracing(otel_cfg: Option<&OtelConfig>) -> Option<sdktrace::SdkTracerProvider> {
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    let (otel_layer, provider) = match otel_cfg {
        Some(cfg) => {
            match build_otlp_provider(cfg) {
                Ok(p) => {
                    use opentelemetry::trace::TracerProvider as _;
                    let tracer = p.tracer(cfg.service_name.clone());
                    let layer = tracing_opentelemetry::layer().with_tracer(tracer);
                    (Some(layer), Some(p))
                }
                Err(e) => {
                    // Don't crash on misconfigured OTLP; just warn and continue.
                    eprintln!("WARN: failed to build OTLP exporter: {e}");
                    (None, None)
                }
            }
        }
        None => (None, None),
    };

    tracing_subscriber::registry()
        .with(env_filter)
        .with(tracing_subscriber::fmt::layer())
        .with(otel_layer)
        .init();

    provider
}

fn build_otlp_provider(cfg: &OtelConfig) -> anyhow::Result<sdktrace::SdkTracerProvider> {
    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint(&cfg.endpoint)
        .build()?;

    let resource = Resource::builder_empty()
        .with_service_name(cfg.service_name.clone())
        .build();

    let provider = sdktrace::SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .with_resource(resource)
        .build();

    Ok(provider)
}
