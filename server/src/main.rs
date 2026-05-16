use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use actix_web::{App, HttpServer};
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{Resource, trace as sdktrace};
use tracing_actix_web::TracingLogger;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

use proxy_cache_adapters::{
    auth::{KubernetesAuthProvider, OidcAuthProvider, StaticTokenAuthProvider},
    db::PgPackageRepository,
    registry::{CargoRegistryClient, FanoutRegistryClient, GithubRegistryClient, NpmRegistryClient},
    storage::FilesystemStorageBackend,
};
use proxy_cache_config::{
    load,
    schema::{AuthConfig, OtelConfig, RegistryConfig, RuleConfig, StorageConfig},
};
use proxy_cache_core::{
    entities::Role,
    ports::{AuthProvider, CacheStore, InMemoryCacheStore},
    rules::{BlockListRule, RbacRule, ReleaseAgeGateRule},
    services::{AdminService, ProxyService, RegistryPolicy},
};
use proxy_cache_web::{configure_app, openapi_spec};

// ── CLI ───────────────────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(name = "proxy-cache", about = "Smart proxy cache for package registries")]
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
    let config = load(&cli.config)
        .with_context(|| format!("loading config from '{}'", cli.config))?;

    // Handle subcommands before initialising anything heavy.
    if let Some(Command::DumpSpec) = cli.command {
        let spec = openapi_spec();
        println!("{}", spec.to_pretty_json().expect("serialize openapi spec"));
        return Ok(());
    }

    // ── Tracing ───────────────────────────────────────────────────────────────
    let _tracer_provider = init_tracing(config.otel.as_ref());

    tracing::info!(config = %cli.config, "proxy-cache starting");

    // ── Database ──────────────────────────────────────────────────────────────
    let repo = Arc::new(
        PgPackageRepository::new(&config.database.url)
            .await
            .context("connecting to database")?,
    );
    repo.run_migrations().await.context("running migrations")?;

    // ── Storage ───────────────────────────────────────────────────────────────
    let storage: Arc<dyn proxy_cache_core::ports::StorageBackend> = match &config.storage {
        StorageConfig::Filesystem(fs) => Arc::new(
            FilesystemStorageBackend::new(&fs.path)
                .await
                .with_context(|| format!("initialising filesystem storage at '{}'", fs.path))?,
        ),
        StorageConfig::S3(_s3) => {
            anyhow::bail!("S3 storage adapter not yet compiled; enable the 'storage-s3' feature");
        }
    };

    // ── Auth providers ────────────────────────────────────────────────────────
    let mut auth_providers: Vec<Arc<dyn AuthProvider>> = Vec::new();
    for auth_cfg in &config.auth {
        match auth_cfg {
            AuthConfig::Token(tok) => {
                let entries = tok.tokens.iter().map(|t| {
                    let role = parse_role(&t.role);
                    (t.value.clone(), t.user_id.clone(), role)
                });
                auth_providers.push(Arc::new(StaticTokenAuthProvider::new(entries)));
            }
            AuthConfig::Oidc(oidc_cfg) => {
                let provider = OidcAuthProvider::new(oidc_cfg)
                    .await
                    .context("initialising OIDC auth provider")?;
                auth_providers.push(Arc::new(provider));
            }
            AuthConfig::Kubernetes(k8s_cfg) => {
                let provider = KubernetesAuthProvider::new(k8s_cfg)
                    .await
                    .context("initialising Kubernetes auth provider")?;
                auth_providers.push(Arc::new(provider));
            }
        }
    }

    // ── Registries + policies ─────────────────────────────────────────────────
    let cache: Arc<dyn CacheStore> = Arc::new(InMemoryCacheStore::new());
    let mut registry_clients: HashMap<String, Arc<dyn proxy_cache_core::ports::RegistryClient>> =
        HashMap::new();
    let mut policies: HashMap<String, RegistryPolicy> = HashMap::new();

    for reg in &config.registries {
        let client = build_registry_client(reg);
        registry_clients.insert(reg.name.clone(), client);

        let policy = build_policy(
            reg,
            repo.clone() as Arc<dyn proxy_cache_core::ports::PackageRepository>,
        );
        policies.insert(reg.name.clone(), policy);
    }

    // ── Services ──────────────────────────────────────────────────────────────
    let proxy_svc = Arc::new(ProxyService {
        registries: registry_clients,
        storage: storage.clone(),
        cache: cache.clone(),
        repo: repo.clone() as Arc<dyn proxy_cache_core::ports::PackageRepository>,
        policies,
    });

    let admin_svc = Arc::new(AdminService::new(
        repo.clone() as Arc<dyn proxy_cache_core::ports::PackageRepository>,
    ));

    // ── HTTP server ───────────────────────────────────────────────────────────
    let bind_addr = format!("{}:{}", config.server.host, config.server.port);
    let static_dir = config.server.static_dir.clone();

    tracing::info!(addr = %bind_addr, "listening");

    HttpServer::new(move || {
        App::new()
            .wrap(TracingLogger::default())
            .wrap(proxy_cache_web::AuthMiddlewareFactory::new(
                auth_providers.clone(),
            ))
            .service(proxy_cache_web::swagger_ui())
            .configure(configure_app(
                proxy_svc.clone(),
                admin_svc.clone(),
                static_dir.clone(),
            ))
    })
    .bind(&bind_addr)
    .with_context(|| format!("binding to {bind_addr}"))?
    .run()
    .await
    .context("HTTP server error")?;

    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn parse_role(s: &str) -> Role {
    match s {
        "admin" => Role::Admin,
        "user" => Role::User,
        _ => Role::Anonymous,
    }
}

