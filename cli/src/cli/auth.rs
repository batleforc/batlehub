use anyhow::Result;
use clap::Subcommand;
use comfy_table::Table;

use crate::api::{auth::CreateTokenRequest, BatleHubClient};

#[derive(Subcommand)]
pub enum AuthCommand {
    /// Show the current identity
    Whoami,
    /// Token management
    Token {
        #[command(subcommand)]
        cmd: TokenCommand,
    },
}

#[derive(Subcommand)]
pub enum TokenCommand {
    /// List your active API tokens
    List,
    /// Create a new API token (requires OIDC session)
    Create {
        /// Display name for the token
        #[arg(long, short = 'n')]
        name: String,
        /// Lifetime in days (1–90)
        #[arg(long, short = 'd', default_value = "30")]
        days: u64,
        /// Role: user or admin
        #[arg(long, default_value = "user")]
        role: String,
    },
    /// Revoke a token by its UUID
    Revoke {
        /// Token UUID
        id: uuid::Uuid,
    },
}

pub async fn run(cmd: AuthCommand, client: &BatleHubClient, json: bool) -> Result<()> {
    match cmd {
        AuthCommand::Whoami => {
            let me = client.whoami().await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&me)?);
            } else {
                let mut table = Table::new();
                table.add_row(["User ID", me.user_id.as_deref().unwrap_or("(anonymous)")]);
                table.add_row(["Role", &me.role]);
                table.add_row(["Provider", me.auth_provider.as_deref().unwrap_or("-")]);
                if !me.groups.is_empty() {
                    table.add_row(["Groups", &me.groups.join(", ")]);
                }
                println!("{table}");
            }
        }
        AuthCommand::Token { cmd } => match cmd {
            TokenCommand::List => {
                let tokens = client.list_tokens().await?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&tokens)?);
                } else {
                    let mut table = Table::new();
                    table.set_header(["ID", "Name", "Role", "Expires"]);
                    for t in &tokens {
                        table.add_row([
                            &t.id.to_string(),
                            &t.name,
                            &t.role,
                            &t.expires_at.format("%Y-%m-%d").to_string(),
                        ]);
                    }
                    println!("{table}");
                    println!("{} token(s)", tokens.len());
                }
            }
            TokenCommand::Create { name, days, role } => {
                let resp = client
                    .create_token(CreateTokenRequest {
                        name: name.clone(),
                        expires_in_days: days,
                        role: role.clone(),
                    })
                    .await?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&resp)?);
                } else {
                    println!(
                        "Created token '{name}' (role: {role}, expires: {})",
                        resp.expires_at.format("%Y-%m-%d")
                    );
                    println!();
                    println!("Token (store this — it will not be shown again):");
                    println!("  {}", resp.token);
                }
            }
            TokenCommand::Revoke { id } => {
                client.revoke_token(id).await?;
                println!("Revoked token {id}");
            }
        },
    }
    Ok(())
}
