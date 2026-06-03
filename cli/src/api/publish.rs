use std::path::Path;

use anyhow::Result;
use serde_json::Value;

use super::BatleHubClient;

/// Metadata extracted from (or provided for) an artifact.
#[derive(Debug, Clone)]
pub struct ArtifactMeta {
    pub registry_type: String,
    pub name: String,
    pub version: String,
}

/// Attempt to detect registry type and extract name/version from an artifact file.
pub fn detect_meta(path: &Path) -> Option<ArtifactMeta> {
    let ext = path.extension()?.to_str()?;
    let file_name = path.file_name()?.to_str()?;

    match ext {
        "nupkg" => {
            // filename: <name>.<version>.nupkg
            let stem = file_name.strip_suffix(".nupkg")?;
            // find the first segment that looks like a semver part
            let dot_pos = stem.find(|c: char| c.is_ascii_digit())?;
            if dot_pos == 0 {
                return None;
            }
            let name = &stem[..dot_pos - 1];
            let version = &stem[dot_pos..];
            Some(ArtifactMeta {
                registry_type: "nuget".into(),
                name: name.to_string(),
                version: version.to_string(),
            })
        }
        "whl" => {
            // filename: <name>-<version>-*.whl
            let parts: Vec<&str> = file_name.split('-').collect();
            if parts.len() >= 2 {
                return Some(ArtifactMeta {
                    registry_type: "pypi".into(),
                    name: parts[0].replace('_', "-"),
                    version: parts[1].to_string(),
                });
            }
            None
        }
        "gem" => {
            // filename: <name>-<version>.gem
            let stem = file_name.strip_suffix(".gem")?;
            let dash = stem.rfind('-')?;
            Some(ArtifactMeta {
                registry_type: "rubygems".into(),
                name: stem[..dash].to_string(),
                version: stem[dash + 1..].to_string(),
            })
        }
        _ => None,
    }
}

impl BatleHubClient {
    /// Upload a `.nupkg` artifact to a NuGet local/hybrid registry.
    pub async fn publish_nuget(&self, registry: &str, file_path: &Path) -> Result<Value> {
        let bytes = std::fs::read(file_path)?;
        let file_name = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("package.nupkg")
            .to_string();

        let part = reqwest::multipart::Part::bytes(bytes)
            .file_name(file_name)
            .mime_str("application/octet-stream")?;
        let form = reqwest::multipart::Form::new().part("package", part);

        self.post_multipart(&format!("/api/v1/local/{registry}/nuget/v2/package"), form)
            .await
    }
}
