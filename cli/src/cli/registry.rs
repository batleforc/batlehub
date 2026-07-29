use std::path::PathBuf;

use anyhow::Result;
use clap::Subcommand;
use comfy_table::Table;

use crate::api::suggest::{
    render_client_env, render_mise_toml, render_toml, suggest_registries, SuggestedRegistry,
};
use crate::api::BatleHubClient;

#[derive(Subcommand)]
pub enum RegistryCommand {
    /// List all accessible registries
    List,
    /// Show details for a single registry
    Info {
        /// Registry name
        name: String,
    },
    /// Suggest the registries a project needs, from its mise.lock and manifests
    Suggest {
        /// Directory to scan (defaults to the current working directory)
        #[arg(long, short = 'd')]
        dir: Option<PathBuf>,

        /// How many subdirectory levels to scan for manifests (0 = root only)
        #[arg(long, default_value = "0")]
        depth: usize,

        /// Also print the client-side environment variables for each toolchain
        #[arg(long)]
        client_env: bool,

        /// Also print a mise [settings.url_replacements] block routing mise
        /// through the proxy
        #[arg(long)]
        mise: bool,

        /// Comment out every line of the --mise block, for committing into a
        /// shared mise.toml
        #[arg(long, requires = "mise")]
        mise_commented: bool,

        /// Include suggestions the server already has a registry for
        #[arg(long)]
        include_existing: bool,
    },
}

pub async fn run(cmd: RegistryCommand, client: &BatleHubClient, json: bool) -> Result<()> {
    match cmd {
        RegistryCommand::List => {
            let registries = client.list_registries().await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&registries)?);
            } else {
                let mut table = Table::new();
                table.set_header(["Name", "Type", "Mode"]);
                for r in &registries {
                    table.add_row([&r.name, &r.registry_type, &r.mode]);
                }
                println!("{table}");
                println!("{} registry/registries", registries.len());
            }
        }
        RegistryCommand::Info { name } => {
            let registries = client.list_registries().await?;
            let reg = registries
                .into_iter()
                .find(|r| r.name == name)
                .ok_or_else(|| anyhow::anyhow!("registry '{name}' not found"))?;
            if json {
                println!("{}", serde_json::to_string_pretty(&reg)?);
            } else {
                let mut table = Table::new();
                table.add_row(["Name", &reg.name]);
                table.add_row(["Type", &reg.registry_type]);
                table.add_row(["Mode", &reg.mode]);
                println!("{table}");
            }
        }
        RegistryCommand::Suggest {
            dir,
            depth,
            client_env,
            mise,
            mise_commented,
            include_existing,
        } => {
            let dir = match dir {
                Some(d) => d,
                None => std::env::current_dir()?,
            };
            let suggestions = suggest_registries(&dir, depth);

            // Scanning is purely local, so an unreachable server must not fail
            // the command — it only costs the "already configured" annotation.
            let existing = if include_existing {
                Vec::new()
            } else {
                match client.list_registries().await {
                    Ok(regs) => regs.into_iter().map(|r| r.registry_type).collect(),
                    Err(_) => Vec::new(),
                }
            };

            let (wanted, already): (Vec<_>, Vec<_>) = suggestions
                .into_iter()
                .partition(|s| include_existing || !existing.contains(&s.registry_type));

            if json {
                print_json(&wanted, &already, &client.base_url, mise_commented)?;
            } else {
                print_human(
                    &wanted,
                    &already,
                    &dir,
                    &client.base_url,
                    client_env,
                    mise.then_some(mise_commented),
                );
            }
        }
    }
    Ok(())
}

fn print_json(
    wanted: &[SuggestedRegistry],
    already: &[SuggestedRegistry],
    server_url: &str,
    mise_commented: bool,
) -> Result<()> {
    let to_json = |s: &SuggestedRegistry| {
        serde_json::json!({
            "name": s.name,
            "type": s.registry_type,
            "upstreams": s.upstreams,
            "path_allow": s.path_allow,
            "sources": s.sources,
            "note": s.note,
            "proxy_url": s.proxy_url(server_url),
            "client_env": s.resolved_env(server_url)
                .into_iter()
                .collect::<std::collections::BTreeMap<_, _>>(),
            "mise_url_replacements": s.mise_url_replacements(server_url)
                .into_iter()
                .collect::<std::collections::BTreeMap<_, _>>(),
        })
    };
    let out = serde_json::json!({
        "suggested": wanted.iter().map(to_json).collect::<Vec<_>>(),
        "already_configured": already.iter().map(to_json).collect::<Vec<_>>(),
        "toml": render_toml(wanted),
        "mise_toml": render_mise_toml(wanted, server_url, mise_commented),
    });
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}

fn print_human(
    wanted: &[SuggestedRegistry],
    already: &[SuggestedRegistry],
    dir: &std::path::Path,
    server_url: &str,
    client_env: bool,
    // `Some(commented)` when `--mise` was passed.
    mise: Option<bool>,
) {
    if wanted.is_empty() && already.is_empty() {
        println!("No package sources found in: {}", dir.display());
        println!(
            "Looked at: mise.lock, mise.toml, Cargo.toml, go.mod, package.json, \
             pyproject.toml, pom.xml, composer.json, *.gemspec, *.nuspec, *.csproj, \
             *.tf, environment.yml"
        );
        return;
    }

    if !already.is_empty() {
        println!(
            "Already covered by an existing registry of the same type: {}",
            already
                .iter()
                .map(|s| s.registry_type.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
        println!("(pass --include-existing to emit these too)");
        println!();
    }

    if wanted.is_empty() {
        println!("Every detected source is already covered by a configured registry.");
        return;
    }

    let mut table = Table::new();
    table.set_header(["Name", "Type", "Upstream", "Detected from"]);
    for s in wanted {
        let upstream = if s.upstreams.is_empty() {
            "(adapter default)".to_owned()
        } else {
            s.upstreams.join(", ")
        };
        table.add_row([
            s.name.clone(),
            s.registry_type.clone(),
            upstream,
            s.sources.join(", "),
        ]);
    }
    println!("{table}");
    println!();
    println!("Add to config.toml:");
    println!();
    println!("{}", render_toml(wanted));

    if let Some(commented) = mise {
        println!();
        println!("Route mise through the proxy:");
        println!();
        println!("{}", render_mise_toml(wanted, server_url, commented));
    }

    if client_env {
        println!();
        println!("Point clients at the proxy:");
        println!();
        println!("{}", render_client_env(wanted, server_url));
    }

    if !client_env || mise.is_none() {
        let mut hints = Vec::new();
        if !client_env {
            hints.push("--client-env for the matching client environment variables");
        }
        if mise.is_none() {
            hints.push("--mise for a mise [settings.url_replacements] block");
        }
        println!("Re-run with {}.", hints.join(", "));
    }
}
