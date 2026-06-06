mod api;
mod cli;
mod config;
mod tui;

use anyhow::Result;
use clap::Parser;

use cli::{admin, auth, config_cmd, owner, package, publish, registry, setup, version, Cli, Command};
use config::ConfigFile;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Commands that need neither config nor a server connection
    if let Command::Config { cmd } = cli.command {
        return config_cmd::run(cmd);
    }
    if let Command::Completion { shell } = cli.command {
        use clap::CommandFactory;
        use clap_complete::generate;
        let mut cmd = Cli::command();
        generate(shell, &mut cmd, "batlehub-cli", &mut std::io::stdout());
        return Ok(());
    }
    let mut cfg = ConfigFile::load()?;

    if let Command::Setup { cmd } = cli.command {
        let resolved_server = cli.server.clone().or_else(|| {
            match cli.profile.as_deref() {
                Some(n) => cfg.profiles.get(n).and_then(|p| p.server_url.clone()),
                None => cfg.default.server_url.clone(),
            }
        });
        return setup::run(cmd, resolved_server.as_deref());
    }

    // Determine base URL for potential OIDC auto-refresh (before building the client)
    let base_url_for_refresh = cli
        .server
        .clone()
        .or_else(|| {
            cli.profile
                .as_deref()
                .and_then(|n| cfg.profiles.get(n))
                .or(Some(&cfg.default))
                .and_then(|p| p.server_url.clone())
        })
        .unwrap_or_else(|| "http://localhost:8080".to_string());

    // If the user supplied --token, use it directly; otherwise auto-resolve
    // (reads K8s token file or refreshes expiring OIDC token).
    let effective_token = if let Some(ref t) = cli.token {
        Some(t.clone())
    } else {
        api::auth::resolve_token(&base_url_for_refresh, cli.profile.as_deref(), &mut cfg).await?
    };

    let resolved = cfg.resolve(
        cli.profile.as_deref(),
        cli.server.clone(),
        effective_token,
        cli.registry.clone(),
    );

    let client = api::BatleHubClient::new(&resolved.server_url, resolved.token.as_deref())?;

    match cli.command {
        Command::Registry { cmd } => registry::run(cmd, &client, cli.json).await?,
        Command::Package { cmd } => {
            package::run(cmd, &client, resolved.registry.as_deref(), cli.json).await?
        }
        Command::Version { cmd } => version::run(cmd, &client).await?,
        Command::Owners { cmd } => owner::run(cmd, &client, cli.json).await?,
        Command::Publish(args) => publish::run(args, &client, resolved.registry.as_deref()).await?,
        Command::Auth { cmd } => auth::run(cmd, &client, cli.json, cli.profile.as_deref()).await?,
        Command::Admin { cmd } => admin::run(cmd, &client, cli.json).await?,
        Command::Tui => tui::run(client).await?,
        Command::Config { .. } | Command::Completion { .. } | Command::Setup { .. } => unreachable!(),
    }

    Ok(())
}
