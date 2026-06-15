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

    #[test]
    fn parse_pep_metadata_strips_environment_markers() {
        let deps = parse_pep_metadata("Requires-Dist: pytest >=7 ; extra == 'test'\n");
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "pytest");
        assert_eq!(deps[0].version_req.as_deref(), Some(">=7"));
        assert_eq!(deps[0].ecosystem, "pypi");
    }

    #[test]
    fn extract_pypi_deps_reads_wheel_metadata() {
        use std::io::Write;
        use zip::write::SimpleFileOptions;
        let mut buf = Vec::new();
        {
            let mut zw = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            zw.start_file(
                "requests-2.31.0.dist-info/METADATA",
                SimpleFileOptions::default(),
            )
            .unwrap();
            zw.write_all(b"Requires-Dist: urllib3 >=1.21\n").unwrap();
            zw.finish().unwrap();
        }
        let deps = extract_pypi_deps(&Bytes::from(buf));
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "urllib3");
        assert_eq!(deps[0].ecosystem, "pypi");
    }

    #[test]
    fn extract_pypi_deps_reads_sdist_pkg_info() {
        use flate2::{write::GzEncoder, Compression};
        use std::io::Write;
        let pkg_info: &[u8] = b"Requires-Dist: certifi\n";
        let mut tar_buf = Vec::new();
        {
            let mut b = tar::Builder::new(&mut tar_buf);
            let mut h = tar::Header::new_gnu();
            h.set_size(pkg_info.len() as u64);
            h.set_mode(0o644);
            h.set_cksum();
            b.append_data(&mut h, "requests-2.31.0/PKG-INFO", pkg_info)
                .unwrap();
            b.finish().unwrap();
        }
        let mut gz = GzEncoder::new(Vec::new(), Compression::default());
        gz.write_all(&tar_buf).unwrap();
        let deps = extract_pypi_deps(&Bytes::from(gz.finish().unwrap()));
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "certifi");
    }

    #[test]
    fn extract_pypi_deps_on_garbage_is_empty() {
        assert!(extract_pypi_deps(&Bytes::from_static(b"not an archive")).is_empty());
    }
}
