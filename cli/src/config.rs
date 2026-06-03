use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Profile {
    pub server_url: Option<String>,
    pub token: Option<String>,
    pub registry: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ConfigFile {
    #[serde(default)]
    pub default: Profile,
    #[serde(default)]
    pub profiles: HashMap<String, Profile>,
}

/// Resolved connection settings after merging config file + CLI overrides.
#[derive(Debug, Clone)]
pub struct ResolvedConfig {
    pub server_url: String,
    pub token: Option<String>,
    pub registry: Option<String>,
}

impl ConfigFile {
    pub fn config_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("batlehub")
            .join("config.toml")
    }

    pub fn load() -> Result<Self> {
        let path = Self::config_path();
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        toml::from_str(&content).with_context(|| format!("parsing {}", path.display()))
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating config dir {}", parent.display()))?;
        }
        let content = toml::to_string_pretty(self).context("serializing config")?;
        std::fs::write(&path, content).with_context(|| format!("writing {}", path.display()))
    }

    pub fn resolve(
        &self,
        profile: Option<&str>,
        server_override: Option<String>,
        token_override: Option<String>,
        registry_override: Option<String>,
    ) -> ResolvedConfig {
        let base = match profile {
            Some(name) => self.profiles.get(name).cloned().unwrap_or_default(),
            None => self.default.clone(),
        };

        let server_url = server_override
            .or(base.server_url)
            .unwrap_or_else(|| "http://localhost:8080".to_string());

        let token = token_override.or(base.token);
        let registry = registry_override.or(base.registry);

        ResolvedConfig {
            server_url,
            token,
            registry,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg_with_default(url: &str, token: &str) -> ConfigFile {
        ConfigFile {
            default: Profile {
                server_url: Some(url.to_string()),
                token: Some(token.to_string()),
                registry: None,
            },
            profiles: HashMap::new(),
        }
    }

    #[test]
    fn resolve_uses_default_when_no_profile() {
        let cfg = cfg_with_default("http://example.com", "tok");
        let r = cfg.resolve(None, None, None, None);
        assert_eq!(r.server_url, "http://example.com");
        assert_eq!(r.token.as_deref(), Some("tok"));
    }

    #[test]
    fn cli_overrides_win_over_default() {
        let cfg = cfg_with_default("http://example.com", "tok");
        let r = cfg.resolve(
            None,
            Some("http://other.com".into()),
            Some("override".into()),
            None,
        );
        assert_eq!(r.server_url, "http://other.com");
        assert_eq!(r.token.as_deref(), Some("override"));
    }

    #[test]
    fn resolve_uses_named_profile() {
        let mut cfg = ConfigFile::default();
        cfg.profiles.insert(
            "prod".into(),
            Profile {
                server_url: Some("http://prod.example.com".into()),
                token: Some("prod-tok".into()),
                registry: None,
            },
        );
        let r = cfg.resolve(Some("prod"), None, None, None);
        assert_eq!(r.server_url, "http://prod.example.com");
    }

    #[test]
    fn fallback_to_localhost_when_empty() {
        let cfg = ConfigFile::default();
        let r = cfg.resolve(None, None, None, None);
        assert_eq!(r.server_url, "http://localhost:8080");
        assert!(r.token.is_none());
    }
}
