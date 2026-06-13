use batlehub_core::ports::SbomDependency;
use bytes::Bytes;

pub(super) fn extract_pypi_deps(data: &Bytes) -> Vec<SbomDependency> {
    // Try wheel (zip) first, then sdist (tar.gz).
    extract_pypi_wheel(data)
        .or_else(|| extract_pypi_sdist(data))
        .unwrap_or_default()
}

fn extract_pypi_wheel(data: &Bytes) -> Option<Vec<SbomDependency>> {
    use std::io::{Cursor, Read};
    use zip::ZipArchive;

    let cursor = Cursor::new(data.as_ref());
    let mut archive = ZipArchive::new(cursor).ok()?;

    for i in 0..archive.len() {
        let mut file = archive.by_index(i).ok()?;
        let name = file.name().to_owned();
        if name.ends_with(".dist-info/METADATA") {
            let mut content = String::new();
            file.read_to_string(&mut content).ok()?;
            return Some(parse_pep_metadata(&content));
        }
    }
    None
}

fn extract_pypi_sdist(data: &Bytes) -> Option<Vec<SbomDependency>> {
    use flate2::read::GzDecoder;
    use std::io::Read;
    use tar::Archive;

    let gz = GzDecoder::new(data.as_ref());
    let mut archive = Archive::new(gz);

    for entry in archive.entries().ok()?.flatten() {
        let path = entry.path().ok()?.into_owned();
        let fname = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if fname == "PKG-INFO" || fname == "METADATA" {
            let mut reader = entry;
            let mut content = String::new();
            if reader.read_to_string(&mut content).is_ok() {
                return Some(parse_pep_metadata(&content));
            }
        }
    }
    None
}

fn parse_pep_metadata(content: &str) -> Vec<SbomDependency> {
    content
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            line.strip_prefix("Requires-Dist:").map(|rest| {
                // "Requires-Dist: requests >=2.0" or "requests"
                let dep = rest.trim().split(';').next().unwrap_or(rest.trim());
                let mut parts = dep.splitn(2, ' ');
                let name = parts.next().unwrap_or("").trim().to_owned();
                let ver = parts.next().map(|v| v.trim().to_owned());
                SbomDependency {
                    name,
                    version_req: ver.filter(|v| !v.is_empty()),
                    ecosystem: "pypi".into(),
                }
            })
        })
        .filter(|d| !d.name.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_pep_metadata_basic() {
        let metadata =
            "Name: requests\nVersion: 2.31.0\nRequires-Dist: urllib3 >=1.21\nRequires-Dist: certifi\n";
        let deps = parse_pep_metadata(metadata);
        assert_eq!(deps.len(), 2);
        assert_eq!(deps[0].name, "urllib3");
        assert_eq!(deps[0].version_req.as_deref(), Some(">=1.21"));
        assert_eq!(deps[1].name, "certifi");
        assert!(deps[1].version_req.is_none());
    }
}
