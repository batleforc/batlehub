use anyhow::Result;
use clap::Subcommand;
use comfy_table::{Cell, Color, Table};

use crate::api::{package::PackageQuery, package::PackageStatus, BatleHubClient};

#[derive(Subcommand)]
pub enum PackageCommand {
    /// List packages (across all or a specific registry)
    List {
        /// Filter by registry name
        #[arg(long, short = 'r')]
        registry: Option<String>,
        /// Filter by name substring
        #[arg(long, short = 's')]
        search: Option<String>,
        /// Show only blocked packages
        #[arg(long)]
        blocked_only: bool,
        /// Page number (0-based)
        #[arg(long, default_value = "0")]
        page: u64,
        /// Results per page
        #[arg(long, default_value = "50")]
        per_page: u64,
    },
    /// Show all versions of a package
    Versions {
        /// Registry name
        registry: String,
        /// Package name
        name: String,
    },
}

pub async fn run(
    cmd: PackageCommand,
    client: &BatleHubClient,
    default_registry: Option<&str>,
    json: bool,
) -> Result<()> {
    match cmd {
        PackageCommand::List {
            registry,
            search,
            blocked_only,
            page,
            per_page,
        } => {
            let reg = registry.or_else(|| default_registry.map(str::to_string));
            let resp = client
                .list_packages(PackageQuery {
                    registry: reg,
                    name: search,
                    page,
                    per_page,
                })
                .await?;

            let items: Vec<_> = if blocked_only {
                resp.items
                    .into_iter()
                    .filter(|p| matches!(p.status, PackageStatus::Blocked { .. }))
                    .collect()
            } else {
                resp.items
            };

            if json {
                println!("{}", serde_json::to_string_pretty(&items)?);
            } else {
                let mut table = Table::new();
                table.set_header(["Registry", "Name", "Version", "Status", "Accesses"]);
                for p in &items {
                    let status_cell = match &p.status {
                        PackageStatus::Available => Cell::new("available").fg(Color::Green),
                        PackageStatus::Blocked { reason } => {
                            Cell::new(format!("blocked: {reason}")).fg(Color::Red)
                        }
                    };
                    table.add_row(vec![
                        Cell::new(&p.registry),
                        Cell::new(&p.name),
                        Cell::new(&p.version),
                        status_cell,
                        Cell::new(p.access_count),
                    ]);
                }
                println!("{table}");
                println!("{} / {} package(s)", items.len(), resp.total);
            }
        }
        PackageCommand::Versions { registry, name } => {
            let resp = client
                .list_packages(PackageQuery {
                    registry: Some(registry.clone()),
                    name: Some(name.clone()),
                    page: 0,
                    per_page: 200,
                })
                .await?;

            let items: Vec<_> = resp
                .items
                .into_iter()
                .filter(|p| p.name == name && p.registry == registry)
                .collect();

            if json {
                println!("{}", serde_json::to_string_pretty(&items)?);
            } else {
                let mut table = Table::new();
                table.set_header(["Version", "Status", "Accesses"]);
                for p in &items {
                    let status_cell = match &p.status {
                        PackageStatus::Available => Cell::new("available").fg(Color::Green),
                        PackageStatus::Blocked { reason } => {
                            Cell::new(format!("blocked: {reason}")).fg(Color::Red)
                        }
                    };
                    table.add_row(vec![
                        Cell::new(&p.version),
                        status_cell,
                        Cell::new(p.access_count),
                    ]);
                }
                println!("{table}");
                println!("{} version(s)", items.len());
            }
        }
    }
    Ok(())
}
