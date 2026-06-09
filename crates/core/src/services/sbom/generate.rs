use chrono::Utc;
use uuid::Uuid;

use crate::entities::PackageMetadata;
use crate::ports::SbomDependency;

// ── PURL helpers ──────────────────────────────────────────────────────────────

pub(super) fn registry_to_purl(registry_type: &str, name: &str, version: &str) -> String {
    match registry_type {
        "cargo" => format!("pkg:cargo/{name}@{version}"),
        "npm" => format!("pkg:npm/{name}@{version}"),
        "maven" => format!("pkg:maven/{name}@{version}"),
        "pypi" => format!("pkg:pypi/{name}@{version}"),
        "rubygems" => format!("pkg:gem/{name}@{version}"),
        "goproxy" => format!("pkg:golang/{name}@{version}"),
        "composer" => format!("pkg:composer/{name}@{version}"),
        "conda" => format!("pkg:conda/{name}@{version}"),
        _ => format!("pkg:generic/{name}@{version}"),
    }
}

// ── SPDX 2.3 JSON generation ──────────────────────────────────────────────────

pub(super) fn generate_spdx(
    meta: &PackageMetadata,
    artifact_key: &str,
    deps: &[SbomDependency],
) -> serde_json::Value {
    let doc_ns = format!(
        "https://batlehub/sbom/{}/{}/{}/{}",
        meta.id.registry,
        meta.id.name,
        meta.id.version,
        Uuid::new_v4()
    );

    let download_location = meta
        .download_url
        .clone()
        .unwrap_or_else(|| "NOASSERTION".to_owned());

    let mut checksums = serde_json::json!([]);
    if let Some(ref ck) = meta.checksum {
        checksums = serde_json::json!([{"algorithm": "SHA256", "checksumValue": ck}]);
    }

    let mut packages = vec![serde_json::json!({
        "SPDXID": "SPDXRef-Package",
        "name": meta.id.name,
        "versionInfo": meta.id.version,
        "downloadLocation": download_location,
        "filesAnalyzed": false,
        "checksums": checksums,
        "supplier": "NOASSERTION",
        "comment": artifact_key,
    })];

    let mut relationships = vec![serde_json::json!({
        "spdxElementId": "SPDXRef-DOCUMENT",
        "relationshipType": "DESCRIBES",
        "relatedSpdxElement": "SPDXRef-Package",
    })];

    for (i, dep) in deps.iter().enumerate() {
        let dep_id = format!("SPDXRef-Dep-{i}");
        packages.push(serde_json::json!({
            "SPDXID": dep_id,
            "name": dep.name,
            "versionInfo": dep.version_req.as_deref().unwrap_or("NOASSERTION"),
            "downloadLocation": "NOASSERTION",
            "filesAnalyzed": false,
        }));
        relationships.push(serde_json::json!({
            "spdxElementId": format!("SPDXRef-Dep-{i}"),
            "relationshipType": "DEPENDENCY_OF",
            "relatedSpdxElement": "SPDXRef-Package",
        }));
    }

    serde_json::json!({
        "spdxVersion": "SPDX-2.3",
        "dataLicense": "CC0-1.0",
        "SPDXID": "SPDXRef-DOCUMENT",
        "name": format!("{}-{}", meta.id.name, meta.id.version),
        "documentNamespace": doc_ns,
        "packages": packages,
        "relationships": relationships,
    })
}

// ── CycloneDX 1.4 JSON generation ─────────────────────────────────────────────

pub(super) fn generate_cyclonedx(
    meta: &PackageMetadata,
    artifact_key: &str,
    deps: &[SbomDependency],
) -> serde_json::Value {
    let purl = registry_to_purl(&meta.id.registry, &meta.id.name, &meta.id.version);

    let mut hashes = serde_json::json!([]);
    if let Some(ref ck) = meta.checksum {
        hashes = serde_json::json!([{"alg": "SHA-256", "content": ck}]);
    }

    let main_component = serde_json::json!({
        "type": "library",
        "name": meta.id.name,
        "version": meta.id.version,
        "purl": purl,
        "hashes": hashes,
        "comment": artifact_key,
    });

    let dep_components: Vec<_> = deps
        .iter()
        .map(|d| {
            let dep_purl = registry_to_purl(
                &meta.id.registry,
                &d.name,
                d.version_req.as_deref().unwrap_or("*"),
            );
            serde_json::json!({
                "type": "library",
                "name": d.name,
                "version": d.version_req.as_deref().unwrap_or(""),
                "purl": dep_purl,
            })
        })
        .collect();

    let mut components = vec![main_component];
    components.extend(dep_components);

    serde_json::json!({
        "bomFormat": "CycloneDX",
        "specVersion": "1.4",
        "version": 1,
        "serialNumber": format!("urn:uuid:{}", Uuid::new_v4()),
        "metadata": {
            "timestamp": Utc::now().to_rfc3339(),
            "component": {
                "type": "library",
                "name": meta.id.name,
                "version": meta.id.version,
            }
        },
        "components": components,
    })
}

