use std::path::PathBuf;

use anyhow::Result;
use clap::Subcommand;

use crate::api::ide::detect_ides;
use crate::api::registry::{host_of, RegistryInfo, RegistryTargets};
use crate::api::setup::{api_registry_type, scan_project_types};
use crate::api::BatleHubClient;

/// How long to wait for the registry list before falling back to placeholders.
/// `setup` is a "tell me what to paste" command; it must not sit on a server
/// that is down or unreachable from this network.
const REGISTRY_FETCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

#[derive(Subcommand)]
pub enum SetupCommand {
    /// Scan a directory for known project manifests and print setup instructions
    Detect {
        /// Directory to scan (defaults to the current working directory)
        #[arg(long, short = 'd')]
        dir: Option<PathBuf>,

        /// How many subdirectory levels to scan (0 = root only, 1 = immediate subdirs, …)
        #[arg(long, default_value = "0")]
        depth: usize,

        /// Server URL to embed in the generated config snippets
        #[arg(long, env = "BATLEHUB_SERVER", default_value = "http://localhost:8080")]
        server: String,

        /// Do not contact the server; print `<registry>` placeholders instead of
        /// the configured registry names and their host-routed URLs
        #[arg(long)]
        offline: bool,

        /// Output raw JSON instead of human-readable text
        #[arg(long)]
        json: bool,
    },

    /// Detect the current editor/IDE and print how to point its extension or
    /// plugin ecosystem at BatleHub (VS Code / VSCodium → OpenVSX or VS Code
    /// Marketplace; JetBrains → JetBrains Marketplace)
    Ide {
        /// Server URL to embed in the generated config snippets
        #[arg(long, env = "BATLEHUB_SERVER", default_value = "http://localhost:8080")]
        server: String,

        /// Do not contact the server; print `<…>` registry placeholders
        #[arg(long)]
        offline: bool,

        /// Output raw JSON instead of human-readable text
        #[arg(long)]
        json: bool,
    },
}

/// Best-effort registry list.
///
/// The instructions are still useful without it — that is the shape this
/// command shipped with — so a server that is down, unauthenticated or simply
/// not configured yet degrades to `<registry>` placeholders rather than to an
/// error. Only a *reachable* server can tell us that a registry lives on its own
/// host, and that is the difference between a snippet that works and one that
/// 404s, so it is worth asking for.
async fn load_registries(server: &str, token: Option<&str>, offline: bool) -> Vec<RegistryInfo> {
    if offline {
        return Vec::new();
    }
    let Ok(client) = BatleHubClient::new(server, token) else {
        return Vec::new();
    };
    match tokio::time::timeout(REGISTRY_FETCH_TIMEOUT, client.list_registries()).await {
        Ok(Ok(registries)) => registries,
        // Both arms are the same fallback; kept apart so the reason can be
        // reported, since "no registries" and "could not ask" look identical in
        // the output otherwise.
        Ok(Err(e)) => {
            eprintln!("Note: could not list registries on {server} ({e}); using placeholders.");
            Vec::new()
        }
        Err(_) => {
            eprintln!("Note: {server} did not answer within 5s; using placeholders.");
            Vec::new()
        }
    }
}

/// The `~/.netrc` block for `hosts`, or nothing when there is no host to name.
///
/// One stanza per host: `.netrc` is matched by hostname, so a host-routed
/// registry needs its own entry — credentials for the main host are not sent to
/// `npm1.batlehub.example.com`, and the install would 401.
///
/// The token is a placeholder even when one is configured: this prints to a
/// terminal (and often into a paste), and the value the reader must fill in is
/// theirs to choose — `batlehub-cli auth token` prints it when they want it.
fn netrc_block(hosts: &[String]) -> Option<String> {
    if hosts.is_empty() {
        return None;
    }
    let mut out = String::from(
        "Credentials — ~/.netrc (one stanza per host; chmod 600 ~/.netrc):\n\
         \n",
    );
    for host in hosts {
        out.push_str(&format!(
            "machine {host}\nlogin <user>\npassword <token>\n\n"
        ));
    }
    out.push_str("Get a token with: batlehub-cli auth login\n");
    Some(out)
}

