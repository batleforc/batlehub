use anyhow::Result;
use clap::Subcommand;

use crate::config::ConfigFile;

#[derive(Subcommand)]
pub enum ConfigCommand {
    /// Interactive setup wizard
    Init,
    /// Print resolved configuration
    Show {
        /// Profile to show (defaults to 'default')
        #[arg(long)]
        profile: Option<String>,
    },
    /// Set a config value in the default profile
    Set {
        /// Key: server_url | token | registry
        key: String,
        /// Value to set
        value: String,
        /// Profile to write to (defaults to 'default')
        #[arg(long)]
        profile: Option<String>,
    },
}

pub fn run(cmd: ConfigCommand) -> Result<()> {
    match cmd {
        ConfigCommand::Init => {
            interactive_init()?;
        }
        ConfigCommand::Show { profile } => {
            let cfg = ConfigFile::load()?;
            let resolved = cfg.resolve(profile.as_deref(), None, None, None);
            println!("server_url: {}", resolved.server_url);
            println!(
                "token:      {}",
                resolved
                    .token
                    .as_deref()
                    .map(mask_token)
                    .unwrap_or_else(|| "(not set)".to_string())
            );
            println!(
                "registry:   {}",
                resolved.registry.as_deref().unwrap_or("(not set)")
            );
            println!();
            println!("Config file: {}", ConfigFile::config_path().display());
        }
        ConfigCommand::Set {
            key,
            value,
            profile,
        } => {
            let mut cfg = ConfigFile::load()?;
            let entry = match profile.as_deref() {
                Some(name) => cfg.profiles.entry(name.to_string()).or_default(),
                None => &mut cfg.default,
            };
            match key.as_str() {
                "server_url" => entry.server_url = Some(value.clone()),
                "token" => entry.token = Some(value.clone()),
                "registry" => entry.registry = Some(value.clone()),
                other => {
                    anyhow::bail!("unknown key '{other}'; expected server_url, token, or registry")
                }
            }
            cfg.save()?;
            println!("Set {key} = {value}");
        }
    }
    Ok(())
}

fn interactive_init() -> Result<()> {
    use std::io::{self, Write};

    println!("BatleHub CLI — initial setup");
    println!("(press Enter to accept the default shown in brackets)");
    println!();

    let server_url = prompt("Server URL", "http://localhost:8080")?;
    let token = prompt("Auth token", "")?;
    let registry = prompt("Default registry (optional)", "")?;

    let mut cfg = ConfigFile::load().unwrap_or_default();
    cfg.default.server_url = Some(server_url);
    if !token.is_empty() {
        cfg.default.token = Some(token);
    }
    if !registry.is_empty() {
        cfg.default.registry = Some(registry);
    }
    cfg.save()?;
    println!();
    println!("Config written to {}", ConfigFile::config_path().display());

    fn prompt(label: &str, default: &str) -> io::Result<String> {
        if default.is_empty() {
            print!("{label}: ");
        } else {
            print!("{label} [{default}]: ");
        }
        io::stdout().flush()?;
        let mut line = String::new();
        io::stdin().read_line(&mut line)?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            Ok(default.to_string())
        } else {
            Ok(trimmed.to_string())
        }
    }

    Ok(())
}

fn mask_token(t: &str) -> String {
    if t.len() <= 8 {
        return "****".to_string();
    }
    format!("{}…{}", &t[..4], &t[t.len() - 4..])
}
