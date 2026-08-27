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
    /// Delete a version's artifact; the version number is then spent forever
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
                // Two separate facts, and the second is the one people are
                // surprised by: the bytes go, *and* the number is spent. A
                // deleted version can never be republished, so "delete and
                // re-upload to fix it" is not a plan (RFC 0016 §4.4).
                eprint!(
                    "Delete {registry}/{name}@{version}? The artifact is dropped and the \
                     version number is spent permanently — {version} can never be published \
                     again. [y/N] "
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
