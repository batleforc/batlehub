use std::path::PathBuf;

use anyhow::{bail, Result};
use clap::Args;

use crate::api::{publish::detect_meta, BatleHubClient};

#[derive(Args)]
pub struct PublishArgs {
    /// Path to the artifact file
    pub file: PathBuf,

    /// Target registry name (overrides default)
    #[arg(long, short = 'r')]
    pub registry: Option<String>,

    /// Package name (overrides auto-detection)
    #[arg(long, short = 'n')]
    pub name: Option<String>,

    /// Package version (overrides auto-detection)
    #[arg(long, short = 'v')]
    pub version: Option<String>,
}

pub async fn run(
    args: PublishArgs,
    client: &BatleHubClient,
    default_registry: Option<&str>,
) -> Result<()> {
    if !args.file.exists() {
        bail!("file not found: {}", args.file.display());
    }

    // Auto-detect or use provided metadata
    let meta = detect_meta(&args.file);

    let name = args
        .name
        .or_else(|| meta.as_ref().map(|m| m.name.clone()))
        .ok_or_else(|| anyhow::anyhow!("could not detect package name; use --name"))?;

    let version = args
        .version
        .or_else(|| meta.as_ref().map(|m| m.version.clone()))
        .ok_or_else(|| anyhow::anyhow!("could not detect package version; use --version"))?;

    let registry_type = meta
        .as_ref()
        .map(|m| m.registry_type.as_str())
        .unwrap_or("?");

    let registry = args
        .registry
        .or_else(|| default_registry.map(str::to_string))
        .ok_or_else(|| {
            anyhow::anyhow!("no registry specified; use --registry or set a default in config")
        })?;

    println!(
        "Publishing {} {}@{} to registry '{}' ...",
        registry_type, name, version, registry
    );

    // Dispatch by detected (or inferred) registry type
    match registry_type {
        "nuget" => {
            client.publish_nuget(&registry, &args.file).await?;
        }
        "pacman" => {
            client.publish_pacman(&registry, &args.file).await?;
        }
        _ => {
            bail!(
                "automatic publish is not yet supported for registry type '{registry_type}'. \
                 Use the native tooling (dotnet nuget push, cargo publish, twine upload, …) \
                 configured to point at this BatleHub registry."
            );
        }
    }

    println!("Published successfully.");
    Ok(())
}
