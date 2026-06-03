use anyhow::Result;
use clap::Subcommand;

use crate::api::BatleHubClient;

#[derive(Subcommand)]
pub enum VersionCommand {
    /// Yank a specific version (marks it unavailable but keeps it)
    Yank {
        /// Registry name
        registry: String,
        /// Package name
        name: String,
        /// Version string
        version: String,
    },
    /// Unyank a previously yanked version
    Unyank {
        /// Registry name
        registry: String,
        /// Package name
        name: String,
        /// Version string
        version: String,
    },
    /// Permanently delete a version
    Delete {
        /// Registry name
        registry: String,
        /// Package name
        name: String,
        /// Version string
        version: String,
        /// Skip confirmation prompt
        #[arg(long, short = 'y')]
        yes: bool,
    },
}

pub async fn run(cmd: VersionCommand, client: &BatleHubClient) -> Result<()> {
    match cmd {
        VersionCommand::Yank {
            registry,
            name,
            version,
        } => {
            client.yank_version(&registry, &name, &version).await?;
            println!("Yanked {registry}/{name}@{version}");
        }
        VersionCommand::Unyank {
            registry,
            name,
            version,
        } => {
            client.unyank_version(&registry, &name, &version).await?;
            println!("Unyanked {registry}/{name}@{version}");
        }
        VersionCommand::Delete {
            registry,
            name,
            version,
            yes,
        } => {
            if !yes {
                eprint!(
                    "Permanently delete {registry}/{name}@{version}? This cannot be undone. [y/N] "
                );
                let mut input = String::new();
                std::io::stdin().read_line(&mut input)?;
                if !input.trim().eq_ignore_ascii_case("y") {
                    println!("Aborted.");
                    return Ok(());
                }
            }
            client.delete_version(&registry, &name, &version).await?;
            println!("Deleted {registry}/{name}@{version}");
        }
    }
    Ok(())
}
