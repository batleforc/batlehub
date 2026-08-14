pub mod auth;
pub mod network;
pub mod notifications;
pub mod registry;
pub mod routing;
pub mod rules;
pub mod server;
pub mod storage;
pub mod warnings;

pub use auth::{
    ActionsGroupRule, ActionsOidcAuthConfig, AuthConfig, Condition, ConditionMatchType,
    KubernetesAuthConfig, OidcAuthConfig, RuleMatch, TokenAuthConfig, TokenEntry,
};
pub use network::{
    BasicAuthConfig, BearerAuthConfig, GroupRateLimitConfig, HeaderAuthConfig, IpBlockingConfig,
    RateLimitConfig, RateLimitEnforcement, UpstreamAuthConfig, UpstreamProxyConfig,
    UpstreamTlsConfig,
};
pub use notifications::{
    EmailChannelConfig, InboundWebhookConfig, NotificationChannelConfig, NotificationsConfig,
    SlackChannelConfig, TeamsChannelConfig, WebhookChannelConfig,
};
pub use registry::{
    default_true, BetaChannelConfig, CachePolicy, FeatureFlagsConfig, IntegrityConfig, QuotaConfig,
    QuotaEnforcement, RegistryConfig, RegistryMode, RepoSigningConfig, SbomConfig, SigningConfig,
    VersioningPolicy,
};
pub use routing::{
    is_dns_label, normalise_host, validate_host_entry, wildcard_host, HostSyntaxError,
    RegistryHostBinding, SubdomainRoutingConfig,
};
pub use rules::{
    CveGateConfig, DenyLatestConfig, ExploreRbacConfig, LicenseGateConfig, RbacConfig,
    ReleaseAgeGateConfig, RequireSignedReleaseConfig, RuleConfig, TrustedPublisherConfig,
    VersionGateConfig,
};
pub use server::{
    default_service_name, parse_trusted_proxies, CacheConfig, DatabaseConfig, OtelConfig,
    ServerConfig,
};
pub use warnings::ConfigWarning;

pub use storage::{
    FilesystemStorageConfig, MultiStorageConfig, NamedStorageConfig, S3StorageConfig,
    StorageBackendConfig, StoragesConfig,
};

use anyhow::{bail, Result};
use batlehub_core::ports::LICENSE_EXTRACTION_TYPES;
use serde::Deserialize;

// ── Top-level ─────────────────────────────────────────────────────────────────

/// The current config schema version this binary understands. Bump this only
/// for changes that would silently break an existing config file if applied
/// unchanged (removing/renaming a field, changing a default's meaning) — see
/// "Config versioning" in `docs/guide/configuration.md`.
pub const CURRENT_CONFIG_VERSION: u32 = 1;

#[derive(Debug, Deserialize)]
pub struct AppConfig {
    /// Config schema version. Optional; absent is treated as
    /// [`CURRENT_CONFIG_VERSION`] so every existing config file keeps working
    /// unchanged. An explicit value newer than this binary supports is
    /// rejected at startup rather than silently misbehaving.
    #[serde(default)]
    pub config_version: Option<u32>,
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    #[serde(default)]
    pub auth: Vec<AuthConfig>,
    pub storage: StoragesConfig,
    #[serde(default)]
    pub cache: CacheConfig,
    #[serde(default)]
    pub registries: Vec<RegistryConfig>,
    #[serde(default)]
    pub otel: Option<OtelConfig>,
    #[serde(default)]
    pub limits: LimitsConfig,
    /// Optional global IP-based blocking (fail2ban) configuration.
    #[serde(default)]
    pub ip_blocking: Option<IpBlockingConfig>,
    /// Optional webhook and notification configuration.
    #[serde(default)]
    pub notifications: Option<NotificationsConfig>,
    /// Global HTTP/SOCKS proxy applied to all registry upstreams that do not
    /// define their own `[registries.proxy]` section.
    ///
    /// Can be overridden at runtime via `PROXY_CACHE__PROXY__URL` (and related
    /// variables) without changing the config file.
    #[serde(default)]
    pub proxy: Option<UpstreamProxyConfig>,
    /// Optional periodic re-check of cached SBOMs against the OSV vulnerability
    /// database. When absent or `enabled = false`, no background scan runs.
    #[serde(default)]
    pub vulnerability_scan: Option<VulnerabilityScanConfig>,
    /// Optional wildcard host derivation for host-based registry routing.
    /// Absent or `enabled = false` derives no wildcard hosts; a registry can
    /// still declare explicit `hosts`.
    #[serde(default)]
    pub subdomain_routing: Option<SubdomainRoutingConfig>,
    /// What numbers this instance keeps and publishes. Absent means today's
    /// behaviour: `/metrics` served, history recorded, 30 days retained.
    #[serde(default)]
    pub stats: StatsConfig,
}

// ── Stats ─────────────────────────────────────────────────────────────────────

fn default_history_retention_days() -> u32 {
    30
}

