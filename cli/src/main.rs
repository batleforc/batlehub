mod api;
mod cli;
mod config;
mod tui;

use anyhow::Result;
use clap::Parser;

use cli::{admin, auth, config_cmd, owner, package, publish, registry, version, Cli, Command};
use config::ConfigFile;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Config commands don't need a client
    if let Command::Config { cmd } = cli.command {
        return config_cmd::run(cmd);
    }

    let cfg = ConfigFile::load()?;
    let resolved = cfg.resolve(
        cli.profile.as_deref(),
        cli.server.clone(),
        cli.token.clone(),
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
        Command::Auth { cmd } => auth::run(cmd, &client, cli.json).await?,
        Command::Admin { cmd } => admin::run(cmd, &client, cli.json).await?,
        Command::Tui => tui::run(client).await?,
        Command::Config { .. } => unreachable!(),
    }

    Ok(())
}
