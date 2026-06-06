use std::path::PathBuf;

use anyhow::Result;
use clap::Subcommand;

use crate::api::setup::scan_project_types;

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

        /// Output raw JSON instead of human-readable text
        #[arg(long)]
        json: bool,
    },
}

pub fn run(cmd: SetupCommand, global_server: Option<&str>) -> Result<()> {
    match cmd {
        SetupCommand::Detect { dir, depth, server, json } => {
            let dir = match dir {
                Some(d) => d,
                None => std::env::current_dir()?,
            };

            let effective_server = global_server.unwrap_or(&server);
            let detections = scan_project_types(&dir, effective_server, depth);

            if json {
                let out: Vec<serde_json::Value> = detections
                    .iter()
                    .map(|d| {
                        serde_json::json!({
                            "registry_type": d.registry_type,
                            "package_name": d.package_name,
                            "relative_path": d.relative_path,
                        })
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&out)?);
            } else if detections.is_empty() {
                println!(
                    "No known project manifests found in: {}",
                    dir.display()
                );
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
            }
        }
    }
    Ok(())
}
