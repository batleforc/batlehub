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
        type = "maven"
        name = "my-maven"
        "#);
        let config: AppConfig = toml::from_str(&toml).unwrap();
        let err = config.validate().expect_err("unknown registry type should fail validation");
        assert!(err.to_string().contains("maven"));
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
}
