use std::io::{self, Write};

use anyhow::Result;
use clap::Subcommand;
use comfy_table::Table;

use crate::api::{
    auth::{parse_oidc_paste, CreateTokenRequest},
    BatleHubClient,
};
use crate::config::ConfigFile;

#[derive(Subcommand)]
pub enum AuthCommand {
    /// Show the current identity
    Whoami,
    /// Token management
    Token {
        #[command(subcommand)]
        cmd: TokenCommand,
    },
    /// Log in via OIDC (browser) or Kubernetes service account; saves token to config
    Login {
        /// OIDC provider name (defaults to the first configured provider)
        #[arg(long)]
        provider: Option<String>,
        /// Path to a Kubernetes service account token file to use instead of OIDC
        #[arg(long)]
        kubernetes_token_path: Option<String>,
        /// Config profile to save credentials into (defaults to 'default')
        #[arg(long)]
        profile: Option<String>,
    },
    /// Manually refresh a cached OIDC access token using the stored refresh token
    Refresh {
        /// OIDC provider name (defaults to the first configured provider)
        #[arg(long)]
        provider: Option<String>,
        /// Config profile whose refresh token to use (defaults to 'default')
        #[arg(long)]
        profile: Option<String>,
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

fn mask_token(t: &str) -> String {
    if t.len() <= 8 {
        return "****".to_string();
    }
    format!("{}…{}", &t[..4], &t[t.len() - 4..])
}

pub async fn run(
    cmd: AuthCommand,
    client: &BatleHubClient,
    json: bool,
    global_profile: Option<&str>,
) -> Result<()> {
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
        AuthCommand::Token { cmd } => handle_token_command(cmd, client, json).await?,

        AuthCommand::Login {
            provider,
            kubernetes_token_path,
            profile,
        } => {
            handle_auth_login(
                client,
                provider,
                kubernetes_token_path,
                profile,
                global_profile,
            )
            .await?
        }

        AuthCommand::Refresh { provider, profile } => {
            handle_auth_refresh(client, provider, profile, global_profile).await?
        }
    }
    Ok(())
}

async fn handle_token_command(
    cmd: TokenCommand,
    client: &BatleHubClient,
    json: bool,
) -> Result<()> {
    match cmd {
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
    }
    Ok(())
}

async fn handle_auth_login(
    client: &BatleHubClient,
    provider: Option<String>,
    kubernetes_token_path: Option<String>,
    profile: Option<String>,
    global_profile: Option<&str>,
) -> Result<()> {
    let target_profile = profile.as_deref().or(global_profile);
    let mut cfg = ConfigFile::load()?;

    if let Some(k8s_path) = kubernetes_token_path {
        let entry = match target_profile {
            Some(n) => cfg.profiles.entry(n.to_string()).or_default(),
            None => &mut cfg.default,
        };
        entry.kubernetes_token_path = Some(k8s_path.clone());
        entry.token = None;
        entry.oidc_refresh_token = None;
        entry.oidc_expires_at = None;
        cfg.save()?;
        println!("Kubernetes token path saved: {k8s_path}");
        println!("The token will be read fresh from this path on each request.");
        return Ok(());
    }

    let providers = client.list_oidc_providers().await.unwrap_or_default();
    if providers.is_empty() {
        anyhow::bail!(
            "OIDC is not configured on this server. \
            Use `auth token create` for static tokens, or \
            `auth login --kubernetes-token-path <path>` for Kubernetes."
        );
    }

    let csrf = uuid::Uuid::new_v4().to_string();
    let login_url = client.oidc_login_url(&csrf, provider.as_deref()).await?;

    println!("Open this URL in your browser:");
    println!();
    println!("  {login_url}");
    println!();
    println!("After login you will land on a URL containing oidc_access_token=…");
    println!("Paste the full URL (or just the token value):");
    print!("> ");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let input = input.trim();
    if input.is_empty() {
        anyhow::bail!("No input provided — login cancelled.");
    }

    let (access_token, refresh_token, expires_at) = parse_oidc_paste(input);

    let entry = match target_profile {
        Some(n) => cfg.profiles.entry(n.to_string()).or_default(),
        None => &mut cfg.default,
    };
    entry.token = Some(access_token.clone());
    entry.oidc_refresh_token = refresh_token;
    entry.oidc_expires_at = expires_at;
    entry.kubernetes_token_path = None;
    cfg.save()?;

    println!(
        "Logged in. Token saved to profile '{}'.",
        target_profile.unwrap_or("default")
    );
    println!("  {}", mask_token(&access_token));
    Ok(())
}

async fn handle_auth_refresh(
    client: &BatleHubClient,
    provider: Option<String>,
    profile: Option<String>,
    global_profile: Option<&str>,
) -> Result<()> {
    let target_profile = profile.as_deref().or(global_profile);
    let mut cfg = ConfigFile::load()?;

    let refresh_token = {
        let entry = match target_profile {
            Some(n) => cfg.profiles.get(n),
            None => Some(&cfg.default),
        };
        entry
            .and_then(|p| p.oidc_refresh_token.clone())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "No OIDC refresh token stored for profile '{}'. \
                    Run `auth login` first.",
                    target_profile.unwrap_or("default")
                )
            })?
    };

    let (access_token, new_refresh, expires_in) = client
        .oidc_refresh(&refresh_token, provider.as_deref())
        .await?;

    let entry = match target_profile {
        Some(n) => cfg.profiles.entry(n.to_string()).or_default(),
        None => &mut cfg.default,
    };
    entry.token = Some(access_token);
    if let Some(rt) = new_refresh {
        entry.oidc_refresh_token = Some(rt);
    }
    if let Some(exp) = expires_in {
        entry.oidc_expires_at = Some(chrono::Utc::now().timestamp() + exp as i64);
    }
    cfg.save()?;
    println!("Token refreshed successfully.");
    Ok(())
}
