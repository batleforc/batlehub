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
