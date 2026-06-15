pub mod admin;
pub mod auth;
pub mod config_cmd;
pub mod download;
pub mod owner;
pub mod package;
pub mod publish;
pub mod registry;
pub mod setup;
pub mod version;

use clap::{Parser, Subcommand};
use clap_complete::Shell;

#[derive(Parser)]
#[command(
    name = "batlehub-cli",
    about = "BatleHub CLI — interact with a BatleHub registry server",
    version
)]
pub struct Cli {
    /// Config profile to use (from ~/.config/batlehub/config.toml)
    #[arg(long, short = 'P', global = true, env = "BATLEHUB_PROFILE")]
    pub profile: Option<String>,

    /// Override server URL
    #[arg(long, global = true, env = "BATLEHUB_SERVER")]
    pub server: Option<String>,

    /// Override auth token
    #[arg(long, global = true, env = "BATLEHUB_TOKEN")]
    pub token: Option<String>,

    /// Default registry name
    #[arg(long, short = 'r', global = true, env = "BATLEHUB_REGISTRY")]
    pub registry: Option<String>,

    /// Output raw JSON instead of pretty tables
    #[arg(long, global = true)]
    pub json: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// List and inspect registries
    Registry {
        #[command(subcommand)]
        cmd: registry::RegistryCommand,
    },
    /// List and inspect packages
    Package {
        #[command(subcommand)]
        cmd: package::PackageCommand,
    },
    /// Yank, unyank, or delete specific versions
    Version {
        #[command(subcommand)]
        cmd: version::VersionCommand,
    },
    /// Manage package owners
    Owners {
        #[command(subcommand)]
        cmd: owner::OwnerCommand,
    },
    /// Publish an artifact to a local/hybrid registry
    Publish(publish::PublishArgs),
    /// Download a file through the proxy cache (warms path-addressed registries)
    Download(download::DownloadArgs),
    /// Authentication commands (tokens, whoami)
    Auth {
        #[command(subcommand)]
        cmd: auth::AuthCommand,
    },
    /// Admin operations (quota, ip-block, config, cache, banner, audit)
    Admin {
        #[command(subcommand)]
        cmd: admin::AdminCommand,
    },
    /// Manage CLI configuration
    Config {
        #[command(subcommand)]
        cmd: config_cmd::ConfigCommand,
    },
    /// Detect project type and print registry setup instructions
    Setup {
        #[command(subcommand)]
        cmd: setup::SetupCommand,
    },
    /// Launch interactive TUI
    Tui,
    /// Print shell completion script to stdout
    Completion {
        /// Target shell
        #[arg(value_enum)]
        shell: Shell,
    },
}
