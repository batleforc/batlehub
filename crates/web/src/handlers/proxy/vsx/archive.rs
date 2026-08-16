//! Reading a VSIX.
//!
//! A `.vsix` is a ZIP whose extension lives under `extension/`, with the
//! manifest at `extension/package.json`. Everything the gallery serves except
//! the package itself — the manifest, the README, the changelog, the licence,
//! the icon — is a file *inside* that archive, so one cached artifact answers
//! every asset request and local mode works identically to proxy mode.
//!
//! Two hazards this module exists to contain, both the same ones
//! `jetbrains_marketplace/plugin_archive.rs` guards against: a decompression
//! bomb, and a path that escapes the archive.

use std::io::Read;

use bytes::Bytes;

use crate::error::AppError;

/// Largest single entry this will decompress. Generous for a README with
/// embedded images, far below anything that would threaten the process.
const MAX_ENTRY_BYTES: u64 = 32 * 1024 * 1024;

/// The manifest is JSON describing one extension; 10 MiB is already absurd for
/// that, and the same ceiling `plugin_archive.rs` puts on `plugin.xml`.
const MAX_MANIFEST_BYTES: u64 = 10 * 1024 * 1024;

/// Where the VS Code extension itself lives inside the archive.
const EXTENSION_PREFIX: &str = "extension/";