/// Both of the instance's statistical outputs, in one block.
///
/// They live together because an operator deciding "do I want this instance
/// keeping numbers" is asking one question, not two (RFC 0004 R9) — even though
/// the two flags answer different halves of it: `metrics_enabled` is about
/// *exposure*, `history_enabled` is about *storage*.
///
/// ```toml
/// [stats]
/// # The rollup behind the dashboard's trend.
/// history_enabled        = true   # default
/// history_retention_days = 30     # 0 disables retention pruning
///
/// # The Prometheus recorder and the /metrics endpoint.
/// metrics_enabled        = true   # default: today's behaviour
/// ```
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatsConfig {
    /// Record the hourly cache-statistics rollup that backs the admin
    /// dashboard's trend. `false` restores the pre-RFC-0004 dashboard, which
    /// shows only counters since the current process started.
    #[serde(default = "default_true")]
    pub history_enabled: bool,

    /// How many days of rollup rows to keep. `0` disables pruning rather than
    /// disabling history — that is what `history_enabled = false` is for.
    ///
    /// One row per registry per hour is under 9 000 rows a year per registry,
    /// so this is a tidiness setting, not a storage argument (RFC 0004 R9).
    #[serde(default = "default_history_retention_days")]
    pub history_retention_days: u32,

    /// Install the Prometheus recorder and serve `/metrics`.
    ///
    /// **This is a security control, not a preference** (RFC 0004 §7).
    /// `/metrics` is unauthenticated and, before RFC 0004, unconditional: it
    /// publishes cache hit rates, per-registry pull volumes and upstream
    /// latencies to anyone who can reach the port. That is defensible behind an
    /// ingress that does not route it, and indefensible for a self-hoster who
    /// had no way to turn it off. Defaults to `true` so no existing scrape
    /// breaks on upgrade.
    #[serde(default = "default_true")]
    pub metrics_enabled: bool,
}

impl Default for StatsConfig {
    fn default() -> Self {
        Self {
            history_enabled: true,
            history_retention_days: default_history_retention_days(),
            metrics_enabled: true,
        }
    }
}

// ── Vulnerability scan ──────────────────────────────────────────────────────────

fn default_vuln_interval_secs() -> u64 {
    86_400
}

fn default_vuln_batch_size() -> usize {
    100
}

/// Periodic SBOM-vs-CVE re-check configuration.
///
/// ```toml
/// [vulnerability_scan]
/// enabled       = true
/// interval_secs = 86400          # daily
/// osv_api_url   = "https://api.osv.dev"
/// batch_size    = 100
/// ```
#[derive(Debug, Deserialize)]
pub struct VulnerabilityScanConfig {
    /// Enable the periodic background scan.
    #[serde(default)]
    pub enabled: bool,
    /// Seconds between scan runs. Defaults to one day.
    #[serde(default = "default_vuln_interval_secs")]
    pub interval_secs: u64,
    /// Base URL of the OSV API. Defaults to `https://api.osv.dev` when absent.
    #[serde(default)]
    pub osv_api_url: Option<String>,
    /// Number of SBOMs processed per page. Defaults to 100.
    #[serde(default = "default_vuln_batch_size")]
    pub batch_size: usize,
}

// ── Limits ────────────────────────────────────────────────────────────────────

/// Upload size limits.
///
/// ```toml
/// [limits]
/// max_artifact_size_bytes = 524288000  # 500 MiB
/// ```
#[derive(Debug, Deserialize, Default)]
pub struct LimitsConfig {
    /// Maximum artifact size for proxy downloads and local publishes.
    /// Defaults to 500 MiB when absent.
    pub max_artifact_size_bytes: Option<u64>,
}

impl AppConfig {
    /// The effective reverse-proxy trust list, or `None` when no policy is
    /// configured anywhere.
    ///
    /// `[server].trusted_proxies` is authoritative; the deprecated
    /// `[ip_blocking].trusted_proxies` is the fallback so existing configs keep
    /// working unchanged. An empty `[ip_blocking].trusted_proxies` does *not*
    /// count as a policy: it is that section's serde default, so it carries no
    /// operator intent (`[server].trusted_proxies = []`, being an `Option`,
    /// does — it means "trust nobody").
    pub fn effective_trusted_proxies(&self) -> Option<&[String]> {
        if let Some(list) = self.server.trusted_proxies.as_deref() {
            return Some(list);
        }
        self.ip_blocking
            .as_ref()
            .map(|c| c.trusted_proxies.as_slice())
            .filter(|l| !l.is_empty())
    }

    /// True when the deprecated `[ip_blocking].trusted_proxies` is the list
    /// actually in force (i.e. `[server].trusted_proxies` is absent).
    pub fn uses_deprecated_trusted_proxies(&self) -> bool {
        self.server.trusted_proxies.is_none()
            && self
                .ip_blocking
                .as_ref()
                .is_some_and(|c| !c.trusted_proxies.is_empty())
    }

