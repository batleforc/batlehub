//! npm: the packument.
//!
//! The one document that answers both halves of what the package page is
//! missing — the version list *and* the README — which is why RFC 0007 §2.3
//! argues the discovery read is worth having at all.

use super::{is_prerelease, json, parse_time, UpstreamDetail, UpstreamVersion};
use crate::entities::{MetadataLinks, MetadataReadme, ReadmeFormat};
use crate::ports::VersionDocument;

/// npm's placeholder for "the tarball had no README" — a string, so a presence
/// check alone would show an error message as documentation.
const MISSING_README: &str = "ERROR: No README data found!";

pub(super) fn read(doc: &VersionDocument) -> UpstreamDetail {
    let Some(root) = json(doc) else {
        return UpstreamDetail::default();
    };
    let Some(versions) = root.get("versions").and_then(|v| v.as_object()) else {
        return UpstreamDetail::default();
    };
    let times = root.get("time").and_then(|t| t.as_object());
    let latest = root
        .get("dist-tags")
        .and_then(|t| t.get("latest"))
        .and_then(|v| v.as_str());

    let mut detail = UpstreamDetail::default();
    for (version, meta) in versions {
        detail.versions.push(UpstreamVersion {
            version: version.clone(),
            published_at: times
                .and_then(|t| t.get(version))
                .and_then(|v| v.as_str())
                .and_then(parse_time),
            is_prerelease: is_prerelease(version),
            // npm has no `yanked`: an unpublished version is absent from the
            // document rather than marked in it.
            yanked: false,
            deprecated: meta
                .get("deprecated")
                .and_then(|v| v.as_str())
                .map(str::to_owned),
        });
        if let Some(text) = usable(meta.get("readme")) {
            detail.readmes.insert(
                version.clone(),
                MetadataReadme::text(text, ReadmeFormat::Markdown),
            );
        }
    }

    // The document-root README describes whatever `dist-tags.latest` points at,
    // and is attributed to that version and to no other — inventing a
    // per-version claim from a package-level field would show 2.x's API to a
    // 1.x reader (RFC 0007 §2.4, decision 6).
    if let Some(latest) = latest {
        if !detail.readmes.contains_key(latest) {
            if let Some(text) = usable(root.get("readme")) {
                detail.readmes.insert(
                    latest.to_owned(),
                    MetadataReadme::text(text, ReadmeFormat::Markdown).package_level(),
                );
            }
        }
    }

    // The links, read from `dist-tags.latest`'s entry and falling back to the
    // document root. Package-level either way — this is the answer for the
    // package, and the selected version's own entry is what the metadata cache
    // holds when something has resolved it.
    //
    // `latest` first because a packument's root fields are a copy of the latest
    // publish's `package.json` and go stale when a package moves forge without
    // cutting a release; the two agree for every package where nothing moved.
    let latest_meta = latest.and_then(|latest| versions.get(latest));
    detail.links = MetadataLinks::new(
        repository_url(latest_meta).or_else(|| repository_url(Some(root))),
        string(latest_meta.and_then(|m| m.get("homepage")))
            .or_else(|| string(root.get("homepage"))),
    );

    detail
}

/// npm spells `repository` two ways and both are in the wild: a bare string
/// (often the `github:user/repo` shorthand) or `{ "type": "git", "url": … }`.
/// `MetadataLinks::new` untangles the *spelling* of the URL; this only has to
/// accept both shapes of the field.
fn repository_url(meta: Option<&serde_json::Value>) -> Option<&str> {
    let repository = meta?.get("repository")?;
    match repository {
        serde_json::Value::String(url) => Some(url.as_str()),
        _ => repository.get("url").and_then(|v| v.as_str()),
    }
}

fn string(value: Option<&serde_json::Value>) -> Option<&str> {
    value.and_then(|v| v.as_str())
}

fn usable(value: Option<&serde_json::Value>) -> Option<String> {
    value
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|t| !t.is_empty() && *t != MISSING_README)
        .map(str::to_owned)
}