fn build_registry_client(reg: &RegistryConfig) -> Arc<dyn proxy_cache_core::ports::RegistryClient> {
    fn resolve_urls(configured: &[String], default: &str) -> Vec<String> {
        if configured.is_empty() { vec![default.to_owned()] } else { configured.to_vec() }
    }

    fn make_one(registry_type: &str, url: &str) -> Arc<dyn proxy_cache_core::ports::RegistryClient> {
        match registry_type {
            "github" => Arc::new(GithubRegistryClient::new(url, None)),
            "npm"    => Arc::new(NpmRegistryClient::new(url)),
            "cargo"  => Arc::new(CargoRegistryClient::new(url)),
            other    => panic!("registry type '{other}' is configured but no adapter is compiled in"),
        }
    }

    let urls = match reg.registry_type.as_str() {
        "github" => resolve_urls(&reg.upstreams, "https://api.github.com"),
        "npm"    => resolve_urls(&reg.upstreams, "https://registry.npmjs.org"),
        "cargo"  => resolve_urls(&reg.upstreams, "https://crates.io"),
        other    => panic!("registry type '{other}' is configured but no adapter is compiled in"),
    };

    if urls.len() == 1 {
        make_one(&reg.registry_type, &urls[0])
    } else {
        let clients = urls.iter().map(|u| make_one(&reg.registry_type, u)).collect();
        Arc::new(FanoutRegistryClient::new(&reg.registry_type, clients))
    }
}

fn build_policy(
    reg: &RegistryConfig,
    repo: Arc<dyn proxy_cache_core::ports::PackageRepository>,
) -> RegistryPolicy {
    let mut rules: Vec<Box<dyn proxy_cache_core::rules::Rule>> = Vec::new();

    // 1. RBAC rule (always first)
    let rbac_perms = HashMap::from([
        (Role::Anonymous, reg.rbac.anonymous.clone()),
        (Role::User, reg.rbac.user.clone()),
        (Role::Admin, reg.rbac.admin.clone()),
    ]);
    rules.push(Box::new(RbacRule::new(rbac_perms)));

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
        }
    }

    RegistryPolicy {
        metadata_ttl: Some(Duration::from_secs(reg.cache.metadata_ttl_secs)),
        rules,
    }
}

/// Initialise tracing.  Returns the `TracerProvider` when OTLP is configured
/// so the caller can keep it alive for the process lifetime and flush on exit.
fn init_tracing(otel_cfg: Option<&OtelConfig>) -> Option<sdktrace::SdkTracerProvider> {
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"));

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