    /// True when both trusted-proxy keys carry a list, so `[server]` shadows the
    /// deprecated one (surfaced as a warning, see [`AppConfig::warnings`]).
    pub fn shadows_deprecated_trusted_proxies(&self) -> bool {
        self.server.trusted_proxies.is_some()
            && self
                .ip_blocking
                .as_ref()
                .is_some_and(|c| !c.trusted_proxies.is_empty())
    }

    // ── Host-based routing ────────────────────────────────────────────────────

    /// The `base_domain` wildcard hosts hang off, when wildcard derivation is on.
    fn wildcard_base_domain(&self) -> Option<&str> {
        self.subdomain_routing
            .as_ref()
            .filter(|s| s.enabled)
            .and_then(|s| s.base_domain.as_deref())
    }

    /// Scheme used when advertising registry public URLs. Never affects routing.
    pub fn subdomain_scheme(&self) -> &str {
        self.subdomain_routing
            .as_ref()
            .map_or("https", |s| s.scheme.as_str())
    }

    /// Every `host -> registry` binding this config declares, explicit `hosts`
    /// entries before wildcard-derived ones.
    ///
    /// Materialised once at config-load time so a request-time lookup is a single
    /// hash lookup with no suffix parsing. Registry names that are not usable DNS
    /// labels simply contribute no wildcard binding (and a warning); duplicates
    /// across registries are rejected by [`AppConfig::validate`], not here.
    pub fn registry_host_bindings(&self) -> Vec<RegistryHostBinding> {
        let base_domain = self.wildcard_base_domain();
        let mut bindings = Vec::new();
        for registry in &self.registries {
            for host in &registry.hosts {
                let normalised = normalise_host(host);
                if normalised.is_empty() {
                    continue;
                }
                bindings.push(RegistryHostBinding {
                    host: normalised,
                    registry: registry.name.clone(),
                    explicit: true,
                });
            }
        }
        if let Some(base) = base_domain {
            for registry in &self.registries {
                if let Some(host) = wildcard_host(&registry.name, base) {
                    bindings.push(RegistryHostBinding {
                        host,
                        registry: registry.name.clone(),
                        explicit: false,
                    });
                }
            }
        }
        bindings
    }

    /// True when any host-based routing is configured — wildcard derivation is
    /// enabled, or some registry declares `hosts`.
    ///
    /// This is the trigger for the strict proxy-trust requirement: once a header
    /// selects a *registry*, a deployment with no stated trust policy is not a
    /// state we let it reach.
    pub fn host_routing_configured(&self) -> bool {
        self.subdomain_routing.as_ref().is_some_and(|s| s.enabled)
            || self.registries.iter().any(|r| !r.hosts.is_empty())
    }

    /// The preferred public URL of each registry that has one: the first explicit
    /// host when present, otherwise the wildcard host, prefixed with
    /// [`AppConfig::subdomain_scheme`]. This is what the API and UI advertise.
    pub fn registry_public_urls(&self) -> Vec<(String, String)> {
        let scheme = self.subdomain_scheme();
        let base_domain = self.wildcard_base_domain();
        self.registries
            .iter()
            .filter_map(|registry| {
                let host = registry
                    .hosts
                    .iter()
                    .map(|h| normalise_host(h))
                    .find(|h| !h.is_empty())
                    .or_else(|| base_domain.and_then(|b| wildcard_host(&registry.name, b)))?;
                Some((registry.name.clone(), format!("{scheme}://{host}")))
            })
            .collect()
    }

    /// Registry names that are reachable *only* by host (`path_routing = false`).
    pub fn host_only_registries(&self) -> Vec<String> {
        self.registries
            .iter()
            .filter(|r| !r.path_routing)
            .map(|r| r.name.clone())
            .collect()
    }

    /// Non-fatal configuration problems, a sibling of [`AppConfig::validate`].
    ///
    /// `validate` refuses to start; this reports states that degrade instead —
    /// a shadowed deprecated key, a permissive default left in place. Emitted as
    /// `tracing::warn!` at startup and on every reload, and served from
    /// `GET /api/v1/admin/config/warnings` so an operator sees them without
    /// grepping logs.
    pub fn warnings(&self) -> Vec<ConfigWarning> {
        let mut out = Vec::new();
        self.proxy_trust_warnings(&mut out);
        self.subdomain_warnings(&mut out);
        self.cors_warnings(&mut out);
        self.license_gate_warnings(&mut out);
        out
    }

