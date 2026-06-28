use anyhow::Result;
use clap::Subcommand;
use comfy_table::Table;

use crate::api::{
    admin::{AuditEntry, AuditQuery},
    BatleHubClient,
};

fn parse_pkg_version(s: &str) -> anyhow::Result<(String, String)> {
    let (name, version) = s
        .split_once('@')
        .ok_or_else(|| anyhow::anyhow!("expected name@version, got: {s}"))?;
    Ok((name.to_string(), version.to_string()))
}

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
        /// Purge entries older than this ISO-8601 datetime (e.g. 2024-01-01T00:00:00Z)
        #[arg(long, conflicts_with_all = &["registry","user","from","to","denied_only"])]
        purge_before: Option<String>,
    },
    /// Show aggregate server statistics (cache hit rate, bytes served, …)
    Stats,
    /// Show per-registry and backend health status
    Health,
    /// Manage package visibility
    Visibility {
        #[command(subcommand)]
        cmd: VisibilityCommand,
    },
    /// Manage team namespace prefix claims
    Namespace {
        #[command(subcommand)]
        cmd: NamespaceCommand,
    },
    /// Manage blocked users
    Users {
        #[command(subcommand)]
        cmd: UsersCommand,
    },
    /// Show or export software bill of materials
    Sbom {
        #[command(subcommand)]
        cmd: SbomCommand,
    },
    /// Manage notification channels and subscriptions
    Notifications {
        #[command(subcommand)]
        cmd: NotificationsCommand,
    },
    /// Bulk yank / unyank / delete versions across a registry
    Bulk {
        #[command(subcommand)]
        cmd: BulkCommand,
    },
    /// Mark a package version as deprecated
    Deprecate {
        registry: String,
        name: String,
        version: String,
        #[arg(long)]
        message: Option<String>,
    },
    /// Remove deprecation from a package version
    Undeprecate {
        registry: String,
        name: String,
        version: String,
    },
    /// Hide a package version from search / listings (without deleting)
    Unlist {
        registry: String,
        name: String,
        version: String,
    },
    /// Re-list a previously unlisted package version
    Relist {
        registry: String,
        name: String,
        version: String,
    },
    /// Simulate whether an identity would be allowed to access a registry resource
    AccessCheck {
        /// Registry to evaluate the policy against
        #[arg(long, short = 'r')]
        registry: String,
        /// Package name
        #[arg(long, short = 'p')]
        package: String,
        /// Package version
        #[arg(long, short = 'v')]
        version: String,
        /// Resource type to check (e.g. "releases:read", "source:read")
        #[arg(long, default_value = "releases:read")]
        resource: String,
        /// Simulated user id
        #[arg(long)]
        user: Option<String>,
        /// Simulated role: anonymous, user, or admin (default: anonymous)
        #[arg(long)]
        role: Option<String>,
        /// Simulated OIDC groups (comma-separated or repeated flag)
        #[arg(long, value_delimiter = ',')]
        groups: Vec<String>,
    },
    /// Export audit log events for compliance review
    ExportAuditLog {
        /// Start datetime (RFC 3339)
        #[arg(long)]
        from: Option<String>,
        /// End datetime (RFC 3339)
        #[arg(long)]
        to: Option<String>,
        /// Filter by registry
        #[arg(long)]
        registry: Option<String>,
        /// Output format: json (default) or csv
        #[arg(long, default_value = "json")]
        format: String,
        /// Write output to file instead of stdout
        #[arg(long, short = 'o')]
        output: Option<String>,
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
    /// Print the current active server configuration (TOML)
    View,
    /// Validate a local TOML config file against the server
    Validate {
        /// Path to the TOML config file to validate
        file: String,
    },
    /// Apply a local TOML config file as the new pending configuration
    FromFile {
        /// Path to the TOML config file to apply
        file: String,
    },
}