// ── Export document builders ──────────────────────────────────────────────────

pub(super) fn build_spdx_document(
    packages: Vec<serde_json::Value>,
    relationships: Vec<serde_json::Value>,
) -> serde_json::Value {
    serde_json::json!({
        "spdxVersion": "SPDX-2.3",
        "dataLicense": "CC0-1.0",
        "SPDXID": "SPDXRef-DOCUMENT",
        "name": format!("batlehub-org-export-{}", Utc::now().format("%Y%m%d")),
        "documentNamespace": format!("https://batlehub/sbom/export/{}", Uuid::new_v4()),
        "packages": packages,
        "relationships": relationships,
    })
}

pub(super) fn build_cyclonedx_document(components: Vec<serde_json::Value>) -> serde_json::Value {
    serde_json::json!({
        "bomFormat": "CycloneDX",
        "specVersion": "1.4",
        "version": 1,
        "serialNumber": format!("urn:uuid:{}", Uuid::new_v4()),
        "metadata": {
            "timestamp": Utc::now().to_rfc3339(),
        },
        "components": components,
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::{PackageId, PackageMetadata};

    fn make_meta(
        registry: &str,
        name: &str,
        version: &str,
        checksum: Option<&str>,
    ) -> PackageMetadata {
        PackageMetadata {
            id: PackageId::new(registry, name, version),
            published_at: None,
            download_url: None,
            checksum: checksum.map(|s| s.to_owned()),
            is_signed: None,
            extra: serde_json::Value::Null,
            cache_control: None,
        }
    }

    #[test]
    fn generate_spdx_required_fields() {
        let meta = make_meta("cargo", "tokio", "1.0.0", Some("abc123"));
        let doc = generate_spdx(&meta, "artifact:cargo/tokio/1.0.0", &[]);

        assert_eq!(doc["spdxVersion"], "SPDX-2.3");
        assert_eq!(doc["dataLicense"], "CC0-1.0");
        assert_eq!(doc["packages"][0]["versionInfo"], "1.0.0");
        assert_eq!(doc["packages"][0]["checksums"][0]["algorithm"], "SHA256");
        assert_eq!(
            doc["packages"][0]["checksums"][0]["checksumValue"],
            "abc123"
        );
        assert_eq!(doc["relationships"][0]["relationshipType"], "DESCRIBES");
    }

    #[test]
    fn generate_spdx_no_checksum() {
        let meta = make_meta("npm", "lodash", "4.17.21", None);
        let doc = generate_spdx(&meta, "k", &[]);
        assert!(doc["packages"][0]["checksums"]
            .as_array()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn generate_spdx_with_deps() {
        let meta = make_meta("npm", "express", "4.0.0", None);
        let deps = vec![SbomDependency {
            name: "accepts".into(),
            version_req: Some("1.3.8".into()),
            ecosystem: "npm".into(),
        }];
        let doc = generate_spdx(&meta, "k", &deps);
        // main package + 1 dep
        assert_eq!(doc["packages"].as_array().unwrap().len(), 2);
        assert_eq!(doc["relationships"].as_array().unwrap().len(), 2);
        assert_eq!(doc["relationships"][1]["relationshipType"], "DEPENDENCY_OF");
    }

    #[test]
    fn generate_cyclonedx_required_fields() {
        let meta = make_meta("cargo", "serde", "1.0.0", Some("deadbeef"));
        let doc = generate_cyclonedx(&meta, "k", &[]);

        assert_eq!(doc["bomFormat"], "CycloneDX");
        assert_eq!(doc["specVersion"], "1.4");
        assert_eq!(doc["components"][0]["name"], "serde");
        assert_eq!(doc["components"][0]["version"], "1.0.0");
        assert_eq!(doc["components"][0]["purl"], "pkg:cargo/serde@1.0.0");
        assert_eq!(doc["components"][0]["hashes"][0]["alg"], "SHA-256");
    }

    #[test]
    fn registry_to_purl_variants() {
        assert_eq!(
            registry_to_purl("cargo", "tokio", "1.0.0"),
            "pkg:cargo/tokio@1.0.0"
        );
        assert_eq!(
            registry_to_purl("npm", "lodash", "4.17.21"),
            "pkg:npm/lodash@4.17.21"
        );
        assert_eq!(
            registry_to_purl("pypi", "requests", "2.31.0"),
            "pkg:pypi/requests@2.31.0"
        );
        assert_eq!(
            registry_to_purl("rubygems", "rails", "7.0.0"),
            "pkg:gem/rails@7.0.0"
        );
        assert_eq!(
            registry_to_purl("goproxy", "github.com/gin-gonic/gin", "v1.9.0"),
            "pkg:golang/github.com/gin-gonic/gin@v1.9.0"
        );
        assert_eq!(
            registry_to_purl("unknown", "foo", "1.0"),
            "pkg:generic/foo@1.0"
        );
    }
}