    /// A `license_gate` on a registry type with no manifest parser.
    ///
    /// Licence extraction covers five of the twenty-one registry types
    /// (`LICENSE_EXTRACTION_TYPES`). On the other sixteen the licence is
    /// permanently unknown, so the gate either never fires or refuses
    /// everything — and which of those it does is `allow_unknown`. Both states
    /// are silent at runtime: nothing errors, the config is valid, and the rule
    /// is listed in the policy like any other.
    ///
    /// This is the same shape as the gates RFC 0004-bis §1 is about — a rule
    /// reporting the same green whether or not it can observe the condition it
    /// governs — so it is named at the point where the operator wrote it down.
    fn license_gate_warnings(&self, out: &mut Vec<ConfigWarning>) {
        for (index, registry) in self.registries.iter().enumerate() {
            let has_parser = LICENSE_EXTRACTION_TYPES.contains(&registry.registry_type.as_str());
            // The licence is recorded as a side effect of SBOM generation, so
            // SBOM being off makes even a supported registry type permanently
            // unknown. Checked first because it is the one an operator is most
            // likely to hit: the type is right, the rule is right, and nothing
            // happens.
            let sbom_on = registry.sbom.as_ref().is_some_and(|s| s.enabled);

            for (rule_index, rule) in registry.rules.iter().enumerate() {
                let RuleConfig::LicenseGate(cfg) = rule else {
                    continue;
                };
                let path = format!("registries[{index}].rules[{rule_index}]");
                let supported = LICENSE_EXTRACTION_TYPES.join(", ");

                if !sbom_on {
                    out.push(ConfigWarning::new(
                        warnings::LICENSE_GATE_SBOM_DISABLED,
                        path,
                        format!(
                            "registry '{}' has a license_gate but no enabled [registries.sbom] \
                             block. The licence is read out of the archive during SBOM \
                             generation, so with SBOM off nothing is ever extracted and this \
                             rule sees an unknown licence for every version — it will {}. Add \
                             [registries.sbom] with enabled = true.",
                            registry.name,
                            if cfg.block && !cfg.allow_unknown {
                                "therefore refuse every download"
                            } else {
                                "therefore never deny anything"
                            },
                        ),
                    ));
                    continue;
                }

                if has_parser {
                    continue;
                }

                if cfg.block && !cfg.allow_unknown {
                    out.push(ConfigWarning::new(
                        warnings::LICENSE_GATE_DENIES_EVERYTHING,
                        path,
                        format!(
                            "registry '{}' has type '{}', which has no manifest parser, so the \
                             licence of every version is unknown. With block = true and \
                             allow_unknown = false this rule refuses every download from this \
                             registry. Licence extraction currently covers: {supported}. Set \
                             allow_unknown = true, or remove the rule from this registry.",
                            registry.name, registry.registry_type,
                        ),
                    ));
                } else {
                    out.push(ConfigWarning::new(
                        warnings::LICENSE_GATE_NO_EXTRACTOR,
                        path,
                        format!(
                            "registry '{}' has type '{}', which has no manifest parser, so the \
                             licence of every version is unknown and this rule never denies \
                             anything. Licence extraction currently covers: {supported}. The \
                             allow/deny lists here have no effect — remove the rule, or keep it \
                             only as a record of intent.",
                            registry.name, registry.registry_type,
                        ),
                    ));
                }
            }
        }
    }

    /// `cors_allowed_origins = ["*"]` reopens what 1.1.0 closed by default. It is
    /// a legitimate choice for a public mirror, so this warns rather than
    /// refusing — but an operator who copied the wildcard from an old config
    /// without meaning to should see it named.
    fn cors_warnings(&self, out: &mut Vec<ConfigWarning>) {
        let origins = self
            .server
            .cors_allowed_origins
            .as_deref()
            .unwrap_or_default();
        if !origins.iter().any(|o| o == "*") {
            return;
        }
        out.push(ConfigWarning::new(
            warnings::CORS_ANY_ORIGIN,
            "server.cors_allowed_origins",
            "'*' allows any website to make cross-origin requests to this server and read \
             the responses. Credentials are never sent cross-origin, so this is not a \
             token-theft path, but on a private network it lets a public page enumerate \
             internal package metadata through a visitor's browser. Replace it with the \
             explicit origin(s) that serve the UI unless this is a public mirror.",
        ));
    }

    /// `[subdomain_routing]` is on but a registry name cannot become a DNS label.
    /// That registry gets no wildcard host; it stays reachable by path and by any
    /// explicit `hosts` entry, so this degrades rather than fails.
    fn subdomain_warnings(&self, out: &mut Vec<ConfigWarning>) {
        if self.wildcard_base_domain().is_none() {
            return;
        }
        for (index, registry) in self.registries.iter().enumerate() {
            if is_dns_label(&registry.name) {
                continue;
            }
            out.push(ConfigWarning::new(
                warnings::SUBDOMAIN_INVALID_DNS_LABEL,
                format!("registries[{index}].name"),
                format!(
                    "registry '{}' is not a valid DNS label, so no wildcard host is derived \
                     for it. It stays reachable at /proxy/{}/… and at any explicit 'hosts' \
                     entry. Rename it to letters, digits and inner hyphens only, or give it \
                     an explicit host.",
                    registry.name, registry.name
                ),
            ));
        }
    }

