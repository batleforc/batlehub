use anyhow::Result;
use clap::Subcommand;
use comfy_table::Table;

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
    }
    Ok(())
}
