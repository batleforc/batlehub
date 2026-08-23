use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context, Result};
use tracing::info;

use batlehub_adapters::{
    auth::{
        ActionsOidcAuthProvider, KubernetesAuthProvider, OidcAuthProvider, OidcSsoFlow,
        StaticTokenAuthProvider, UserTokenAuthProvider,
    },
    storage::{FilesystemStorageBackend, StorageRouter},
};
use batlehub_config::schema::{AuthConfig, RegistryMode, StorageBackendConfig, StoragesConfig};
use batlehub_core::entities::RegistryKind;
use batlehub_core::ports::{AuthProvider, StorageBackend, UserTokenRepository};

use crate::builders::parse_role;

// ── Storage ───────────────────────────────────────────────────────────────────

pub(super) async fn initialize_storage(
    config: &batlehub_config::schema::AppConfig,
    pool: sqlx::PgPool,
) -> Result<Arc<dyn StorageBackend>> {
    let storage: Arc<dyn StorageBackend> = match &config.storage {
        StoragesConfig::Single(backend_cfg) => {
            let backend = build_single_backend(backend_cfg).await?;
            let mut backends = HashMap::new();
            backends.insert("default".to_string(), backend);
            Arc::new(StorageRouter::new(
                backends,
                "default".to_string(),
                HashMap::new(),
                pool,
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
                pool,
            ))
        }
    };
    Ok(storage)
}

pub(super) async fn build_single_backend(
    cfg: &StorageBackendConfig,
) -> Result<Arc<dyn StorageBackend>> {
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
                let backend = S3StorageBackend::new(_s3).await.with_context(|| {
                    format!("initialising S3 storage for bucket '{}'", _s3.bucket)
                })?;
                Ok(Arc::new(backend))
            }
            #[cfg(not(feature = "storage-s3"))]
            anyhow::bail!("S3 storage requires the 'storage-s3' feature flag at compile time");
        }
    }
}

// ── Auth providers ─────────────────────────────────────────────────────────────

pub(super) struct AuthSetup {
    pub providers: Vec<Arc<dyn AuthProvider>>,
    pub sso_flows: Vec<OidcSsoFlow>,
    /// Every OIDC provider that was *configured*, whether or not it also has a
    /// browser SSO flow. `POST /api/v1/auth/tokens` checks the caller against
    /// this set, so an OIDC provider without `redirect_uri` still counts as an
    /// OIDC session.
    ///
    /// Deliberately populated from the config rather than from the providers
    /// that came up: a provider whose IdP was unreachable at startup is skipped
    /// (see the `warn` below), and its users would otherwise get an unexplained
    /// 403 on token creation instead of the "authenticate first" they deserve.
    pub oidc_provider_names: batlehub_web::OidcProviderNames,
}

/// What to do when a JWT provider cannot be built at startup.
///
/// The old behaviour was always to warn and carry on, which is a quiet way to
/// come up in a state that looks healthy and is not: with the provider missing,
/// every request that would have carried an identity resolves to anonymous, so
/// the symptom reads as a permissions problem rather than an outage — and the
/// one line saying otherwise scrolls past at boot.
///
/// `required` (true by default for `type = "oidc"`) turns that into a refusal to
/// start. The degraded path stays available for deployments that genuinely
/// prefer it, and marks itself in `batlehub_auth_provider_down` so it is
/// alertable rather than only greppable.
fn provider_unavailable(
    kind: &str,
    name: &str,
    issuer_url: &str,
    required: bool,
    err: anyhow::Error,
) -> Result<()> {
    if required {
        return Err(err).with_context(|| {
            format!(
                "{kind} auth provider '{name}' ({issuer_url}) could not be initialised. \
                 Starting without it would silently downgrade every one of its users to \
                 anonymous. Set `required = false` on this [[auth]] entry to start anyway."
            )
        });
    }
    metrics::gauge!("batlehub_auth_provider_down", "provider" => name.to_owned()).set(1.0);
    tracing::warn!(
        provider = %name,
        issuer = %issuer_url,
        error = %err,
        "{kind} provider unreachable at startup — continuing without it, so its users \
         will authenticate as anonymous until the next restart"
    );
    Ok(())
}