    fn proxy_trust_warnings(&self, out: &mut Vec<ConfigWarning>) {
        // Entries of the deprecated key that `validate` no longer rejects (see
        // there). Only reported when that key is the list actually in force: a
        // shadowed list changes nothing, and `PROXY_TRUST_SHADOWED_DEPRECATED_KEY`
        // already tells the operator to delete it — a second warning about an
        // entry of a list that has no effect is noise.
        for entry in self
            .ip_blocking
            .as_ref()
            .filter(|_| self.uses_deprecated_trusted_proxies())
            .map(|c| c.trusted_proxies.as_slice())
            .unwrap_or_default()
        {
            if parse_trusted_proxies(std::slice::from_ref(entry)).is_err() {
                out.push(ConfigWarning::new(
                    warnings::PROXY_TRUST_INVALID_DEPRECATED_ENTRY,
                    "ip_blocking.trusted_proxies",
                    format!(
                        "'{entry}' is not an IP address or CIDR range and is ignored. Replace it \
                         with the proxy's address(es) under [server].trusted_proxies; a hostname \
                         is not resolved."
                    ),
                ));
            }
        }
        if self.shadows_deprecated_trusted_proxies() {
            out.push(ConfigWarning::new(
                warnings::PROXY_TRUST_SHADOWED_DEPRECATED_KEY,
                "ip_blocking.trusted_proxies",
                "both [server].trusted_proxies and the deprecated \
                 [ip_blocking].trusted_proxies are set; [server] wins and this list is \
                 ignored entirely. Delete it.",
            ));
        } else if self.uses_deprecated_trusted_proxies() {
            out.push(ConfigWarning::new(
                warnings::PROXY_TRUST_DEPRECATED_KEY_ONLY,
                "ip_blocking.trusted_proxies",
                "proxy trust comes from the deprecated [ip_blocking].trusted_proxies. It is \
                 honoured — and now governs the forwarded host and scheme as well as the \
                 client IP — but move the list to [server].trusted_proxies.",
            ));
        } else if self.effective_trusted_proxies().is_none() && !self.host_routing_configured() {
            // With host routing on, no list at all is a hard error (see
            // `validate`), so this branch only ever describes the legacy case.
            out.push(ConfigWarning::new(
                warnings::PROXY_TRUST_UNCONFIGURED,
                "server.trusted_proxies",
                "no trusted-proxy list is configured, so Forwarded / X-Forwarded-Host / \
                 X-Forwarded-Proto are believed from any client and decide the URLs this \
                 server advertises. Set [server].trusted_proxies to your ingress's CIDR \
                 ranges, or to [] if BatleHub is exposed directly.",
            ));
        }
    }

    /// Fail-fast checks for host-based routing (RFC 0001 §4.3).
    ///
    /// Every condition here is one where the deployment would come up looking
    /// healthy but route wrongly — a host silently bound to the last registry
    /// that claimed it, a registry nothing can reach, the admin API shadowed by a
    /// vanity host, or routing driven by a header the server has no policy about.
    fn validate_host_routing(&self) -> Result<()> {
        if let Some(subdomain) = self.subdomain_routing.as_ref() {
            if subdomain.enabled {
                let base = subdomain.base_domain.as_deref().unwrap_or_default();
                if normalise_host(base).is_empty() {
                    bail!(
                        "[subdomain_routing]: 'enabled = true' requires a non-empty 'base_domain' \
                         (e.g. base_domain = \"hub.example.com\"); without one no wildcard host \
                         is derived and the section routes nothing"
                    );
                }
                // A pasted URL normalises to something non-empty ("https://hub.example.com" ->
                // "https"), so it would otherwise flow into the wildcard hosts, the public URLs
                // and the collision checks below as a plausible-looking domain.
                validate_host_entry(base).map_err(|e| {
                    anyhow::anyhow!("[subdomain_routing]: invalid 'base_domain' '{base}': {e}")
                })?;
            }
        }

        // Syntax first, so a pasted URL is reported as such rather than as a
        // mystery collision after normalisation.
        for registry in &self.registries {
            for host in &registry.hosts {
                validate_host_entry(host).map_err(|e| {
                    anyhow::anyhow!(
                        "registry '{}': invalid 'hosts' entry '{host}': {e}",
                        registry.name
                    )
                })?;
            }
        }

        // The bare base_domain stays the main host: it serves the admin API, the
        // SPA, /healthz and /metrics. A vanity host equal to it would rewrite all
        // of that into one registry and hide the admin API entirely.
        if let Some(base) = self
            .subdomain_routing
            .as_ref()
            .and_then(|s| s.base_domain.as_deref())
            .map(normalise_host)
            .filter(|d| !d.is_empty())
        {
            for registry in &self.registries {
                if registry.hosts.iter().any(|h| normalise_host(h) == base) {
                    bail!(
                        "registry '{}': 'hosts' entry '{base}' is the [subdomain_routing] \
                         base_domain itself; that host serves the admin API and the SPA, and \
                         binding it to a registry would hide them",
                        registry.name
                    );
                }
            }
        }

        // One host, one registry. Same-registry duplicates are fine — an explicit
        // `hosts` entry that repeats that registry's own wildcard host just wins.
        let mut claimed: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
        let bindings = self.registry_host_bindings();
        for binding in &bindings {
            match claimed.get(binding.host.as_str()) {
                Some(&owner) if owner != binding.registry => bail!(
                    "host '{}' is claimed by both registry '{owner}' and registry '{}'; \
                     a host routes to exactly one registry",
                    binding.host,
                    binding.registry
                ),
                Some(_) => {}
                None => {
                    claimed.insert(binding.host.as_str(), binding.registry.as_str());
                }
            }
        }

        // A registry with neither ingress is unreachable — catch it here rather
        // than as a stream of 404s nobody can explain.
        for registry in &self.registries {
            if registry.path_routing {
                continue;
            }
            let has_host = bindings.iter().any(|b| b.registry == registry.name);
            if !has_host {
                bail!(
                    "registry '{}': 'path_routing = false' leaves it with no ingress — it has \
                     no 'hosts' entry, and no wildcard host is derived for it (either \
                     [subdomain_routing] is off, or the name is not a valid DNS label). Add a \
                     host, or drop 'path_routing = false'.",
                    registry.name
                );
            }
        }

        // Routing now depends on a header, so an unstated trust policy is not a
        // state we let a deployment reach. The deprecated key counts as a policy
        // (and warns), so an existing config adopting host routing changes nothing.
        if self.host_routing_configured() && self.effective_trusted_proxies().is_none() {
            bail!(
                "host-based routing is configured ([subdomain_routing] or a registry 'hosts' \
                 entry), which routes on the Forwarded / X-Forwarded-Host header. Declare which \
                 peers may set it:\n\n\
                 \x20   [server]\n\
                 \x20   trusted_proxies = [\"10.42.0.0/16\"]   # your ingress's CIDR ranges\n\n\
                 Use trusted_proxies = [] if BatleHub is exposed directly and no proxy sits in \
                 front of it."
            );
        }

        Ok(())
    }

