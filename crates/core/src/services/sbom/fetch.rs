use std::collections::HashSet;

use chrono::{DateTime, Utc};

use crate::entities::ArtifactSbom;
use crate::error::CoreError;
use crate::ports::SbomRepository;

// ── Paged collection ──────────────────────────────────────────────────────────

pub(super) async fn collect_all_sbom_pages(
    repo: &dyn SbomRepository,
    registry: Option<&str>,
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
) -> Result<Vec<ArtifactSbom>, CoreError> {
    let page_size: u64 = 100;
    let mut offset: u64 = 0;
    let mut all = Vec::new();
    loop {
        let page = repo
            .list_sboms_for_export(registry, from, to, page_size, offset)
            .await?;
        let done = page.len() < page_size as usize;
        all.extend(page);
        offset += page_size;
        if done {
            break;
        }
    }
    Ok(all)
}

// ── SPDX merge helpers ────────────────────────────────────────────────────────

pub(super) fn collect_spdx_entries(
    sboms: &[ArtifactSbom],
) -> (Vec<serde_json::Value>, Vec<serde_json::Value>) {
    let mut packages: Vec<serde_json::Value> = Vec::new();
    let mut relationships: Vec<serde_json::Value> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for sbom in sboms {
        if let Some(pkgs) = sbom.document.get("packages").and_then(|v| v.as_array()) {
            for pkg in pkgs {
                let key = format!(
                    "{}@{}",
                    pkg.get("name").and_then(|v| v.as_str()).unwrap_or(""),
                    pkg.get("versionInfo")
                        .and_then(|v| v.as_str())
                        .unwrap_or(""),
                );
                if seen.insert(key) {
                    packages.push(pkg.clone());
                }
            }
        }
        if let Some(rels) = sbom
            .document
            .get("relationships")
            .and_then(|v| v.as_array())
        {
            relationships.extend_from_slice(rels);
        }
    }
    (packages, relationships)
}

// ── CycloneDX merge helpers ───────────────────────────────────────────────────

pub(super) fn collect_cyclonedx_components(sboms: &[ArtifactSbom]) -> Vec<serde_json::Value> {
    let mut components: Vec<serde_json::Value> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for sbom in sboms {
        if let Some(comps) = sbom.document.get("components").and_then(|v| v.as_array()) {
            for comp in comps {
                let key = format!(
                    "{}@{}",
                    comp.get("name").and_then(|v| v.as_str()).unwrap_or(""),
                    comp.get("version").and_then(|v| v.as_str()).unwrap_or(""),
                );
                if seen.insert(key) {
                    components.push(comp.clone());
                }
            }
        }
    }
    components
}
