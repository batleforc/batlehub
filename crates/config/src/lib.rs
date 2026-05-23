pub mod schema;

pub use schema::AppConfig;

use anyhow::{Context, Result};
use std::path::Path;

pub fn load(path: impl AsRef<Path>) -> Result<AppConfig> {
    let raw = std::fs::read_to_string(path.as_ref())
        .with_context(|| format!("reading config file: {}", path.as_ref().display()))?;
    let mut config: AppConfig =
        toml::from_str(&raw).with_context(|| "parsing config TOML")?;
    config.apply_env_overrides();
    config.validate()?;
    Ok(config)
}

#[cfg(test)]
mod tests {
    use crate::schema::{AppConfig, AuthConfig, StorageBackendConfig, StoragesConfig};

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
        assert!(matches!(cfg.storage, StoragesConfig::Single(StorageBackendConfig::Filesystem(_))));
    }

    #[test]
    fn parse_config_with_static_token_auth() {
        let toml = format!("{}\n{}", minimal(), r#"
        [[auth]]
        type = "token"
        [[auth.tokens]]
        value = "secret"
        role = "admin"
        user_id = "alice"
        "#);
        let cfg = parse(&toml);
        assert_eq!(cfg.auth.len(), 1);
        assert!(matches!(cfg.auth[0], AuthConfig::Token(_)));
    }

    #[test]
    fn parse_config_with_oidc_auth() {
        let toml = format!("{}\n{}", minimal(), r#"
        [[auth]]
        type = "oidc"
        issuer_url = "https://idp.example.com"
        client_id = "my-client"
        "#);
        let cfg = parse(&toml);
        assert!(matches!(cfg.auth[0], AuthConfig::Oidc(_)));
    }

    #[test]
    fn parse_config_with_registry() {
        let toml = format!("{}\n{}", minimal(), r#"
        [[registries]]
        type = "github"
        name = "gh"
        "#);
        let cfg = parse(&toml);
        assert_eq!(cfg.registries.len(), 1);
        assert_eq!(cfg.registries[0].name, "gh");
        assert_eq!(cfg.registries[0].registry_type, "github");
    }

    #[test]
    fn unknown_registry_type_returns_validation_error() {
        let toml = format!("{}\n{}", minimal(), r#"
        [[registries]]
        type = "bogus-registry"
        name = "my-bogus"
        "#);
        let config: AppConfig = toml::from_str(&toml).unwrap();
        let err = config.validate().expect_err("unknown registry type should fail validation");
        assert!(err.to_string().contains("bogus-registry"));
    }

    #[test]
    fn registry_missing_name_returns_validation_error() {
        let toml = format!("{}\n{}", minimal(), r#"
        [[registries]]
        type = "github"
        name = ""
        "#);
        let config: AppConfig = toml::from_str(&toml).unwrap();
        assert!(config.validate().is_err(), "empty registry name should fail validation");
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
        let mut cfg: AppConfig = toml::from_str(minimal()).unwrap();
        std::env::set_var("PROXY_CACHE__DATABASE__URL", "postgresql://env-host/env-db");
        cfg.apply_env_overrides();
        std::env::remove_var("PROXY_CACHE__DATABASE__URL");
        assert_eq!(cfg.database.url, "postgresql://env-host/env-db");
    }

    #[test]
    fn env_override_replaces_server_port() {
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
        let toml = format!("{}\n{}", minimal(), r#"
        [cache]
        type = "postgres"
        "#);
        let cfg: AppConfig = toml::from_str(&toml).unwrap();
        assert_eq!(cfg.cache.cache_type, "postgres");
    }

    #[test]
    fn cache_policy_serve_stale_defaults_to_true() {
        let toml = format!("{}\n{}", minimal(), r#"
        [[registries]]
        type = "npm"
        name = "npmjs"
        [registries.cache]
        metadata_ttl_secs = 300
        "#);
        let cfg: AppConfig = toml::from_str(&toml).unwrap();
        assert!(cfg.registries[0].cache.serve_stale, "serve_stale should default to true");
    }

    #[test]
    fn cache_policy_serve_stale_can_be_disabled() {
        let toml = format!("{}\n{}", minimal(), r#"
        [[registries]]
        type = "npm"
        name = "npmjs"
        [registries.cache]
        metadata_ttl_secs = 300
        serve_stale = false
        "#);
        let cfg: AppConfig = toml::from_str(&toml).unwrap();
        assert!(!cfg.registries[0].cache.serve_stale);
    }

    #[test]
    fn parse_config_with_kubernetes_auth() {
        let toml = format!("{}\n{}", minimal(), r#"
        [[auth]]
        type = "kubernetes"
        api_server = "https://k8s.example.com"
        [auth.role_mappings]
        "system:masters" = "admin"
        "#);
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
        assert!(matches!(cfg.storage, StoragesConfig::Single(StorageBackendConfig::S3(_))));
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
        let toml = format!("{}\n{}", minimal(), r#"
        [otel]
        endpoint = "http://otel:4317"
        service_name = "old-name"
        "#);
        let mut cfg: AppConfig = toml::from_str(&toml).unwrap();
        std::env::set_var("PROXY_CACHE__OTEL__SERVICE_NAME", "new-name");
        cfg.apply_env_overrides();
        std::env::remove_var("PROXY_CACHE__OTEL__SERVICE_NAME");
        assert_eq!(cfg.otel.as_ref().unwrap().service_name, "new-name");
    }

    #[test]
    fn env_override_static_dir() {
        let mut cfg: AppConfig = toml::from_str(minimal()).unwrap();
        std::env::set_var("PROXY_CACHE__SERVER__STATIC_DIR", "/var/www");
        cfg.apply_env_overrides();
        std::env::remove_var("PROXY_CACHE__SERVER__STATIC_DIR");
        assert_eq!(cfg.server.static_dir.as_deref(), Some("/var/www"));
    }

    #[test]
    fn env_override_database_max_connections() {
        let mut cfg: AppConfig = toml::from_str(minimal()).unwrap();
        std::env::set_var("PROXY_CACHE__DATABASE__MAX_CONNECTIONS", "25");
        cfg.apply_env_overrides();
        std::env::remove_var("PROXY_CACHE__DATABASE__MAX_CONNECTIONS");
        assert_eq!(cfg.database.max_connections, 25);
    }

    #[test]
    fn parse_config_with_upstream_bearer_auth() {
        let toml = format!("{}\n{}", minimal(), r#"
        [[registries]]
        type = "npm"
        name = "private-npm"
        [registries.upstream_auth]
        type = "bearer"
        token = "secret-token"
        "#);
        let cfg: AppConfig = toml::from_str(&toml).unwrap();
        assert!(cfg.registries[0].upstream_auth.is_some());
        assert!(matches!(cfg.registries[0].upstream_auth, Some(crate::schema::UpstreamAuthConfig::Bearer(_))));
    }

    #[test]
    fn parse_config_with_upstream_basic_auth() {
        let toml = format!("{}\n{}", minimal(), r#"
        [[registries]]
        type = "cargo"
        name = "private-cargo"
        [registries.upstream_auth]
        type = "basic"
        username = "user"
        password = "pass"
        "#);
        let cfg: AppConfig = toml::from_str(&toml).unwrap();
        assert!(matches!(cfg.registries[0].upstream_auth, Some(crate::schema::UpstreamAuthConfig::Basic(_))));
    }

    #[test]
    fn parse_config_with_upstream_header_auth() {
        let toml = format!("{}\n{}", minimal(), r#"
        [[registries]]
        type = "npm"
        name = "api-keyed-npm"
        [registries.upstream_auth]
        type = "header"
        name = "X-API-Key"
        value = "my-key"
        "#);
        let cfg: AppConfig = toml::from_str(&toml).unwrap();
        assert!(matches!(cfg.registries[0].upstream_auth, Some(crate::schema::UpstreamAuthConfig::Header(_))));
    }

    #[test]
    fn parse_config_with_release_age_gate_rule() {
        let toml = format!("{}\n{}", minimal(), r#"
        [[registries]]
        type = "npm"
        name = "npmjs"
        [[registries.rules]]
        kind = "release_age_gate"
        min_age_secs = 7200
        bypass_roles = ["admin"]
        "#);
        let cfg: AppConfig = toml::from_str(&toml).unwrap();
        assert_eq!(cfg.registries[0].rules.len(), 1);
        assert!(matches!(cfg.registries[0].rules[0], crate::schema::RuleConfig::ReleaseAgeGate(_)));
    }

    #[test]
    fn parse_config_firewall_only() {
        let toml = format!("{}\n{}", minimal(), r#"
        [[registries]]
        type = "npm"
        name = "npmjs"
        firewall_only = true
        "#);
        let cfg: AppConfig = toml::from_str(&toml).unwrap();
        assert!(cfg.registries[0].firewall_only);
    }
}