pub(super) async fn initialize_auth_providers(
    config: &batlehub_config::schema::AppConfig,
) -> Result<AuthSetup> {
    let mut auth_providers: Vec<Arc<dyn AuthProvider>> = Vec::new();
    let mut oidc_sso_flows: Vec<OidcSsoFlow> = Vec::new();
    let mut oidc_names: Vec<String> = Vec::new();

    for auth_cfg in &config.auth {
        match auth_cfg {
            AuthConfig::Token(tok) => {
                let entries = tok
                    .tokens
                    .iter()
                    .map(|t| {
                        Ok::<_, anyhow::Error>((
                            t.value.clone(),
                            t.user_id.clone(),
                            parse_role(&t.role)?,
                        ))
                    })
                    .collect::<anyhow::Result<Vec<_>>>()?;
                auth_providers.push(Arc::new(StaticTokenAuthProvider::new(entries)));
                info!("configured static token auth provider");
            }
            AuthConfig::Oidc(oidc_cfg) => {
                oidc_names.push(oidc_cfg.name.clone());
                match OidcAuthProvider::new(oidc_cfg).await {
                    Ok(provider) => {
                        if let Some(flow) = provider.sso_flow().cloned() {
                            oidc_sso_flows.push(flow);
                        }
                        auth_providers.push(Arc::new(provider));
                        tracing::info!(issuer = %oidc_cfg.issuer_url, "OIDC auth provider ready");
                    }
                    Err(e) => provider_unavailable(
                        "OIDC",
                        &oidc_cfg.name,
                        &oidc_cfg.issuer_url,
                        oidc_cfg.required,
                        e,
                    )?,
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
            AuthConfig::ActionsOidc(cfg) => match ActionsOidcAuthProvider::new(cfg).await {
                Ok(provider) => {
                    auth_providers.push(Arc::new(provider));
                    tracing::info!(
                        name = %cfg.name,
                        issuer = %cfg.issuer_url,
                        rules = cfg.rules.len(),
                        "Actions OIDC auth provider ready"
                    );
                }
                Err(e) => provider_unavailable(
                    "Actions OIDC",
                    &cfg.name,
                    &cfg.issuer_url,
                    cfg.required,
                    e,
                )?,
            },
        }
    }
    Ok(AuthSetup {
        providers: auth_providers,
        sso_flows: oidc_sso_flows,
        oidc_provider_names: batlehub_web::OidcProviderNames::new(oidc_names),
    })
}

// ── Cargo index map ───────────────────────────────────────────────────────────

pub(super) fn build_initial_cargo_index_map(
    config: &batlehub_config::schema::AppConfig,
) -> Result<batlehub_web::CargoIndexMap> {
    let mut map: HashMap<String, batlehub_web::CargoIndexProxy> = HashMap::new();
    for reg in &config.registries {
        if reg.registry_type == RegistryKind::Cargo.as_str()
            && !matches!(reg.mode, RegistryMode::Local)
        {
            let index = crate::builders::build_cargo_index(reg, config.proxy.as_ref())
                .with_context(|| format!("building cargo index client for '{}'", reg.name))?;
            map.insert(reg.name.clone(), index);
        }
    }
    Ok(batlehub_web::CargoIndexMap::new(map))
}

// ── Warming map ───────────────────────────────────────────────────────────────

pub(super) fn build_warming_map(
    config: &batlehub_config::schema::AppConfig,
    warming_clients: &HashMap<String, Arc<dyn batlehub_core::ports::RegistryClient>>,
    storage: Arc<dyn StorageBackend>,
    pool: sqlx::PgPool,
    coordinator: Arc<dyn batlehub_core::ports::WarmCoordinator>,
    proxy_metrics: Arc<batlehub_core::services::ProxyMetrics>,
) -> batlehub_web::handlers::back_office::ops::warming::WarmingServiceMap {
    use batlehub_adapters::db::PgArtifactMetaRepository;
    use batlehub_core::services::WarmingService;
    use std::collections::HashMap as HM;

    let mut warming_map: batlehub_web::handlers::back_office::ops::warming::WarmingServiceMap =
        HM::new();
    for reg in &config.registries {
        if let Some(client) = warming_clients.get(&reg.name) {
            let warming_svc = Arc::new(WarmingService {
                client: Arc::clone(client),
                storage: storage.clone(),
                artifact_meta: Arc::new(PgArtifactMetaRepository::new(pool.clone()))
                    as Arc<dyn batlehub_core::ports::ArtifactCacheMeta>,
                registry_name: reg.name.clone(),
                latest_n: reg.cache.warm_latest_n,
                concurrency: reg.cache.warm_concurrency,
                coordinator: Arc::clone(&coordinator),
                metrics: Arc::clone(&proxy_metrics),
            });
            warming_map.insert(reg.name.clone(), warming_svc);
        }
    }
    warming_map
}

// ── Eviction map ──────────────────────────────────────────────────────────────

pub(super) fn build_eviction_map(
    config: &batlehub_config::schema::AppConfig,
    storage: Arc<dyn StorageBackend>,
    pool: sqlx::PgPool,
) -> batlehub_web::handlers::back_office::ops::eviction::EvictionServiceMap {
    use batlehub_adapters::db::PgArtifactMetaRepository;
    use batlehub_core::services::{EvictionConfig, EvictionService};
    use std::collections::HashMap as HM;

    let mut eviction_map: batlehub_web::handlers::back_office::ops::eviction::EvictionServiceMap =
        HM::new();
    for reg in &config.registries {
        let cache = &reg.cache;
        let configured = cache.artifact_ttl_secs.is_some()
            || cache.idle_days.is_some()
            || cache.max_size_bytes.is_some()
            || cache.keep_latest_n.is_some();
        if !configured {
            continue;
        }
        let eviction_svc = Arc::new(EvictionService::new(
            Arc::new(PgArtifactMetaRepository::new(pool.clone()))
                as Arc<dyn batlehub_core::ports::ArtifactMetaRepository>,
            storage.clone(),
            EvictionConfig {
                artifact_ttl_secs: cache.artifact_ttl_secs,
                idle_days: cache.idle_days,
                max_size_bytes: cache.max_size_bytes,
                keep_latest_n: cache.keep_latest_n,
                registry: reg.name.clone(),
            },
        ));
        eviction_map.insert(reg.name.clone(), eviction_svc);
    }
    eviction_map
}

// ── OIDC login states ─────────────────────────────────────────────────────────

/// How often expired in-flight logins are swept.
///
/// Only housekeeping: `LoginStateStore::take` enforces expiry on read, so a
/// prune that never ran cannot let a stale login through — it would only leave
/// dead rows behind.
const LOGIN_STATE_PRUNE_INTERVAL_SECS: u64 = 900;

pub(super) fn spawn_login_state_prune(store: Arc<dyn batlehub_core::ports::LoginStateStore>) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(
            LOGIN_STATE_PRUNE_INTERVAL_SECS,
        ));
        loop {
            ticker.tick().await;
            match store.prune_expired().await {
                Ok(0) => {}
                Ok(n) => tracing::debug!(pruned = n, "oidc login states: pruned expired entries"),
                Err(e) => tracing::warn!(error = %e, "oidc login states: periodic prune failed"),
            }
        }
    });
}

// ── User token provider ───────────────────────────────────────────────────────

pub(super) fn add_user_token_provider(
    auth_providers: &mut Vec<Arc<dyn AuthProvider>>,
    token_repo: Arc<dyn UserTokenRepository>,
) {
    auth_providers.push(Arc::new(UserTokenAuthProvider::new(token_repo)));
    info!("configured user-token auth provider");
}