#[derive(Subcommand)]
pub enum CacheCommand {
    /// Pre-warm the cache for a registry
    Warm {
        registry: String,
        /// Comma-separated list of package names to warm
        #[arg(long)]
        packages: Option<String>,
        /// Comma-separated upstream artifact paths to warm, for path-addressed
        /// registries (deb/rpm/jetbrains), e.g. "idea/ideaIC-2024.1.4.tar.gz"
        #[arg(long)]
        paths: Option<String>,
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

#[derive(Subcommand)]
pub enum VisibilityCommand {
    /// Get the visibility of a package
    Get { registry: String, name: String },
    /// Set the visibility of a package (public | internal | team)
    Set {
        registry: String,
        name: String,
        visibility: String,
    },
}

#[derive(Subcommand)]
pub enum NamespaceCommand {
    /// List claimed namespace prefixes for a registry
    List { registry: String },
    /// Claim a namespace prefix for a team group
    Claim {
        registry: String,
        prefix: String,
        group_id: String,
    },
    /// Release a claimed namespace prefix
    Release { registry: String, prefix: String },
}

#[derive(Subcommand)]
pub enum UsersCommand {
    /// List all blocked users
    ListBlocked,
    /// Block a user
    Block {
        user_id: String,
        #[arg(long)]
        reason: Option<String>,
    },
    /// Unblock a user
    Unblock { user_id: String },
}

#[derive(Subcommand)]
pub enum SbomCommand {
    /// Show SBOM for a specific package version
    Get {
        registry: String,
        name: String,
        version: String,
        #[arg(long, default_value = "cyclonedx")]
        format: String,
    },
    /// Export SBOMs for a registry or time range
    Export {
        #[arg(long)]
        registry: Option<String>,
        #[arg(long)]
        from: Option<String>,
        #[arg(long)]
        to: Option<String>,
        #[arg(long, default_value = "cyclonedx")]
        format: String,
        #[arg(long, short = 'o')]
        output: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum NotificationsCommand {
    /// List configured notification channels
    Channels,
    /// List notification subscriptions
    List,
    /// Delete a notification subscription by ID
    Delete { id: String },
}

#[derive(Subcommand)]
pub enum BulkCommand {
    /// Yank multiple versions (format: name@version)
    Yank {
        registry: String,
        #[arg(required = true)]
        packages: Vec<String>,
    },
    /// Unyank multiple versions (format: name@version)
    Unyank {
        registry: String,
        #[arg(required = true)]
        packages: Vec<String>,
    },
    /// Delete multiple versions (format: name@version)
    Delete {
        registry: String,
        #[arg(required = true)]
        packages: Vec<String>,
    },
}

pub async fn run(cmd: AdminCommand, client: &BatleHubClient, json: bool) -> Result<()> {
    match cmd {
        AdminCommand::Quota { cmd } => handle_quota(cmd, client, json).await?,
        AdminCommand::IpBlock { cmd } => handle_ip_block(cmd, client, json).await?,
        AdminCommand::Config { cmd } => handle_config_admin(cmd, client, json).await?,
        AdminCommand::Cache { cmd } => handle_cache(cmd, client).await?,
        AdminCommand::Banner { cmd } => handle_banner(cmd, client).await?,
        AdminCommand::AuditLog {
            registry,
            user,
            from,
            to,
            denied_only,
            page,
            per_page,
            purge_before,
        } => {
            handle_audit_log(
                client,
                json,
                AuditLogArgs {
                    registry,
                    user,
                    from,
                    to,
                    denied_only,
                    page,
                    per_page,
                    purge_before,
                },
            )
            .await?
        }
        AdminCommand::Stats => {
            let resp = client.admin_stats().await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&resp)?);
            } else {
                print_json_value_table("Stats", &resp);
            }
        }
        AdminCommand::Health => {
            let resp = client.registry_health().await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&resp)?);
            } else {
                print_json_value_table("Health", &resp);
            }
        }
        AdminCommand::Visibility { cmd } => handle_visibility(cmd, client, json).await?,
        AdminCommand::Namespace { cmd } => handle_namespace(cmd, client, json).await?,
        AdminCommand::Users { cmd } => handle_users(cmd, client, json).await?,
        AdminCommand::Sbom { cmd } => handle_sbom(cmd, client, json).await?,
        AdminCommand::Notifications { cmd } => handle_notifications(cmd, client, json).await?,
        AdminCommand::Bulk { cmd } => handle_bulk(cmd, client, json).await?,
        AdminCommand::Deprecate {
            registry,
            name,
            version,
            message,
        } => {
            client
                .deprecate_package(&registry, &name, &version, message.as_deref())
                .await?;
            println!("Deprecated {registry}/{name}@{version}");
        }
        AdminCommand::Undeprecate {
            registry,
            name,
            version,
        } => {
            client.undeprecate_package(&registry, &name, &version).await?;
            println!("Undeprecated {registry}/{name}@{version}");
        }
        AdminCommand::Unlist {
            registry,
            name,
            version,
        } => {
            client.unlist_package(&registry, &name, &version).await?;
            println!("Unlisted {registry}/{name}@{version}");
        }
        AdminCommand::Relist {
            registry,
            name,
            version,
        } => {
            client.relist_package(&registry, &name, &version).await?;
            println!("Relisted {registry}/{name}@{version}");
        }
        AdminCommand::AccessCheck {
            registry,
            package,
            version,
            resource,
            user,
            role,
            groups,
        } => {
            let resp = client
                .simulate_access(
                    &registry,
                    &package,
                    &version,
                    &resource,
                    user.as_deref(),
                    role.as_deref(),
                    &groups,
                )
                .await?;
            print_access_check(json, &resp)?;
        }
        AdminCommand::ExportAuditLog {
            from,
            to,
            registry,
            format,
            output,
        } => {
            let text = client
                .export_audit_log(
                    from.as_deref(),
                    to.as_deref(),
                    registry.as_deref(),
                    &format,
                )
                .await?;
            match output {
                Some(path) => {
                    std::fs::write(&path, &text)?;
                    println!("Exported to {path}");
                }
                None => print!("{text}"),
            }
        }
    }
    Ok(())
}

