use anyhow::Result;
use clap::Subcommand;
use comfy_table::Table;

use crate::api::{admin::AuditQuery, BatleHubClient};

#[derive(Subcommand)]
pub enum AdminCommand {
    /// Quota management
    Quota {
        #[command(subcommand)]
        cmd: QuotaCommand,
    },
    /// IP block management
    IpBlock {
        #[command(subcommand)]
        cmd: IpBlockCommand,
    },
    /// Configuration management
    Config {
        #[command(subcommand)]
        cmd: ConfigAdminCommand,
    },
    /// Cache management
    Cache {
        #[command(subcommand)]
        cmd: CacheCommand,
    },
    /// Global banner management
    Banner {
        #[command(subcommand)]
        cmd: BannerCommand,
    },
    /// Query the access audit log
    AuditLog {
        #[arg(long)]
        registry: Option<String>,
        #[arg(long)]
        user: Option<String>,
        #[arg(long)]
        from: Option<String>,
        #[arg(long)]
        to: Option<String>,
        /// Show only denied requests
        #[arg(long)]
        denied_only: bool,
        #[arg(long, default_value = "0")]
        page: u64,
        #[arg(long, default_value = "50")]
        per_page: u64,
    },
}

#[derive(Subcommand)]
pub enum QuotaCommand {
    /// List quota usage
    List {
        /// Filter by registry
        #[arg(long, short = 'r')]
        registry: Option<String>,
    },
    /// Reset quota for a specific user in a registry
    Reset { registry: String, user: String },
}

#[derive(Subcommand)]
pub enum IpBlockCommand {
    /// List blocked IPs
    List,
    /// Block an IP address
    Add {
        ip: String,
        #[arg(long)]
        reason: Option<String>,
    },
    /// Unblock an IP address
    Remove { ip: String },
}

#[derive(Subcommand)]
pub enum ConfigAdminCommand {
    /// Trigger an immediate config reload on the server
    Reload,
    /// Show recent config change history
    Changes,
}

#[derive(Subcommand)]
pub enum CacheCommand {
    /// Pre-warm the cache for a registry
    Warm {
        registry: String,
        /// Comma-separated list of package names to warm
        #[arg(long)]
        packages: Option<String>,
    },
    /// Clear the metadata cache for a registry
    Clear { registry: String },
}

#[derive(Subcommand)]
pub enum BannerCommand {
    /// Set the global admin banner
    Set {
        message: String,
        #[arg(long, default_value = "info")]
        level: String,
    },
    /// Clear the global admin banner
    Clear,
}

pub async fn run(cmd: AdminCommand, client: &BatleHubClient, json: bool) -> Result<()> {
    match cmd {
        AdminCommand::Quota { cmd } => match cmd {
            QuotaCommand::List { registry } => {
                let entries = client.list_quota(registry.as_deref()).await?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&entries)?);
                } else {
                    let mut table = Table::new();
                    table.set_header(["Registry", "User", "Storage (bytes)", "Packages"]);
                    for e in &entries {
                        table.add_row([
                            &e.registry,
                            &e.user_id,
                            &e.storage_bytes.to_string(),
                            &e.package_count.to_string(),
                        ]);
                    }
                    println!("{table}");
                }
            }
            QuotaCommand::Reset { registry, user } => {
                client.reset_quota(&registry, &user).await?;
                println!("Reset quota for {user} in {registry}");
            }
        },

        AdminCommand::IpBlock { cmd } => match cmd {
            IpBlockCommand::List => {
                let blocks = client.list_ip_blocks().await?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&blocks)?);
                } else {
                    let mut table = Table::new();
                    table.set_header(["IP", "Reason", "Blocked At"]);
                    for b in &blocks {
                        table.add_row([b.ip.as_str(), b.reason.as_str(), &b.blocked_at.to_string()]);
                    }
                    println!("{table}");
                    println!("{} block(s)", blocks.len());
                }
            }
            IpBlockCommand::Add { ip, reason } => {
                client.add_ip_block(&ip, reason.as_deref()).await?;
                println!("Blocked {ip}");
            }
            IpBlockCommand::Remove { ip } => {
                client.remove_ip_block(&ip).await?;
                println!("Unblocked {ip}");
            }
        },

        AdminCommand::Config { cmd } => match cmd {
            ConfigAdminCommand::Reload => {
                client.config_reload().await?;
                println!("Config reload triggered.");
            }
            ConfigAdminCommand::Changes => {
                let changes = client.config_changes().await?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&changes)?);
                } else {
                    let mut table = Table::new();
                    table.set_header(["Applied At", "Status", "Triggered By", "Summary"]);
                    for c in &changes {
                        table.add_row([
                            c.applied_at.as_deref().unwrap_or("-"),
                            c.status.as_deref().unwrap_or("-"),
                            c.triggered_by.as_deref().unwrap_or("-"),
                            c.summary.as_deref().unwrap_or("-"),
                        ]);
                    }
                    println!("{table}");
                }
            }
        },

        AdminCommand::Cache { cmd } => match cmd {
            CacheCommand::Warm { registry, packages } => {
                let pkgs = packages
                    .unwrap_or_default()
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect();
                client.cache_warm(&registry, pkgs).await?;
                println!("Cache warming started for {registry}");
            }
            CacheCommand::Clear { registry } => {
                client.cache_clear(&registry).await?;
                println!("Cache cleared for {registry}");
            }
        },

        AdminCommand::Banner { cmd } => match cmd {
            BannerCommand::Set { message, level } => {
                client.set_banner(&message, &level).await?;
                println!("Banner set ({level})");
            }
            BannerCommand::Clear => {
                client.clear_banner().await?;
                println!("Banner cleared");
            }
        },

        AdminCommand::AuditLog {
            registry,
            user,
            from,
            to,
            denied_only,
            page,
            per_page,
        } => {
            let resp = client
                .audit_log(AuditQuery {
                    registry,
                    user_id: user,
                    from,
                    to,
                    denied_only: if denied_only { Some(true) } else { None },
                    page,
                    per_page,
                })
                .await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&resp)?);
            } else {
                let mut table = Table::new();
                table.set_header(["Time", "Registry", "User", "Action", "Package", "Denied"]);
                for e in &resp {
                    let registry = e.package_id.as_ref().map(|p| p.registry.as_str()).unwrap_or("-");
                    let package = e.package_id.as_ref().map(|p| p.name.as_str()).unwrap_or("-");
                    let denied = e.result.as_ref().map(|r| r.outcome == "denied").unwrap_or(false);
                    table.add_row([
                        e.timestamp.as_deref().unwrap_or("-"),
                        registry,
                        e.user_id.as_deref().unwrap_or("(anon)"),
                        e.action.as_deref().unwrap_or("-"),
                        package,
                        if denied { "yes" } else { "no" },
                    ]);
                }
                println!("{table}");
                println!("{} entry/entries", resp.len());
            }
        }
    }
    Ok(())
}
