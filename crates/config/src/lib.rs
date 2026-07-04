pub mod schema;

pub use schema::AppConfig;

use anyhow::{bail, Context, Result};
use std::path::Path;

pub fn load(path: impl AsRef<Path>) -> Result<AppConfig> {
    let raw = std::fs::read_to_string(path.as_ref())
        .with_context(|| format!("reading config file: {}", path.as_ref().display()))?;
    load_from_str(&raw)
}

/// Parse a config from a raw TOML string.
///
/// Identical to `load` but takes the raw content directly instead of reading from disk.
/// Environment variable placeholders (`${VAR}`) are still expanded.
pub fn load_from_str(raw: &str) -> Result<AppConfig> {
    let expanded = expand_env_vars(raw)?;
    let mut config: AppConfig = toml::from_str(&expanded).with_context(|| "parsing config TOML")?;
    config.apply_env_overrides();
    config.validate()?;
    Ok(config)
}

/// Expand `${VAR_NAME}` placeholders in a raw config string with their
/// environment variable values.
///
/// Rules:
/// - `${VAR_NAME}` is replaced with `std::env::var("VAR_NAME")`.
///   Returns an error if the variable is not set.
/// - `$${VAR_NAME}` is an escape sequence that produces the literal string
///   `${VAR_NAME}` without any variable lookup.
/// - Any other `$` character is left unchanged.
/// - Placeholders inside a TOML `#` comment (i.e. outside of a quoted
///   string) are left untouched — commented-out example lines never
///   require the referenced variable to be set.
///
/// Read `${VAR_NAME}` from `chars` (the `$` and `{` have already been consumed),
/// look up the variable in the environment, and return its value.
fn expand_braced_var(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> Result<String> {
    let mut var_name = String::new();
    loop {
        match chars.next() {
            Some('}') => break,
            Some(c) => var_name.push(c),
            None => bail!("unclosed '${{...}}' placeholder in config file"),
        }
    }
    if var_name.is_empty() {
        bail!("empty variable name in '${{}}' placeholder in config file");
    }
    std::env::var(&var_name)
        .with_context(|| format!("config references env var '${{{var_name}}}' but it is not set"))
}

/// Tracks TOML string/comment context while scanning so `$` expansion only fires
/// in code positions (not inside `'single'` strings or `#` comments).
#[derive(Default)]
struct QuoteScan {
    in_dquote: bool,
    in_squote: bool,
    in_comment: bool,
}

impl QuoteScan {
    /// Update state for `ch`, pushing it to `out` when it is structural.
    /// Returns `true` when `ch` was consumed here (the caller should move on),
    /// `false` when it is an ordinary character still eligible for `$` handling.
    fn consume(
        &mut self,
        ch: char,
        out: &mut String,
        chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
    ) -> bool {
        if ch == '\n' {
            *self = QuoteScan::default();
            out.push(ch);
            return true;
        }
        if self.in_comment {
            out.push(ch);
            return true;
        }
        if ch == '"' && !self.in_squote {
            self.in_dquote = !self.in_dquote;
            out.push(ch);
            return true;
        }
        if ch == '\\' && self.in_dquote {
            // Don't let an escaped quote (\") toggle string state.
            out.push(ch);
            if let Some(next) = chars.next() {
                out.push(next);
            }
            return true;
        }
        if ch == '\'' && !self.in_dquote {
            self.in_squote = !self.in_squote;
            out.push(ch);
            return true;
        }
        if ch == '#' && !self.in_dquote && !self.in_squote {
            self.in_comment = true;
            out.push(ch);
            return true;
        }
        false
    }
}

fn expand_env_vars(raw: &str) -> Result<String> {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    let mut scan = QuoteScan::default();

    while let Some(ch) = chars.next() {
        if scan.consume(ch, &mut out, &mut chars) {
            continue;
        }
        if ch != '$' {
            out.push(ch);
            continue;
        }
        match chars.peek() {
            Some('$') => {
                // $${ ... } → literal ${ ... }
                chars.next();
                if chars.peek() == Some(&'{') {
                    out.push('$');
                } else {
                    out.push('$');
                    out.push('$');
                }
            }
            Some('{') => {
                chars.next(); // consume '{'
                out.push_str(&expand_braced_var(&mut chars)?);
            }
            _ => out.push('$'),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use crate::schema::{
        AppConfig, AuthConfig, StorageBackendConfig, StoragesConfig, CURRENT_CONFIG_VERSION,
    };
    use crate::{expand_env_vars, load};

    /// `cargo test` runs this crate's unit tests multi-threaded by default, but
    /// `std::env::set_var`/`remove_var` mutate the single process-wide
    /// environment table. Every test below that touches env vars acquires this
    /// lock first so their set/read/remove sequences never interleave.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn parse(toml: &str) -> AppConfig {
        let config: AppConfig = toml::from_str(toml).expect("parse failed");
        config.validate().expect("validate failed");
        config
    }

    fn minimal() -> &'static str {
        r#"
        [server]
        host = "127.0.0.1"
        port = 8080

        [database]
        type = "postgresql"
        url = "postgresql://user:pass@localhost/db"

        [storage]
        type = "filesystem"
        path = "./tmp"
        "#
    }

    #[test]
    fn parse_minimal_valid_config() {
        let cfg = parse(minimal());
        assert_eq!(cfg.server.port, 8080);
        assert_eq!(cfg.server.host, "127.0.0.1");
        assert_eq!(cfg.database.url, "postgresql://user:pass@localhost/db");
        assert!(cfg.registries.is_empty());
        assert!(cfg.auth.is_empty());
        assert!(matches!(
            cfg.storage,
            StoragesConfig::Single(StorageBackendConfig::Filesystem(_))
        ));
    }

    #[test]
    fn parse_config_with_static_token_auth() {
        let toml = format!(
            "{}\n{}",
            minimal(),
            r#"
        [[auth]]
        type = "token"
        [[auth.tokens]]
        value = "secret"
        role = "admin"
        user_id = "alice"
        "#
        );
        let cfg = parse(&toml);
        assert_eq!(cfg.auth.len(), 1);
        assert!(matches!(cfg.auth[0], AuthConfig::Token(_)));
    }

    #[test]
    fn parse_config_with_oidc_auth() {
        let toml = format!(
            "{}\n{}",
            minimal(),
            r#"
        [[auth]]
        type = "oidc"
        issuer_url = "https://idp.example.com"
        client_id = "my-client"
        "#
        );
        let cfg = parse(&toml);
        assert!(matches!(cfg.auth[0], AuthConfig::Oidc(_)));
    }

    #[test]
    fn parse_config_with_registry() {
        let toml = format!(
            "{}\n{}",
            minimal(),
            r#"
        [[registries]]
        type = "github"
        name = "gh"
        "#
        );
        let cfg = parse(&toml);
        assert_eq!(cfg.registries.len(), 1);
        assert_eq!(cfg.registries[0].name, "gh");
        assert_eq!(cfg.registries[0].registry_type, "github");
    }

    #[test]
    fn unknown_registry_type_returns_validation_error() {
        let toml = format!(
            "{}\n{}",
            minimal(),
            r#"
        [[registries]]
        type = "bogus-registry"
        name = "my-bogus"
        "#
        );
        let config: AppConfig = toml::from_str(&toml).unwrap();
        let err = config
            .validate()
            .expect_err("unknown registry type should fail validation");
        assert!(err.to_string().contains("bogus-registry"));
    }

    #[test]
    fn registry_missing_name_returns_validation_error() {
        let toml = format!(
            "{}\n{}",
            minimal(),
            r#"
        [[registries]]
        type = "github"
        name = ""
        "#
        );
        let config: AppConfig = toml::from_str(&toml).unwrap();
        assert!(
            config.validate().is_err(),
            "empty registry name should fail validation"
        );
    }

    #[test]
    fn composer_local_mode_passes_validation() {
        let toml = format!(
            "{}\n{}",
            minimal(),
            r#"
        [[registries]]
        type = "composer"
        name = "my-composer"
        mode = "local"
        "#
        );
        let config: AppConfig = toml::from_str(&toml).unwrap();
        config
            .validate()
            .expect("composer + local mode must be accepted");
    }

    #[test]
    fn composer_hybrid_mode_passes_validation() {
        let toml = format!(
            "{}\n{}",
            minimal(),
            r#"
        [[registries]]
        type = "composer"
        name = "my-composer"
        mode = "hybrid"
        upstreams = ["https://repo.packagist.org"]
        "#
        );
        let config: AppConfig = toml::from_str(&toml).unwrap();
        config
            .validate()
            .expect("composer + hybrid mode must be accepted");
    }

    #[test]
    fn jetbrains_proxy_mode_passes_without_upstream() {
        // jetbrains is proxy-only and has a real default upstream, so no explicit
        // `upstreams` is required (unlike deb/rpm).
        let toml = format!(
            "{}\n{}",
            minimal(),
            r#"
        [[registries]]
        type = "jetbrains"
        name = "jb"
        mode = "proxy"
        "#
        );
        let config: AppConfig = toml::from_str(&toml).unwrap();
        config
            .validate()
            .expect("jetbrains + proxy mode (no upstream) must be accepted");
    }

    #[test]
    fn jetbrains_local_mode_is_rejected() {
        // jetbrains is proxy-only — local/hybrid hosting is not supported.
        let toml = format!(
            "{}\n{}",
            minimal(),
            r#"
        [[registries]]
        type = "jetbrains"
        name = "jb"
        mode = "local"
        "#
        );
        let config: AppConfig = toml::from_str(&toml).unwrap();
        assert!(
            config.validate().is_err(),
            "jetbrains + local mode should fail validation"
        );
    }

    #[test]
    fn config_version_absent_defaults_to_none_and_passes_validation() {
        let cfg = parse(minimal());
        assert_eq!(cfg.config_version, None);
    }

    #[test]
    fn config_version_at_current_passes_validation() {
        let toml = format!("config_version = {}\n{}", CURRENT_CONFIG_VERSION, minimal());
        let cfg = parse(&toml);
        assert_eq!(cfg.config_version, Some(CURRENT_CONFIG_VERSION));
    }

    #[test]
    fn config_version_newer_than_supported_is_rejected() {
        let toml = format!(
            "config_version = {}\n{}",
            CURRENT_CONFIG_VERSION + 1,
            minimal()
        );
        let cfg: AppConfig = toml::from_str(&toml).expect("parse failed");
        let err = cfg
            .validate()
            .expect_err("a config_version newer than supported should fail validation");
        assert!(err.to_string().contains("config_version"));
    }

    #[test]
    fn require_signed_release_enabled_passes_validation() {
        let toml = format!(
            "{}\n{}",
            minimal(),
            r#"
        [[registries]]
        type = "github"
        name = "my-gh"

        [[registries.rules]]
        kind = "require_signed_release"
        enabled = true
        bypass_roles = ["admin"]
        deny_missing_signature = true
        "#
        );
        let config: AppConfig = toml::from_str(&toml).unwrap();
        config
            .validate()
            .expect("require_signed_release rule must be accepted");
    }

    #[test]
    fn server_defaults_applied_when_fields_absent() {
        let toml = r#"
        [server]
        # no host or port

        [database]
        type = "postgresql"
        url = "postgresql://u:p@h/d"

        [storage]
        type = "filesystem"
        path = "./tmp"
        "#;
        let cfg: AppConfig = toml::from_str(toml).unwrap();
        assert_eq!(cfg.server.host, "0.0.0.0");
        assert_eq!(cfg.server.port, 8080);
    }

    #[test]
    fn cors_allowed_origins_parses_correctly() {
        let toml_full = r#"
        [server]
        host = "0.0.0.0"
        port = 8080
        cors_allowed_origins = ["https://app.example.com", "https://staging.example.com"]

        [database]
        type = "postgresql"
        url = "postgresql://u:p@h/d"

        [storage]
        type = "filesystem"
        path = "./tmp"
        "#;
        let cfg: AppConfig = toml::from_str(toml_full).unwrap();
        let origins = cfg.server.cors_allowed_origins.unwrap();
        assert_eq!(origins.len(), 2);
        assert_eq!(origins[0], "https://app.example.com");
    }

    #[test]
    fn env_override_replaces_database_url() {
        let _guard = ENV_LOCK.lock().unwrap();
        let mut cfg: AppConfig = toml::from_str(minimal()).unwrap();
        std::env::set_var("PROXY_CACHE__DATABASE__URL", "postgresql://env-host/env-db");
        cfg.apply_env_overrides();
        std::env::remove_var("PROXY_CACHE__DATABASE__URL");
        assert_eq!(cfg.database.url, "postgresql://env-host/env-db");
    }

    #[test]
    fn env_override_replaces_server_port() {
        let _guard = ENV_LOCK.lock().unwrap();
        let mut cfg: AppConfig = toml::from_str(minimal()).unwrap();
        std::env::set_var("PROXY_CACHE__SERVER__PORT", "9090");
        cfg.apply_env_overrides();
        std::env::remove_var("PROXY_CACHE__SERVER__PORT");
        assert_eq!(cfg.server.port, 9090);
    }

    // ── Offline / stale-metadata config ──────────────────────────────────────

    #[test]
    fn cache_config_defaults_to_memory() {
        let cfg: AppConfig = toml::from_str(minimal()).unwrap();
        assert_eq!(cfg.cache.cache_type, "memory");
    }

    #[test]
    fn cache_config_explicit_postgres() {
        let toml = format!(
            "{}\n{}",
            minimal(),
            r#"
        [cache]
        type = "postgres"
        "#
        );
        let cfg: AppConfig = toml::from_str(&toml).unwrap();
        assert_eq!(cfg.cache.cache_type, "postgres");
    }

    #[test]
    fn cache_policy_serve_stale_defaults_to_true() {
        let toml = format!(
            "{}\n{}",
            minimal(),
            r#"
        [[registries]]
        type = "npm"
        name = "npmjs"
        [registries.cache]
        metadata_ttl_secs = 300
        "#
        );
        let cfg: AppConfig = toml::from_str(&toml).unwrap();
        assert!(
            cfg.registries[0].cache.serve_stale,
            "serve_stale should default to true"
        );
    }

    #[test]
    fn cache_policy_serve_stale_can_be_disabled() {
        let toml = format!(
            "{}\n{}",
            minimal(),
            r#"
        [[registries]]
        type = "npm"
        name = "npmjs"
        [registries.cache]
        metadata_ttl_secs = 300
        serve_stale = false
        "#
        );
        let cfg: AppConfig = toml::from_str(&toml).unwrap();
        assert!(!cfg.registries[0].cache.serve_stale);
    }

    #[test]
    fn parse_config_with_kubernetes_auth() {
        let toml = format!(
            "{}\n{}",
            minimal(),
            r#"
        [[auth]]
        type = "kubernetes"
        api_server = "https://k8s.example.com"
        [auth.role_mappings]
        "system:masters" = "admin"
        "#
        );
        let cfg: AppConfig = toml::from_str(&toml).unwrap();
        assert!(matches!(cfg.auth[0], AuthConfig::Kubernetes(_)));
        if let AuthConfig::Kubernetes(k8s) = &cfg.auth[0] {
            assert_eq!(k8s.api_server.as_deref(), Some("https://k8s.example.com"));
            assert_eq!(k8s.name, "kubernetes");
        }
    }

    #[test]
    fn parse_config_with_s3_storage() {
        let toml = r#"
        [server]
        host = "0.0.0.0"
        port = 8080

        [database]
        type = "postgresql"
        url = "postgresql://u:p@h/d"

        [storage]
        type = "s3"
        bucket = "my-bucket"
        region = "us-east-1"
        endpoint_url = "http://localhost:9000"
        force_path_style = true
        "#;
        let cfg: AppConfig = toml::from_str(toml).unwrap();
        assert!(matches!(
            cfg.storage,
            StoragesConfig::Single(StorageBackendConfig::S3(_))
        ));
    }

    #[test]
    fn parse_config_with_multi_storage() {
        let toml = r#"
        [server]
        host = "0.0.0.0"
        port = 8080

        [database]
        type = "postgresql"
        url = "postgresql://u:p@h/d"

        [storage]
        default = "primary"

        [[storage.backends]]
        name = "primary"
        type = "filesystem"
        path = "./tmp"

        [[storage.backends]]
        name = "secondary"
        type = "s3"
        bucket = "artifacts"
        region = "us-east-1"
        "#;
        let cfg: AppConfig = toml::from_str(toml).unwrap();
        assert!(matches!(cfg.storage, StoragesConfig::Multi(_)));
        if let StoragesConfig::Multi(m) = &cfg.storage {
            assert_eq!(m.default, "primary");
            assert_eq!(m.backends.len(), 2);
            assert_eq!(m.backends[0].name, "primary");
            assert_eq!(m.backends[1].name, "secondary");
        }
    }

    #[test]
    fn env_override_filesystem_storage_path() {
        let _guard = ENV_LOCK.lock().unwrap();
        let mut cfg: AppConfig = toml::from_str(minimal()).unwrap();
        std::env::set_var("PROXY_CACHE__STORAGE__PATH", "/new/path");
        cfg.apply_env_overrides();
        std::env::remove_var("PROXY_CACHE__STORAGE__PATH");
        if let StoragesConfig::Single(StorageBackendConfig::Filesystem(fs)) = &cfg.storage {
            assert_eq!(fs.path, "/new/path");
        } else {
            panic!("expected filesystem storage");
        }
    }

    #[test]
    fn env_override_s3_storage_fields() {
        let _guard = ENV_LOCK.lock().unwrap();
        let toml = r#"
        [server]
        host = "0.0.0.0"
        port = 8080

        [database]
        type = "postgresql"
        url = "postgresql://u:p@h/d"

        [storage]
        type = "s3"
        bucket = "old-bucket"
        region = "eu-west-1"
        "#;
        let mut cfg: AppConfig = toml::from_str(toml).unwrap();
        std::env::set_var("PROXY_CACHE__STORAGE__BUCKET", "new-bucket");
        std::env::set_var("PROXY_CACHE__STORAGE__REGION", "us-east-1");
        std::env::set_var("PROXY_CACHE__STORAGE__ENDPOINT_URL", "http://minio:9000");
        cfg.apply_env_overrides();
        std::env::remove_var("PROXY_CACHE__STORAGE__BUCKET");
        std::env::remove_var("PROXY_CACHE__STORAGE__REGION");
        std::env::remove_var("PROXY_CACHE__STORAGE__ENDPOINT_URL");
        if let StoragesConfig::Single(StorageBackendConfig::S3(s3)) = &cfg.storage {
            assert_eq!(s3.bucket, "new-bucket");
            assert_eq!(s3.region, "us-east-1");
            assert_eq!(s3.endpoint_url.as_deref(), Some("http://minio:9000"));
        } else {
            panic!("expected s3 storage");
        }
    }

    #[test]
    fn env_override_otel_creates_section_when_absent() {
        let _guard = ENV_LOCK.lock().unwrap();
        let mut cfg: AppConfig = toml::from_str(minimal()).unwrap();
        assert!(cfg.otel.is_none());
        std::env::set_var("PROXY_CACHE__OTEL__ENDPOINT", "http://otel:4317");
        cfg.apply_env_overrides();
        std::env::remove_var("PROXY_CACHE__OTEL__ENDPOINT");
        assert_eq!(cfg.otel.as_ref().unwrap().endpoint, "http://otel:4317");
        assert_eq!(cfg.otel.as_ref().unwrap().service_name, "batlehub");
    }

    #[test]
    fn env_override_otel_service_name_when_section_present() {
        let _guard = ENV_LOCK.lock().unwrap();
        let toml = format!(
            "{}\n{}",
            minimal(),
            r#"
        [otel]
        endpoint = "http://otel:4317"
        service_name = "old-name"
        "#
        );
        let mut cfg: AppConfig = toml::from_str(&toml).unwrap();
        std::env::set_var("PROXY_CACHE__OTEL__SERVICE_NAME", "new-name");
        cfg.apply_env_overrides();
        std::env::remove_var("PROXY_CACHE__OTEL__SERVICE_NAME");
        assert_eq!(cfg.otel.as_ref().unwrap().service_name, "new-name");
    }

    #[test]
    fn env_override_static_dir() {
        let _guard = ENV_LOCK.lock().unwrap();
        let mut cfg: AppConfig = toml::from_str(minimal()).unwrap();
        std::env::set_var("PROXY_CACHE__SERVER__STATIC_DIR", "/var/www");
        cfg.apply_env_overrides();
        std::env::remove_var("PROXY_CACHE__SERVER__STATIC_DIR");
        assert_eq!(cfg.server.static_dir.as_deref(), Some("/var/www"));
    }

    #[test]
    fn env_override_database_max_connections() {
        let _guard = ENV_LOCK.lock().unwrap();
        let mut cfg: AppConfig = toml::from_str(minimal()).unwrap();
        std::env::set_var("PROXY_CACHE__DATABASE__MAX_CONNECTIONS", "25");
        cfg.apply_env_overrides();
        std::env::remove_var("PROXY_CACHE__DATABASE__MAX_CONNECTIONS");
        assert_eq!(cfg.database.max_connections, 25);
    }

    #[test]
    fn database_pool_fields_default() {
        let cfg: AppConfig = toml::from_str(minimal()).unwrap();
        assert_eq!(cfg.database.min_connections, 1);
        assert_eq!(cfg.database.acquire_timeout_secs, 30);
    }

    #[test]
    fn env_override_database_min_connections() {
        let _guard = ENV_LOCK.lock().unwrap();
        let mut cfg: AppConfig = toml::from_str(minimal()).unwrap();
        std::env::set_var("PROXY_CACHE__DATABASE__MIN_CONNECTIONS", "3");
        cfg.apply_env_overrides();
        std::env::remove_var("PROXY_CACHE__DATABASE__MIN_CONNECTIONS");
        assert_eq!(cfg.database.min_connections, 3);
    }

    #[test]
    fn env_override_database_acquire_timeout_secs() {
        let _guard = ENV_LOCK.lock().unwrap();
        let mut cfg: AppConfig = toml::from_str(minimal()).unwrap();
        std::env::set_var("PROXY_CACHE__DATABASE__ACQUIRE_TIMEOUT_SECS", "5");
        cfg.apply_env_overrides();
        std::env::remove_var("PROXY_CACHE__DATABASE__ACQUIRE_TIMEOUT_SECS");
        assert_eq!(cfg.database.acquire_timeout_secs, 5);
    }

    // ── env var interpolation ──────────────────────────────────────────────────

    #[test]
    fn env_interpolation_basic() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("_TEST_EXPAND_BASIC", "hello");
        let result = expand_env_vars("value = \"${_TEST_EXPAND_BASIC}\"").unwrap();
        std::env::remove_var("_TEST_EXPAND_BASIC");
        assert_eq!(result, "value = \"hello\"");
    }

    #[test]
    fn env_interpolation_missing_var_errors() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("_TEST_EXPAND_MISSING");
        let err = expand_env_vars("x = \"${_TEST_EXPAND_MISSING}\"").unwrap_err();
        assert!(err.to_string().contains("_TEST_EXPAND_MISSING"));
    }

    #[test]
    fn env_interpolation_escape_produces_literal() {
        let result = expand_env_vars("x = \"$${LITERAL}\"").unwrap();
        assert_eq!(result, "x = \"${LITERAL}\"");
    }

    #[test]
    fn env_interpolation_bare_dollar_unchanged() {
        let result = expand_env_vars("x = \"price is $5\"").unwrap();
        assert_eq!(result, "x = \"price is $5\"");
    }

    #[test]
    fn env_interpolation_oidc_client_secret() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("_TEST_OIDC_SECRET", "super-secret");
        let toml = format!(
            "{}\n{}",
            minimal(),
            r#"
        [[auth]]
        type = "oidc"
        issuer_url = "https://idp.example.com"
        client_id = "my-client"
        client_secret = "${_TEST_OIDC_SECRET}"
        "#
        );
        let expanded = expand_env_vars(&toml).unwrap();
        std::env::remove_var("_TEST_OIDC_SECRET");
        let cfg: AppConfig = toml::from_str(&expanded).unwrap();
        if let AuthConfig::Oidc(oidc) = &cfg.auth[0] {
            assert_eq!(oidc.client_secret.as_deref(), Some("super-secret"));
        } else {
            panic!("expected OIDC auth");
        }
    }

    #[test]
    fn env_interpolation_upstream_bearer_token() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("_TEST_BEARER_TOKEN", "tok-abcdef");
        let toml = format!(
            "{}\n{}",
            minimal(),
            r#"
        [[registries]]
        type = "npm"
        name = "private-npm"
        [registries.upstream_auth]
        type = "bearer"
        token = "${_TEST_BEARER_TOKEN}"
        "#
        );
        let expanded = expand_env_vars(&toml).unwrap();
        std::env::remove_var("_TEST_BEARER_TOKEN");
        let cfg: AppConfig = toml::from_str(&expanded).unwrap();
        assert!(matches!(
            cfg.registries[0].upstream_auth,
            Some(crate::schema::UpstreamAuthConfig::Bearer(ref b)) if b.token == "tok-abcdef"
        ));
    }

    #[test]
    fn env_interpolation_upstream_basic_password() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("_TEST_BASIC_PASS", "s3cr3t");
        let toml = format!(
            "{}\n{}",
            minimal(),
            r#"
        [[registries]]
        type = "cargo"
        name = "private-cargo"
        [registries.upstream_auth]
        type = "basic"
        username = "deploy"
        password = "${_TEST_BASIC_PASS}"
        "#
        );
        let expanded = expand_env_vars(&toml).unwrap();
        std::env::remove_var("_TEST_BASIC_PASS");
        let cfg: AppConfig = toml::from_str(&expanded).unwrap();
        assert!(matches!(
            cfg.registries[0].upstream_auth,
            Some(crate::schema::UpstreamAuthConfig::Basic(ref b)) if b.password == "s3cr3t"
        ));
    }

    #[test]
    fn env_interpolation_multiple_vars_in_one_file() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("_TEST_MULTI_A", "val-a");
        std::env::set_var("_TEST_MULTI_B", "val-b");
        let result = expand_env_vars("a = \"${_TEST_MULTI_A}\"\nb = \"${_TEST_MULTI_B}\"").unwrap();
        std::env::remove_var("_TEST_MULTI_A");
        std::env::remove_var("_TEST_MULTI_B");
        assert_eq!(result, "a = \"val-a\"\nb = \"val-b\"");
    }

    #[test]
    fn env_interpolation_skips_commented_placeholder() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("_TEST_COMMENTED_UNSET");
        let result = expand_env_vars(
            "# token = \"${_TEST_COMMENTED_UNSET}\"   # export _TEST_COMMENTED_UNSET=tok\n",
        )
        .unwrap();
        assert_eq!(
            result,
            "# token = \"${_TEST_COMMENTED_UNSET}\"   # export _TEST_COMMENTED_UNSET=tok\n"
        );
    }

    #[test]
    fn env_interpolation_trailing_comment_after_value() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("_TEST_TRAILING_COMMENT", "real-value");
        let result =
            expand_env_vars("x = \"${_TEST_TRAILING_COMMENT}\" # uses ${_TEST_TRAILING_COMMENT}\n")
                .unwrap();
        std::env::remove_var("_TEST_TRAILING_COMMENT");
        assert_eq!(
            result,
            "x = \"real-value\" # uses ${_TEST_TRAILING_COMMENT}\n"
        );
    }

    #[test]
    fn env_interpolation_two_vars_in_same_value() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("_TEST_CONCAT_USER", "admin");
        std::env::set_var("_TEST_CONCAT_PASS", "s3cr3t");
        let result =
            expand_env_vars("url = \"${_TEST_CONCAT_USER}:${_TEST_CONCAT_PASS}@host\"").unwrap();
        std::env::remove_var("_TEST_CONCAT_USER");
        std::env::remove_var("_TEST_CONCAT_PASS");
        assert_eq!(result, "url = \"admin:s3cr3t@host\"");
    }

    #[test]
    fn env_interpolation_value_with_special_chars() {
        let _guard = ENV_LOCK.lock().unwrap();
        // Real-world passwords contain @, /, =, :, !, +
        std::env::set_var("_TEST_SPECIAL_CHARS", "P@ss/w=rd:1!+x");
        let result = expand_env_vars("password = \"${_TEST_SPECIAL_CHARS}\"").unwrap();
        std::env::remove_var("_TEST_SPECIAL_CHARS");
        assert_eq!(result, "password = \"P@ss/w=rd:1!+x\"");
    }

    #[test]
    fn env_interpolation_double_dollar_not_brace_is_literal() {
        // $$ not followed by { → passes through as $$
        let result = expand_env_vars("x = \"$$VAR\"").unwrap();
        assert_eq!(result, "x = \"$$VAR\"");
    }

    #[test]
    fn env_interpolation_empty_input_is_ok() {
        let result = expand_env_vars("").unwrap();
        assert_eq!(result, "");
    }

    #[test]
    fn env_interpolation_no_placeholders_is_unchanged() {
        let input = "[server]\nhost = \"0.0.0.0\"\nport = 8080\n";
        let result = expand_env_vars(input).unwrap();
        assert_eq!(result, input);
    }

    #[test]
    fn env_interpolation_substituted_value_is_not_re_expanded() {
        let _guard = ENV_LOCK.lock().unwrap();
        // The substituted value itself may contain ${...} — it must NOT be re-expanded.
        std::env::set_var("_TEST_NO_REEXPAND", "${_TEST_EXPAND_BASIC}");
        let result = expand_env_vars("x = \"${_TEST_NO_REEXPAND}\"").unwrap();
        std::env::remove_var("_TEST_NO_REEXPAND");
        assert_eq!(result, "x = \"${_TEST_EXPAND_BASIC}\"");
    }

    #[test]
    fn env_interpolation_upstream_header_value() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("_TEST_API_KEY", "my-api-key-xyz");
        let toml = format!(
            "{}\n{}",
            minimal(),
            r#"
        [[registries]]
        type = "npm"
        name = "api-keyed-npm"
        [registries.upstream_auth]
        type = "header"
        name = "X-API-Key"
        value = "${_TEST_API_KEY}"
        "#
        );
        let expanded = expand_env_vars(&toml).unwrap();
        std::env::remove_var("_TEST_API_KEY");
        let cfg: AppConfig = toml::from_str(&expanded).unwrap();
        assert!(matches!(
            cfg.registries[0].upstream_auth,
            Some(crate::schema::UpstreamAuthConfig::Header(ref h)) if h.value == "my-api-key-xyz"
        ));
    }

    #[test]
    fn env_interpolation_database_url_via_load() {
        let _guard = ENV_LOCK.lock().unwrap();
        let path = std::env::temp_dir().join("_batlehub_test_load_expand.toml");
        std::fs::write(
            &path,
            r#"
[server]
host = "127.0.0.1"
port = 8080

[database]
type = "postgresql"
url  = "${_TEST_LOAD_DB_URL}"

[storage]
type = "filesystem"
path = "./tmp"
"#,
        )
        .unwrap();
        std::env::set_var(
            "_TEST_LOAD_DB_URL",
            "postgresql://env-user:env-pass@db/mydb",
        );
        let cfg = load(&path).expect("load failed");
        std::env::remove_var("_TEST_LOAD_DB_URL");
        let _ = std::fs::remove_file(&path);
        assert_eq!(cfg.database.url, "postgresql://env-user:env-pass@db/mydb");
    }

    #[test]
    fn env_interpolation_unclosed_placeholder_errors() {
        let err = expand_env_vars("x = \"${UNCLOSED\"").unwrap_err();
        assert!(err.to_string().contains("unclosed"));
    }

    #[test]
    fn env_interpolation_empty_var_name_errors() {
        let err = expand_env_vars("x = \"${}\"").unwrap_err();
        assert!(err.to_string().contains("empty variable name"));
    }

    #[test]
    fn parse_config_with_upstream_bearer_auth() {
        let toml = format!(
            "{}\n{}",
            minimal(),
            r#"
        [[registries]]
        type = "npm"
        name = "private-npm"
        [registries.upstream_auth]
        type = "bearer"
        token = "secret-token"
        "#
        );
        let cfg: AppConfig = toml::from_str(&toml).unwrap();
        assert!(cfg.registries[0].upstream_auth.is_some());
        assert!(matches!(
            cfg.registries[0].upstream_auth,
            Some(crate::schema::UpstreamAuthConfig::Bearer(_))
        ));
    }

    #[test]
    fn parse_config_with_upstream_basic_auth() {
        let toml = format!(
            "{}\n{}",
            minimal(),
            r#"
        [[registries]]
        type = "cargo"
        name = "private-cargo"
        [registries.upstream_auth]
        type = "basic"
        username = "user"
        password = "pass"
        "#
        );
        let cfg: AppConfig = toml::from_str(&toml).unwrap();
        assert!(matches!(
            cfg.registries[0].upstream_auth,
            Some(crate::schema::UpstreamAuthConfig::Basic(_))
        ));
    }

    #[test]
    fn parse_config_with_upstream_header_auth() {
        let toml = format!(
            "{}\n{}",
            minimal(),
            r#"
        [[registries]]
        type = "npm"
        name = "api-keyed-npm"
        [registries.upstream_auth]
        type = "header"
        name = "X-API-Key"
        value = "my-key"
        "#
        );
        let cfg: AppConfig = toml::from_str(&toml).unwrap();
        assert!(matches!(
            cfg.registries[0].upstream_auth,
            Some(crate::schema::UpstreamAuthConfig::Header(_))
        ));
    }

    #[test]
    fn parse_config_with_release_age_gate_rule() {
        let toml = format!(
            "{}\n{}",
            minimal(),
            r#"
        [[registries]]
        type = "npm"
        name = "npmjs"
        [[registries.rules]]
        kind = "release_age_gate"
        min_age_secs = 7200
        bypass_roles = ["admin"]
        "#
        );
        let cfg: AppConfig = toml::from_str(&toml).unwrap();
        assert_eq!(cfg.registries[0].rules.len(), 1);
        assert!(matches!(
            cfg.registries[0].rules[0],
            crate::schema::RuleConfig::ReleaseAgeGate(_)
        ));
    }

    #[test]
    fn parse_config_firewall_only() {
        let toml = format!(
            "{}\n{}",
            minimal(),
            r#"
        [[registries]]
        type = "npm"
        name = "npmjs"
        firewall_only = true
        "#
        );
        let cfg: AppConfig = toml::from_str(&toml).unwrap();
        assert!(cfg.registries[0].firewall_only);
    }

    /// The S3 example config showcases every registry type and variant. Loading it
    /// through the real `load()` path (parse + env expansion + validate) guards it
    /// from drift and proves the new registry types/blocks are accepted. It must be
    /// self-contained (no `${VAR}` env placeholders) so it loads with no setup.
    #[test]
    fn example_s3_config_loads_and_validates() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../config.example-s3.toml");
        let cfg = load(path).expect("config.example-s3.toml must load and validate");
        let types: std::collections::HashSet<&str> = cfg
            .registries
            .iter()
            .map(|r| r.registry_type.as_str())
            .collect();
        for expected in [
            "github",
            "forgejo",
            "gitlab",
            "npm",
            "cargo",
            "goproxy",
            "openvsx",
            "vscode-marketplace",
            "maven",
            "rubygems",
            "terraform",
            "composer",
            "pypi",
            "conda",
            "nuget",
            "deb",
            "rpm",
        ] {
            assert!(
                types.contains(expected),
                "s3 example is missing a '{expected}' registry"
            );
        }
        // The signed deb/rpm hosting variants must carry a repo_signing key.
        assert!(cfg
            .registries
            .iter()
            .any(|r| r.registry_type == "deb" && r.repo_signing.is_some()));
        assert!(cfg
            .registries
            .iter()
            .any(|r| r.registry_type == "rpm" && r.repo_signing.is_some()));
    }
}