    pub fn validate(&self) -> Result<()> {
        if let Some(v) = self.config_version {
            if v > CURRENT_CONFIG_VERSION {
                bail!(
                    "config_version {v} is newer than this binary supports (max {CURRENT_CONFIG_VERSION}); \
                     upgrade batlehub-server, or lower config_version if you intended to target an \
                     older schema"
                );
            }
        }
        // Reject malformed proxy-trust entries up front: a typo here silently
        // widens or narrows which peers may set `X-Forwarded-*`, and the
        // consequence (a spoofable routing header, or generated URLs pointing at
        // the internal service host) shows up far from the config file.
        if let Some(list) = self.server.trusted_proxies.as_deref() {
            parse_trusted_proxies(list)
                .map_err(|e| anyhow::anyhow!("[server].trusted_proxies: {e}"))?;
        }
        // The deprecated `[ip_blocking].trusted_proxies` is deliberately *not*
        // validated here. Before it fed the proxy-trust policy it was parsed with
        // `.parse::<IpAddr>().ok()`, so a hostname entry (an ingress DNS name, a
        // Service name) loaded fine and was dropped. Bailing on it now would turn
        // an upgrade into a boot failure for a config that never changed; the
        // entry is dropped as before and surfaced as
        // `PROXY_TRUST_INVALID_DEPRECATED_ENTRY` instead.
        self.validate_host_routing()?;

        // The set of storage backend names a registry's `storage` field may
        // reference. In single-backend mode only the implicit "default" exists;
        // in multi mode it is the declared `[[storage.backends]]` names. A
        // registry pointing at an undeclared backend is rejected here rather
        // than silently falling back to the default backend at runtime (which
        // would write artifacts to the wrong place with no warning).
        let known_backends: std::collections::HashSet<&str> = match &self.storage {
            StoragesConfig::Single(_) => std::iter::once("default").collect(),
            StoragesConfig::Multi(multi) => {
                multi.backends.iter().map(|b| b.name.as_str()).collect()
            }
        };
        // Reject duplicate registry names: every downstream map (clients,
        // policies, rules, rate-limits, storage assignments) is keyed by name,
        // so a duplicate would silently shadow the earlier registry's config
        // (last-write-wins) with no error or log.
        let mut seen_names: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for registry in &self.registries {
            if registry.name.is_empty() {
                bail!("registry is missing a 'name' field");
            }
            if !seen_names.insert(registry.name.as_str()) {
                bail!(
                    "duplicate registry name '{}': registry names must be unique",
                    registry.name
                );
            }
            if let Some(backend) = &registry.storage {
                if !known_backends.contains(backend.as_str()) {
                    match &self.storage {
                        StoragesConfig::Single(_) => bail!(
                            "registry '{}': 'storage = \"{}\"' requires a multi-backend \
                             [[storage.backends]] configuration; single-backend storage has no \
                             named backends to select",
                            registry.name,
                            backend
                        ),
                        StoragesConfig::Multi(_) => bail!(
                            "registry '{}': 'storage = \"{}\"' does not match any backend name in \
                             [[storage.backends]]",
                            registry.name,
                            backend
                        ),
                    }
                }
            }
            let kind: batlehub_core::entities::RegistryKind =
                registry.registry_type.parse().map_err(anyhow::Error::msg)?;
            if matches!(registry.mode, RegistryMode::Local | RegistryMode::Hybrid)
                && !kind.supports_local_mode()
            {
                bail!(
                    "registry '{}': mode 'local'/'hybrid' is not supported for {} registries (no local publish model)",
                    registry.name,
                    kind
                );
            }
            if registry.mode == RegistryMode::Hybrid && registry.upstreams.is_empty() {
                bail!(
                    "registry '{}': hybrid mode requires at least one upstream URL",
                    registry.name
                );
            }
            // deb/rpm have no universal default upstream, so proxy mode (which would
            // otherwise fall back to an unreachable placeholder) also requires an
            // explicit upstream. Caught at startup instead of every fetch failing.
            if registry.mode == RegistryMode::Proxy
                && registry.upstreams.is_empty()
                && kind.requires_explicit_upstream_in_proxy_mode()
            {
                bail!(
                    "registry '{}': {} proxy mode requires at least one upstream URL (no default upstream exists)",
                    registry.name,
                    kind
                );
            }
            // `path_allow` gates a raw upstream path passthrough, so it only means
            // anything for the path-addressed kinds. Accepting it silently elsewhere
            // would read as a working restriction while gating nothing.
            if !registry.path_allow.is_empty() && !kind.is_path_addressed() {
                bail!(
                    "registry '{}': 'path_allow' is only supported for path-addressed registry types \
                     (deb, rpm, pacman, jetbrains, generic), not {}",
                    registry.name,
                    kind
                );
            }
            // A `generic` registry mirrors an arbitrary file tree on a host that may
            // serve unrelated content, so the allowlist is mandatory rather than
            // opt-in. `["**"]` is the explicit way to mirror everything.
            if kind == batlehub_core::entities::RegistryKind::Generic
                && registry.path_allow.is_empty()
            {
                bail!(
                    "registry '{}': generic registries require a non-empty 'path_allow' allowlist \
                     (use path_allow = [\"**\"] to mirror the whole upstream deliberately)",
                    registry.name
                );
            }
            for pattern in &registry.path_allow {
                glob::Pattern::new(pattern).map_err(|e| {
                    anyhow::anyhow!(
                        "registry '{}': invalid path_allow glob '{pattern}': {e}",
                        registry.name
                    )
                })?;
            }
            // `version_pattern` is a publish-time restriction (a security
            // control), so an uncompilable regex must fail the config load
            // rather than silently degrade to "allow every version" (fail-open).
            if let Some(versioning) = &registry.versioning {
                if let Some(pattern) = &versioning.version_pattern {
                    regex::Regex::new(pattern).map_err(|e| {
                        anyhow::anyhow!(
                            "registry '{}': invalid version_pattern '{pattern}': {e}",
                            registry.name
                        )
                    })?;
                }
            }
        }
        Ok(())
    }