pub async fn run(
    cmd: SetupCommand,
    global_server: Option<&str>,
    token: Option<&str>,
) -> Result<()> {
    match cmd {
        SetupCommand::Detect {
            dir,
            depth,
            server,
            offline,
            json,
        } => {
            let dir = match dir {
                Some(d) => d,
                None => std::env::current_dir()?,
            };

            let effective_server = global_server.unwrap_or(&server);
            let registries = load_registries(effective_server, token, offline).await;
            let targets = RegistryTargets::new(effective_server, &registries);
            let detections = scan_project_types(&dir, &targets, depth);

            if json {
                let out: Vec<serde_json::Value> = detections
                    .iter()
                    .map(|d| {
                        serde_json::json!({
                            "registry_type": d.registry_type,
                            "package_name": d.package_name,
                            "relative_path": d.relative_path,
                            "registry_name": d.registry_name,
                            "base_url": d.base_url,
                        })
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&out)?);
            } else if detections.is_empty() {
                println!("No known project manifests found in: {}", dir.display());
                println!(
                    "Supported: Cargo.toml, go.mod, package.json, pyproject.toml, \
                     pom.xml, composer.json, *.gemspec, *.nuspec, *.csproj, *.tf, environment.yml"
                );
            } else {
                for det in &detections {
                    let name = det.package_name.as_deref().unwrap_or("<unknown>");
                    if det.relative_path.is_empty() {
                        println!("Detected: {} ({})", det.registry_type, name);
                    } else {
                        println!(
                            "Detected: {} ({}) [{}]",
                            det.registry_type, name, det.relative_path
                        );
                    }
                    println!();
                    println!("{}", det.instructions);
                    println!();
                    println!("{}", "─".repeat(60));
                    println!();
                }

                // Only the types actually detected: a `.netrc` that names hosts
                // this project never talks to is noise the reader has to audit.
                let types: Vec<&str> = detections
                    .iter()
                    .map(|d| api_registry_type(d.registry_type))
                    .collect();
                if let Some(block) = netrc_block(&targets.netrc_hosts(&types)) {
                    println!("{block}");
                }
            }
        }
        SetupCommand::Ide {
            server,
            offline,
            json,
        } => {
            let effective_server = global_server.unwrap_or(&server);
            let registries = load_registries(effective_server, token, offline).await;
            let setups = detect_ides(effective_server, &registries);

            if json {
                let out: Vec<serde_json::Value> = setups
                    .iter()
                    .map(|s| {
                        serde_json::json!({
                            "ide": s.kind.label(),
                            "registry_type": s.registry_type,
                            "registry_name": s.registry_name,
                            "registry_configured": s.registry_configured,
                            "detected_via": s.detected_via,
                            "base_url": s.base_url,
                        })
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&out)?);
            } else if setups.is_empty() {
                println!("No IDE detected in this environment.");
                println!(
                    "Detection uses $TERM_PROGRAM / VSCODE_* / JetBrains terminal variables, \
                     the ~/.config/{{Code,VSCodium,JetBrains}} directories, and a ./.idea folder. \
                     Run this from your editor's integrated terminal."
                );
            } else {
                for s in &setups {
                    println!("Detected: {} (via {})", s.kind.label(), s.detected_via);
                    println!();
                    println!("{}", s.instructions);
                    println!();
                    println!("{}", "─".repeat(60));
                    println!();
                }

                let mut hosts: Vec<String> = Vec::new();
                for setup in &setups {
                    let host = host_of(&setup.base_url);
                    if !hosts.contains(&host) {
                        hosts.push(host);
                    }
                }
                if let Some(block) = netrc_block(&hosts) {
                    println!("{block}");
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn netrc_block_is_empty_without_hosts() {
        assert!(netrc_block(&[]).is_none());
    }

    #[test]
    fn netrc_block_writes_one_stanza_per_host() {
        let block = netrc_block(&[
            "npm1.batlehub.example.com".to_owned(),
            "batlehub.example.com".to_owned(),
        ])
        .unwrap();
        assert_eq!(block.matches("machine ").count(), 2);
        assert!(block.contains("machine npm1.batlehub.example.com"));
        assert!(block.contains("machine batlehub.example.com"));
        // Never a real credential: this lands in terminal scrollback.
        assert!(block.contains("password <token>"));
    }

    /// `--offline` must not build a client at all — the fallback is the whole
    /// point of the flag, and it has to hold for an unroutable server.
    #[tokio::test]
    async fn load_registries_offline_skips_the_server() {
        assert!(load_registries("http://127.0.0.1:1", None, true)
            .await
            .is_empty());
    }

    #[tokio::test]
    async fn load_registries_falls_back_when_the_server_is_unreachable() {
        assert!(load_registries("http://127.0.0.1:1", None, false)
            .await
            .is_empty());
    }
}