struct AuditLogArgs {
    registry: Option<String>,
    user: Option<String>,
    from: Option<String>,
    to: Option<String>,
    denied_only: bool,
    page: u64,
    per_page: u64,
    purge_before: Option<String>,
}

async fn handle_audit_log(
    client: &BatleHubClient,
    json: bool,
    args: AuditLogArgs,
) -> Result<()> {
    if let Some(before) = args.purge_before {
        let resp = client.purge_audit_log(&before).await?;
        if json {
            println!("{}", serde_json::to_string_pretty(&resp)?);
        } else {
            let deleted = resp.get("deleted").and_then(|v| v.as_u64()).unwrap_or(0);
            println!("Deleted {deleted} audit-log row(s) older than {before}");
        }
        return Ok(());
    }

    let resp = client
        .audit_log(AuditQuery {
            registry: args.registry,
            user_id: args.user,
            from: args.from,
            to: args.to,
            denied_only: if args.denied_only { Some(true) } else { None },
            page: args.page,
            per_page: args.per_page,
        })
        .await?;
    if json {
        println!("{}", serde_json::to_string_pretty(&resp)?);
    } else {
        print_audit_log_table(&resp);
    }
    Ok(())
}

fn print_access_check(json: bool, resp: &serde_json::Value) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(resp)?);
        return Ok(());
    }
    let decision = resp.get("decision").and_then(|v| v.as_str()).unwrap_or("?");
    if decision == "allow" {
        println!("ALLOW");
    } else {
        let reason = resp.get("reason").and_then(|v| v.as_str()).unwrap_or("unknown");
        let rule = resp
            .get("rule_matched")
            .and_then(|v| v.as_str())
            .map(|r| format!("  (rule: {r})"))
            .unwrap_or_default();
        println!("DENY: {reason}{rule}");
    }
    Ok(())
}

async fn handle_quota(cmd: QuotaCommand, client: &BatleHubClient, json: bool) -> Result<()> {
    match cmd {
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
    }
    Ok(())
}

async fn handle_ip_block(cmd: IpBlockCommand, client: &BatleHubClient, json: bool) -> Result<()> {
    match cmd {
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
    }
    Ok(())
}

async fn handle_config_admin(
    cmd: ConfigAdminCommand,
    client: &BatleHubClient,
    json: bool,
) -> Result<()> {
    match cmd {
        ConfigAdminCommand::Reload => {
            client.config_reload().await?;
            println!("Config reload triggered.");
        }
        ConfigAdminCommand::Changes => {
            let changes = client.config_changes().await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&changes)?);
            } else {
                print_config_changes_table(&changes);
            }
        }
        ConfigAdminCommand::View => {
            let resp = client.config_content().await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&resp)?);
            } else {
                let content = resp
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or_else(|| resp.as_str().unwrap_or("(empty)"));
                println!("{content}");
            }
        }
        ConfigAdminCommand::Validate { file } => {
            let content = std::fs::read_to_string(&file)
                .map_err(|e| anyhow::anyhow!("could not read {file}: {e}"))?;
            let resp = client.config_validate(&content).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&resp)?);
            } else {
                print_config_validation(&file, &resp);
            }
        }
        ConfigAdminCommand::FromFile { file } => {
            let content = std::fs::read_to_string(&file)
                .map_err(|e| anyhow::anyhow!("could not read {file}: {e}"))?;
            let resp = client.config_from_content(&content).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&resp)?);
            } else {
                println!("Config applied from {file}");
            }
        }
    }
    Ok(())
}