    /// Apply environment variable overrides on top of the file-based config.
    ///
    /// **Preferred approach for secrets:** use `${VAR_NAME}` placeholders directly
    /// inside the TOML file — they are expanded before parsing, so they work for
    /// any field, including `client_secret`, upstream auth `token`/`password`/`value`,
    /// and any other string field.  See the docs for details.
    ///
    /// This method handles a fixed set of named overrides for non-secret top-level
    /// fields as a convenience.  Convention: `PROXY_CACHE__<SECTION>__<FIELD>`
    /// (double-underscore separator).
    ///
    /// Supported variables:
    /// | Variable                              | Field                        |
    /// |---------------------------------------|------------------------------|
    /// | `PROXY_CACHE__SERVER__HOST`           | `server.host`                |
    /// | `PROXY_CACHE__SERVER__PORT`           | `server.port`                |
    /// | `PROXY_CACHE__SERVER__STATIC_DIR`     | `server.static_dir`          |
    /// | `PROXY_CACHE__DATABASE__URL`          | `database.url`               |
    /// | `PROXY_CACHE__DATABASE__MAX_CONNECTIONS` | `database.max_connections` |
    /// | `PROXY_CACHE__DATABASE__MIN_CONNECTIONS` | `database.min_connections` |
    /// | `PROXY_CACHE__DATABASE__ACQUIRE_TIMEOUT_SECS` | `database.acquire_timeout_secs` |
    /// | `PROXY_CACHE__STORAGE__PATH`          | `storage.path` (single filesystem backend only)  |
    /// | `PROXY_CACHE__STORAGE__BUCKET`        | `storage.bucket` (single S3 backend only)        |
    /// | `PROXY_CACHE__STORAGE__REGION`        | `storage.region` (single S3 backend only)        |
    /// | `PROXY_CACHE__STORAGE__ENDPOINT_URL`  | `storage.endpoint_url` (single S3 backend only)  |
    /// | `PROXY_CACHE__OTEL__ENDPOINT`         | `otel.endpoint`              |
    /// | `PROXY_CACHE__OTEL__SERVICE_NAME`     | `otel.service_name`          |
    pub fn apply_env_overrides(&mut self) {
        let env = |key: &str| std::env::var(key).ok();

        fn parse_env_or_warn<T: std::str::FromStr>(key: &str, v: &str) -> Option<T> {
            match v.parse() {
                Ok(parsed) => Some(parsed),
                Err(_) => {
                    eprintln!(
                        "warning: environment override {key} has invalid value {v:?}; ignoring it \
                         and keeping the configured value"
                    );
                    None
                }
            }
        }

        if let Some(v) = env("PROXY_CACHE__SERVER__HOST") {
            self.server.host = v;
        }
        if let Some(v) = env("PROXY_CACHE__SERVER__PORT") {
            if let Some(p) = parse_env_or_warn("PROXY_CACHE__SERVER__PORT", &v) {
                self.server.port = p;
            }
        }
        if let Some(v) = env("PROXY_CACHE__SERVER__STATIC_DIR") {
            self.server.static_dir = Some(v);
        }
        if let Some(v) = env("PROXY_CACHE__DATABASE__URL") {
            self.database.url = v;
        }
        if let Some(v) = env("PROXY_CACHE__DATABASE__MAX_CONNECTIONS") {
            if let Some(n) = parse_env_or_warn("PROXY_CACHE__DATABASE__MAX_CONNECTIONS", &v) {
                self.database.max_connections = n;
            }
        }
        if let Some(v) = env("PROXY_CACHE__DATABASE__MIN_CONNECTIONS") {
            if let Some(n) = parse_env_or_warn("PROXY_CACHE__DATABASE__MIN_CONNECTIONS", &v) {
                self.database.min_connections = n;
            }
        }
        if let Some(v) = env("PROXY_CACHE__DATABASE__ACQUIRE_TIMEOUT_SECS") {
            if let Some(n) = parse_env_or_warn("PROXY_CACHE__DATABASE__ACQUIRE_TIMEOUT_SECS", &v) {
                self.database.acquire_timeout_secs = n;
            }
        }

        apply_storage_env_overrides(&mut self.storage, &env);
        apply_otel_env_overrides(&mut self.otel, &env);
        apply_proxy_env_overrides(&mut self.proxy, &env);
    }
}