/// The fields of `extension/package.json` the gallery reports.
///
/// Deliberately a small typed subset rather than the whole manifest: these are
/// the values the two protocols actually render, and a partial parse of a
/// manifest with fields this proxy has never seen is better than refusing the
/// extension.
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct VsixManifest {
    pub publisher: String,
    pub name: String,
    pub version: String,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub categories: Vec<String>,
    pub keywords: Vec<String>,
    pub extension_pack: Vec<String>,
    pub extension_dependencies: Vec<String>,
    /// Relative path inside `extension/`, e.g. `images/icon.png`.
    pub icon: Option<String>,
    pub engines: VsixEngines,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct VsixEngines {
    pub vscode: Option<String>,
}

impl VsixManifest {
    /// `{publisher}.{name}`, the id both protocols address by.
    pub fn extension_id(&self) -> String {
        format!("{}.{}", self.publisher, self.name)
    }
}

/// Parse `extension/package.json` out of VSIX bytes.
///
/// `None` — never an error — when the bytes are not a ZIP, the manifest is
/// missing, or it does not parse. The raw `PUT` publish endpoint accepts
/// arbitrary bytes by design, and an extension published without a readable
/// manifest should appear in the gallery with its coordinate and nothing else
/// rather than fail to publish at all.
pub fn parse_manifest(bytes: &[u8]) -> Option<VsixManifest> {
    let cursor = std::io::Cursor::new(bytes);
    let mut zip = zip::ZipArchive::new(cursor).ok()?;

    let file = zip.by_name("extension/package.json").ok()?;
    let mut buf = Vec::new();
    file.take(MAX_MANIFEST_BYTES + 1)
        .read_to_end(&mut buf)
        .ok()?;
    if buf.len() as u64 > MAX_MANIFEST_BYTES {
        tracing::warn!("VSIX manifest exceeds {MAX_MANIFEST_BYTES} bytes; ignoring it");
        return None;
    }
    serde_json::from_slice(&buf).ok()
}

/// Read one file out of a VSIX, by its path *relative to `extension/`*.
///
/// `Ok(None)` when the entry is absent — the caller turns that into a `404`,
/// which is what an editor expects for an extension with no changelog.
pub fn read_entry(bytes: &[u8], relative_path: &str) -> Result<Option<Bytes>, AppError> {
    // Reject traversal before it reaches the archive. `by_name` is an exact key
    // lookup rather than a filesystem join, so this is defence in depth — but a
    // `..` in the path is never legitimate and saying so at the edge keeps the
    // guarantee local to this function.
    if relative_path.is_empty()
        || relative_path
            .split('/')
            .any(|seg| seg == ".." || seg == "." || seg.is_empty())
    {
        return Err(AppError::bad_request(format!(
            "invalid path inside the extension: '{relative_path}'"
        )));
    }

    let cursor = std::io::Cursor::new(bytes);
    let mut zip = zip::ZipArchive::new(cursor).map_err(|e| {
        AppError::bad_gateway(format!("extension package is not a valid VSIX: {e}"))
    })?;

    let key = format!("{EXTENSION_PREFIX}{relative_path}");
    let file = match zip.by_name(&key) {
        Ok(f) => f,
        Err(_) => return Ok(None),
    };

    let mut buf = Vec::new();
    file.take(MAX_ENTRY_BYTES + 1)
        .read_to_end(&mut buf)
        .map_err(|e| {
            AppError::bad_gateway(format!("reading '{relative_path}' from the VSIX: {e}"))
        })?;
    if buf.len() as u64 > MAX_ENTRY_BYTES {
        return Err(AppError::bad_gateway(format!(
            "'{relative_path}' exceeds the {MAX_ENTRY_BYTES}-byte decompressed limit"
        )));
    }
    Ok(Some(Bytes::from(buf)))
}

/// The first entry under `extension/` whose name matches `predicate`, for the
/// assets whose filename is a convention rather than a manifest field —
/// `README.md`, `CHANGELOG.md`, `LICENSE.txt` and their many spellings.
pub fn find_entry(bytes: &[u8], predicate: impl Fn(&str) -> bool) -> Option<String> {
    let cursor = std::io::Cursor::new(bytes);
    let mut zip = zip::ZipArchive::new(cursor).ok()?;
    let mut names: Vec<String> = (0..zip.len())
        .filter_map(|i| zip.by_index(i).ok().map(|f| f.name().to_owned()))
        .filter_map(|n| n.strip_prefix(EXTENSION_PREFIX).map(str::to_owned))
        // Top-level files only: a README nested in a subdirectory of the
        // extension is documentation *of* something inside it, not the
        // extension's own, and matching it would show the wrong document.
        .filter(|n| !n.contains('/') && !n.is_empty())
        .collect();
    // Deterministic across archives whose entry order differs.
    names.sort();
    names.into_iter().find(|n| predicate(n))
}

/// The `Content-Type` for a file served out of a VSIX.
///
/// These bytes are attacker-influenced and served from the same origin as the
/// admin console, which holds a bearer token — so the type is chosen from a
/// closed allowlist rather than guessed from the extension.
///
/// **SVG is deliberately not `image/svg+xml`.** An SVG served with that type
/// executes script in the document's origin, which would turn "an extension
/// shipped an icon" into console-session theft. It goes out as an opaque
/// download instead; the editor renders no icon, which is the correct trade.
pub fn content_type_for(path: &str) -> &'static str {
    let lower = path.to_ascii_lowercase();
    match lower.rsplit('.').next() {
        Some("json") => "application/json",
        Some("md") | Some("markdown") => "text/markdown; charset=utf-8",
        Some("txt") => "text/plain; charset=utf-8",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn vsix(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let mut w = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            let opts: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            for (name, body) in entries {
                w.start_file(*name, opts).unwrap();
                w.write_all(body).unwrap();
            }
            w.finish().unwrap();
        }
        buf
    }

    fn manifest_bytes() -> &'static [u8] {
        br#"{
            "publisher": "acme",
            "name": "tool",
            "version": "1.2.3",
            "displayName": "Acme Tool",
            "description": "Does the thing",
            "categories": ["Linters"],
            "keywords": ["acme"],
            "extensionPack": ["acme.other"],
            "extensionDependencies": ["ms-python.python"],
            "icon": "images/icon.png",
            "engines": { "vscode": "^1.85.0" }
        }"#
    }

    #[test]
    fn the_manifest_is_parsed_from_the_extension_directory() {
        let z = vsix(&[("extension/package.json", manifest_bytes())]);
        let m = parse_manifest(&z).expect("manifest parses");

        assert_eq!(m.extension_id(), "acme.tool");
        assert_eq!(m.version, "1.2.3");
        assert_eq!(m.display_name.as_deref(), Some("Acme Tool"));
        assert_eq!(m.engines.vscode.as_deref(), Some("^1.85.0"));
        assert_eq!(m.categories, ["Linters"]);
        assert_eq!(m.extension_pack, ["acme.other"]);
        assert_eq!(m.icon.as_deref(), Some("images/icon.png"));
    }

    /// A manifest carrying fields this proxy has never modelled must still
    /// parse — refusing it would refuse the extension.
    #[test]
    fn unknown_manifest_fields_are_ignored() {
        let z = vsix(&[(
            "extension/package.json",
            br#"{"publisher":"a","name":"b","version":"1.0.0","contributes":{"commands":[]}}"#,
        )]);
        assert_eq!(parse_manifest(&z).unwrap().extension_id(), "a.b");
    }

    /// The publish endpoint accepts arbitrary bytes by design; a non-ZIP body
    /// degrades to "no manifest", not to an error.
    #[test]
    fn junk_bytes_yield_no_manifest_rather_than_an_error() {
        assert!(parse_manifest(b"PK\x03\x04not-really-a-zip").is_none());
        assert!(parse_manifest(b"").is_none());
    }

    #[test]
    fn a_vsix_without_a_manifest_yields_none() {
        let z = vsix(&[("extension/README.md", b"hi")]);
        assert!(parse_manifest(&z).is_none());
    }

    #[test]
    fn an_entry_is_read_relative_to_the_extension_directory() {
        let z = vsix(&[("extension/README.md", b"# hello")]);
        let got = read_entry(&z, "README.md").unwrap().expect("entry found");
        assert_eq!(&got[..], b"# hello");
    }

    #[test]
    fn a_missing_entry_is_none_not_an_error() {
        let z = vsix(&[("extension/package.json", manifest_bytes())]);
        assert!(read_entry(&z, "CHANGELOG.md").unwrap().is_none());
    }

    /// The path is attacker-supplied on the `unpkg` route.
    #[test]
    fn traversal_is_rejected_at_the_edge() {
        let z = vsix(&[("extension/package.json", manifest_bytes())]);
        for bad in ["../secret", "a/../../b", "./x", "", "a//b"] {
            assert!(
                read_entry(&z, bad).is_err(),
                "'{bad}' should be rejected outright"
            );
        }
    }

    #[test]
    fn find_entry_matches_top_level_files_only() {
        let z = vsix(&[
            ("extension/guide/README.md", b"not this one"),
            ("extension/README.md", b"this one"),
        ]);
        let found = find_entry(&z, |n| n.eq_ignore_ascii_case("readme.md"));
        assert_eq!(found.as_deref(), Some("README.md"));
    }

    /// SVG must never be served as `image/svg+xml` from the console's origin.
    #[test]
    fn svg_is_not_served_as_a_renderable_image() {
        assert_eq!(content_type_for("icon.svg"), "application/octet-stream");
        assert_eq!(content_type_for("ICON.SVG"), "application/octet-stream");
    }

    #[test]
    fn known_types_are_declared_and_unknown_ones_are_opaque() {
        assert_eq!(content_type_for("package.json"), "application/json");
        assert_eq!(
            content_type_for("README.md"),
            "text/markdown; charset=utf-8"
        );
        assert_eq!(content_type_for("images/icon.png"), "image/png");
        assert_eq!(content_type_for("thing.exe"), "application/octet-stream");
        assert_eq!(content_type_for("noextension"), "application/octet-stream");
    }
}