fn print_config_changes_table(changes: &[crate::api::admin::ConfigChangeEntry]) {
    let mut table = Table::new();
    table.set_header(["Applied At", "Status", "Triggered By", "Summary"]);
    for c in changes {
        table.add_row([
            c.applied_at.as_deref().unwrap_or("-"),
            c.status.as_deref().unwrap_or("-"),
            c.triggered_by.as_deref().unwrap_or("-"),
            c.summary.as_deref().unwrap_or("-"),
        ]);
    }
    println!("{table}");
}

fn print_config_validation(file: &str, resp: &serde_json::Value) {
    let ok = resp.get("valid").and_then(|v| v.as_bool()).unwrap_or(true);
    if ok {
        println!("{file}: valid");
    } else {
        let msg = resp
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("validation failed");
        eprintln!("{file}: {msg}");
        std::process::exit(1);
    }
}

async fn handle_cache(cmd: CacheCommand, client: &BatleHubClient) -> Result<()> {
    match cmd {
        CacheCommand::Warm {
            registry,
            packages,
            paths,
        } => {
            let split = |s: Option<String>| -> Vec<String> {
                s.unwrap_or_default()
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect()
            };
            let pkgs = split(packages);
            let pths = split(paths);
            if pkgs.is_empty() && pths.is_empty() {
                anyhow::bail!("specify --packages and/or --paths");
            }
            client.cache_warm(&registry, pkgs, pths).await?;
            println!("Cache warming started for {registry}");
        }
        CacheCommand::Clear { registry } => {
            client.cache_clear(&registry).await?;
            println!("Cache cleared for {registry}");
        }
    }
    Ok(())
}

async fn handle_banner(cmd: BannerCommand, client: &BatleHubClient) -> Result<()> {
    match cmd {
        BannerCommand::Set { message, level } => {
            client.set_banner(&message, &level).await?;
            println!("Banner set ({level})");
        }
        BannerCommand::Clear => {
            client.clear_banner().await?;
            println!("Banner cleared");
        }
    }
    Ok(())
}

async fn handle_visibility(
    cmd: VisibilityCommand,
    client: &BatleHubClient,
    json: bool,
) -> Result<()> {
    match cmd {
        VisibilityCommand::Get { registry, name } => {
            let resp = client.get_visibility(&registry, &name).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&resp)?);
            } else {
                let vis = resp
                    .get("visibility")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                println!("{registry}/{name}: {vis}");
            }
        }
        VisibilityCommand::Set {
            registry,
            name,
            visibility,
        } => {
            client.set_visibility(&registry, &name, &visibility).await?;
            println!("Set {registry}/{name} visibility to {visibility}");
        }
    }
    Ok(())
}

async fn handle_namespace(
    cmd: NamespaceCommand,
    client: &BatleHubClient,
    json: bool,
) -> Result<()> {
    match cmd {
        NamespaceCommand::List { registry } => {
            let resp = client.list_namespaces(&registry).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&resp)?);
            } else {
                print_json_value_table("Namespaces", &resp);
            }
        }
        NamespaceCommand::Claim {
            registry,
            prefix,
            group_id,
        } => {
            let resp = client.claim_namespace(&registry, &prefix, &group_id).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&resp)?);
            } else {
                println!("Claimed namespace prefix '{prefix}' for group '{group_id}' in {registry}");
            }
        }
        NamespaceCommand::Release { registry, prefix } => {
            client.release_namespace(&registry, &prefix).await?;
            println!("Released namespace prefix '{prefix}' from {registry}");
        }
    }
    Ok(())
}

async fn handle_users(cmd: UsersCommand, client: &BatleHubClient, json: bool) -> Result<()> {
    match cmd {
        UsersCommand::ListBlocked => {
            let resp = client.list_blocked_users().await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&resp)?);
            } else {
                print_json_value_table("Blocked Users", &resp);
            }
        }
        UsersCommand::Block { user_id, reason } => {
            client.block_user(&user_id, reason.as_deref()).await?;
            println!("Blocked user {user_id}");
        }
        UsersCommand::Unblock { user_id } => {
            client.unblock_user(&user_id).await?;
            println!("Unblocked user {user_id}");
        }
    }
    Ok(())
}