fn apply_storage_env_overrides(storage: &mut StoragesConfig, env: &dyn Fn(&str) -> Option<String>) {
    let StoragesConfig::Single(ref mut backend) = storage else {
        return;
    };
    match backend {
        StorageBackendConfig::Filesystem(fs) => apply_filesystem_env_overrides(fs, env),
        StorageBackendConfig::S3(s3) => apply_s3_env_overrides(s3, env),
    }
}

fn apply_filesystem_env_overrides(
    fs: &mut FilesystemStorageConfig,
    env: &dyn Fn(&str) -> Option<String>,
) {
    if let Some(v) = env("PROXY_CACHE__STORAGE__PATH") {
        fs.path = v;
    }
}

fn apply_s3_env_overrides(s3: &mut S3StorageConfig, env: &dyn Fn(&str) -> Option<String>) {
    if let Some(v) = env("PROXY_CACHE__STORAGE__BUCKET") {
        s3.bucket = v;
    }
    if let Some(v) = env("PROXY_CACHE__STORAGE__REGION") {
        s3.region = v;
    }
    if let Some(v) = env("PROXY_CACHE__STORAGE__ENDPOINT_URL") {
        s3.endpoint_url = Some(v);
    }
}

fn apply_otel_env_overrides(otel: &mut Option<OtelConfig>, env: &dyn Fn(&str) -> Option<String>) {
    if let Some(v) = env("PROXY_CACHE__OTEL__ENDPOINT") {
        match otel {
            Some(o) => o.endpoint = v,
            None => {
                *otel = Some(OtelConfig {
                    endpoint: v,
                    service_name: server::default_service_name(),
                })
            }
        }
    }
    if let Some(v) = env("PROXY_CACHE__OTEL__SERVICE_NAME") {
        if let Some(o) = otel {
            o.service_name = v;
        }
    }
}

fn apply_proxy_env_overrides(
    proxy: &mut Option<UpstreamProxyConfig>,
    env: &dyn Fn(&str) -> Option<String>,
) {
    if let Some(v) = env("PROXY_CACHE__PROXY__URL") {
        match proxy {
            Some(p) => p.url = v,
            None => {
                *proxy = Some(UpstreamProxyConfig {
                    url: v,
                    username: env("PROXY_CACHE__PROXY__USERNAME"),
                    password: env("PROXY_CACHE__PROXY__PASSWORD"),
                    no_proxy: env("PROXY_CACHE__PROXY__NO_PROXY"),
                })
            }
        }
    }
    if let Some(p) = proxy {
        if let Some(v) = env("PROXY_CACHE__PROXY__USERNAME") {
            p.username = Some(v);
        }
        if let Some(v) = env("PROXY_CACHE__PROXY__PASSWORD") {
            p.password = Some(v);
        }
        if let Some(v) = env("PROXY_CACHE__PROXY__NO_PROXY") {
            p.no_proxy = Some(v);
        }
    }
}

#[cfg(test)]
mod tests;
