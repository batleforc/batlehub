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

    /// Registry type, overriding auto-detection from the file extension.
    /// Required for formats auto-detection can't disambiguate (e.g. Composer
    /// ZIPs, which share the `.zip` extension with other formats).
    #[arg(long, short = 't')]
    pub r#type: Option<String>,

    /// Debian: target suite/distribution (e.g. "stable"). Required when publishing a `.deb`.
    #[arg(long)]
    pub distribution: Option<String>,

    /// Debian: target component (e.g. "main"). Required when publishing a `.deb`.
    #[arg(long)]
    pub component: Option<String>,

    /// Conda: target platform/subdir (e.g. "noarch", "linux-64"). Only used as a
    /// fallback when the package itself carries no `subdir` metadata.
    #[arg(long, default_value = "noarch")]
    pub platform: String,
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

    let registry_type = args
        .r#type
        .clone()
        .or_else(|| meta.as_ref().map(|m| m.registry_type.clone()))
        .unwrap_or_else(|| "?".to_string());

    // Composer publishes are parsed server-side from composer.json inside the
    // ZIP, so no client-side name/version detection is needed or attempted.
    let (name, version) = if registry_type == "composer" {
        (String::new(), args.version.clone().unwrap_or_default())
    } else {
        let name = args
            .name
            .clone()
            .or_else(|| meta.as_ref().map(|m| m.name.clone()))
            .ok_or_else(|| anyhow::anyhow!("could not detect package name; use --name"))?;
        let version = args
            .version
            .clone()
            .or_else(|| meta.as_ref().map(|m| m.version.clone()))
            .ok_or_else(|| anyhow::anyhow!("could not detect package version; use --version"))?;
        (name, version)
    };

    let registry = args
        .registry
        .clone()
        .or_else(|| default_registry.map(str::to_string))
        .ok_or_else(|| {
            anyhow::anyhow!("no registry specified; use --registry or set a default in config")
        })?;

    println!(
        "Publishing {} {}@{} to registry '{}' ...",
        registry_type, name, version, registry
    );

    // Dispatch by detected (or inferred) registry type
    match registry_type.as_str() {
        "nuget" => {
            client.publish_nuget(&registry, &args.file).await?;
        }
        "pacman" => {
            client.publish_pacman(&registry, &args.file).await?;
        }
        "pypi" => {
            client
                .publish_pypi(&registry, &name, &version, &args.file)
                .await?;
        }
        "rubygems" => {
            client.publish_rubygems(&registry, &args.file).await?;
        }
        "npm" => {
            client
                .publish_npm(&registry, &name, &version, &args.file)
                .await?;
        }
        "cargo" => {
            client
                .publish_cargo(&registry, &name, &version, &args.file)
                .await?;
        }
        "composer" => {
            client
                .publish_composer(&registry, &args.file, args.version.as_deref())
                .await?;
        }
        "conda" => {
            client
                .publish_conda(&registry, &args.platform, &args.file)
                .await?;
        }
        "openvsx" => {
            client
                .publish_openvsx(&registry, &name, &version, &args.file)
                .await?;
        }
        "deb" => {
            let distribution = args.distribution.as_deref().ok_or_else(|| {
                anyhow::anyhow!("publishing a .deb requires --distribution (e.g. 'stable')")
            })?;
            let component = args.component.as_deref().ok_or_else(|| {
                anyhow::anyhow!("publishing a .deb requires --component (e.g. 'main')")
            })?;
            client
                .publish_deb(&registry, distribution, component, &args.file)
                .await?;
        }
        "rpm" => {
            client.publish_rpm(&registry, &args.file).await?;
        }
        _ => {
            bail!(
                "automatic publish is not yet supported for registry type '{registry_type}'. \
                 Maven (multi-file: jar+pom+checksums), Terraform (providers need \
                 shasums/signature files, modules need packaging), and Go (needs an \
                 .info/.mod/.zip triad) don't fit this command's single-file model \
                 by design — use the native tooling (mvn deploy, terraform-registry \
                 conventions, go mod) configured to point at this BatleHub registry."
            );
        }
    }

    println!("Published successfully.");
    Ok(())
}