async fn handle_sbom(cmd: SbomCommand, client: &BatleHubClient, _json: bool) -> Result<()> {
    match cmd {
        SbomCommand::Get {
            registry,
            name,
            version,
            format,
        } => {
            let resp = client.get_sbom(&registry, &name, &version, &format).await?;
            println!("{}", serde_json::to_string_pretty(&resp)?);
        }
        SbomCommand::Export {
            registry,
            from,
            to,
            format,
            output,
        } => {
            let resp = client
                .export_sbom(
                    registry.as_deref(),
                    from.as_deref(),
                    to.as_deref(),
                    &format,
                )
                .await?;
            let content = serde_json::to_string_pretty(&resp)?;
            if let Some(path) = output {
                std::fs::write(&path, &content)?;
                println!("SBOM exported to {path}");
            } else {
                println!("{content}");
            }
        }
    }
    Ok(())
}

async fn handle_notifications(
    cmd: NotificationsCommand,
    client: &BatleHubClient,
    json: bool,
) -> Result<()> {
    match cmd {
        NotificationsCommand::Channels => {
            let resp = client.list_notification_channels().await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&resp)?);
            } else {
                print_json_value_table("Notification Channels", &resp);
            }
        }
        NotificationsCommand::List => {
            let resp = client.list_notification_subscriptions().await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&resp)?);
            } else {
                print_json_value_table("Subscriptions", &resp);
            }
        }
        NotificationsCommand::Delete { id } => {
            client.delete_notification_subscription(&id).await?;
            println!("Deleted subscription {id}");
        }
    }
    Ok(())
}

async fn handle_bulk(cmd: BulkCommand, client: &BatleHubClient, json: bool) -> Result<()> {
    match cmd {
        BulkCommand::Yank { registry, packages } => {
            let pkgs = packages
                .iter()
                .map(|s| parse_pkg_version(s))
                .collect::<Result<Vec<_>>>()?;
            let resp = client.bulk_yank(&registry, pkgs).await?;
            print_bulk_result(json, &resp)?;
        }
        BulkCommand::Unyank { registry, packages } => {
            let pkgs = packages
                .iter()
                .map(|s| parse_pkg_version(s))
                .collect::<Result<Vec<_>>>()?;
            let resp = client.bulk_unyank(&registry, pkgs).await?;
            print_bulk_result(json, &resp)?;
        }
        BulkCommand::Delete { registry, packages } => {
            let pkgs = packages
                .iter()
                .map(|s| parse_pkg_version(s))
                .collect::<Result<Vec<_>>>()?;
            let resp = client.bulk_delete(&registry, pkgs).await?;
            print_bulk_result(json, &resp)?;
        }
    }
    Ok(())
}

fn print_bulk_result(json: bool, resp: &serde_json::Value) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(resp)?);
    } else {
        let processed = resp.get("processed").and_then(|v| v.as_u64()).unwrap_or(0);
        let succeeded = resp.get("succeeded").and_then(|v| v.as_u64()).unwrap_or(0);
        let failed = resp.get("failed").and_then(|v| v.as_u64()).unwrap_or(0);
        println!("processed={processed} succeeded={succeeded} failed={failed}");
    }
    Ok(())
}

/// Render a JSON scalar as a plain string (without the surrounding quotes a
/// `to_string()` would add to a `String`).
fn json_cell(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn print_json_array_table(label: &str, arr: &[serde_json::Value]) {
    if arr.is_empty() {
        println!("(no {label})");
        return;
    }
    let mut table = Table::new();
    if let Some(serde_json::Value::Object(first)) = arr.first() {
        table.set_header(first.keys().collect::<Vec<_>>());
        for item in arr {
            if let serde_json::Value::Object(obj) = item {
                table.add_row(obj.values().map(json_cell).collect::<Vec<_>>());
            }
        }
    }
    println!("{table}");
}

fn print_json_value_table(label: &str, value: &serde_json::Value) {
    match value {
        serde_json::Value::Array(arr) => print_json_array_table(label, arr),
        serde_json::Value::Object(obj) => {
            let mut table = Table::new();
            table.set_header(["Key", "Value"]);
            for (k, v) in obj {
                table.add_row([k.clone(), json_cell(v)]);
            }
            println!("{table}");
        }
        other => println!("{other}"),
    }
}

fn print_audit_log_table(entries: &[AuditEntry]) {
    let mut table = Table::new();
    table.set_header(["Time", "Registry", "User", "Action", "Package", "Denied"]);
    for e in entries {
        let registry = e
            .package_id
            .as_ref()
            .map(|p| p.registry.as_str())
            .unwrap_or("-");
        let package = e
            .package_id
            .as_ref()
            .map(|p| p.name.as_str())
            .unwrap_or("-");
        let denied = e
            .result
            .as_ref()
            .map(|r| r.outcome == "denied")
            .unwrap_or(false);
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
    println!("{} entry/entries", entries.len());
}
