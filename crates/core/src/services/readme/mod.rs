//! Storing and serving each version's own README (RFC 0007).
//!
//! Everything that *writes* here is driven by resolution, publication or the
//! single introspection pass over a cached artifact. **Nothing on this path runs
//! on a package-manager request in a way that fetches anything new, and nothing
//! on it is triggered by a page view** — a page view for a version we hold reads
//! one row and, on a miss in the render cache, runs the renderer.

use std::sync::Arc;

use chrono::Utc;

use crate::entities::{
    readme_digest, MetadataReadme, PackageMetadata, PackageReadme, ReadmeFormat, ReadmeSource,
};
use crate::error::CoreError;
use crate::ports::{CacheEntry, CacheStore, ReadmeRepository};
use crate::services::hot_config::ReadmeConfig;

pub mod detect;
pub mod render;
pub mod sanitize;

/// Storing and serving the README of a version.
pub struct ReadmeService {
    pub repo: Arc<dyn ReadmeRepository>,
    /// The render cache, keyed by content digest and renderer version.
    ///
    /// Optional because the write paths — capture on resolve, on publish, on the
    /// introspection pass — do not render anything, and a test that only
    /// exercises those should not have to build a cache. A `None` here costs a
    /// render per read, not a wrong answer.
    pub cache: Option<Arc<dyn CacheStore>>,
}

/// One README, and whether it is the version the caller asked about.
#[derive(Debug, Clone)]
pub struct ReadmeAnswer {
    pub readme: PackageReadme,
    /// The stored README belongs to a different version than the one requested.
    /// The panel says so in words rather than showing prose that belongs to
    /// different code (RFC 0007 §4.2).
    pub is_fallback: bool,
}

/// What a record attempt did, so the caller can log the interesting cases
/// without the service deciding how loud they are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordOutcome {
    /// A row was written or replaced.
    Stored,
    /// The stored text is byte-identical, so nothing was written.
    ///
    /// This is the common case on a re-resolve: metadata TTLs expire far more
    /// often than upstreams edit a published README, and rewriting an identical
    /// row would move `extracted_at` and make the page claim a re-read that
    /// changed nothing.
    Unchanged,
    /// There was nothing to record — the document carried no README, or it was
    /// empty once trimmed.
    Nothing,
}

/// The text with everything the store needs to describe it.
///
/// One struct rather than five parameters because the three `record_*` entry
/// points differ only in where they got it, and a positional `(String,
/// ReadmeFormat, bool)` at three call sites is how a `truncated` flag ends up
/// in the `package_level` slot.
pub struct ReadmeCapture {
    pub content: String,
    pub format: ReadmeFormat,
    pub source: ReadmeSource,
    pub package_level: bool,
}

impl ReadmeService {
    pub fn new(repo: Arc<dyn ReadmeRepository>) -> Self {
        Self { repo, cache: None }
    }

    /// The same service with a render cache attached.
    pub fn with_cache(mut self, cache: Arc<dyn CacheStore>) -> Self {
        self.cache = Some(cache);
        self
    }

    /// The README to show for `version`, applying the fallback rule.
    ///
    /// An exact hit answers for itself. When the requested version has none, the
    /// newest version that does answers instead, flagged — because a package
    /// whose 2.0.0-rc1 ships no README is better served by 1.4.2's than by an
    /// empty panel, as long as the page says which it is showing.
    ///
    /// `ineligible` is the caller's list of versions that may not be a fallback
    /// source: a blocked version serves no README at all, and an unlisted one is
    /// hidden from listings, so neither may be substituted in silently. The
    /// repository knows nothing about firewall state, so the decision stays with
    /// the layer that does (RFC 0007 §4.4).
    pub async fn get_for_version(
        &self,
        registry: &str,
        name: &str,
        version: &str,
        ineligible: &[String],
    ) -> Result<Option<ReadmeAnswer>, CoreError> {
        if let Some(exact) = self.repo.get(registry, name, version).await? {
            return Ok(Some(ReadmeAnswer {
                readme: exact,
                is_fallback: false,
            }));
        }
        // The requested version is itself ineligible only in the sense that it
        // has nothing to give; the exclusion list is about *substitution*, and
        // asking for a version's own README is never a substitution.
        let mut excluded = ineligible.to_vec();
        excluded.push(version.to_owned());
        Ok(self
            .repo
            .get_latest_with_readme(registry, name, &excluded)
            .await?
            .map(|readme| ReadmeAnswer {
                readme,
                is_fallback: true,
            }))
    }

    /// Render `readme` to sanitised HTML, through the cache.
    ///
    /// Keyed by the content digest and the renderer version, so two versions
    /// with an identical README — the common case for a patch release — render
    /// once, and a fix to the sanitiser invalidates every rendering by bumping
    /// [`sanitize::RENDERER_VERSION`] rather than by a backfill.
    ///
    /// A cache failure is not an error: the renderer is a pure function and
    /// running it again is the correct answer, just slower.
    pub async fn render_cached(
        &self,
        readme: &PackageReadme,
        opts: &render::RenderOptions,
    ) -> String {
        let key = render_cache_key(&readme.digest, readme.format, opts);
        if let Some(cache) = &self.cache {
            if let Ok(Some(entry)) = cache.get(&key).await {
                if let Some(html) = entry
                    .metadata
                    .extra
                    .get("readme_html")
                    .and_then(|v| v.as_str())
                {
                    return html.to_owned();
                }
            }
        }
        let html = render::render(&readme.content, readme.format, opts);
        if let Some(cache) = &self.cache {
            let entry = CacheEntry {
                metadata: crate::entities::PackageMetadata {
                    // The coordinate is not part of the key — the cache is
                    // content-addressed — but a `PackageMetadata` needs one, and
                    // recording the version this rendering happened to come from
                    // is more useful to somebody reading the cache than a blank.
                    id: crate::entities::PackageId::new(
                        &readme.registry,
                        &readme.name,
                        &readme.version,
                    ),
                    published_at: None,
                    download_url: None,
                    checksum: None,
                    is_signed: None,
                    extra: serde_json::json!({ "readme_html": html }),
                    cache_control: None,
                },
                cached_at: Utc::now(),
                expires_at: None,
            };
            // No TTL: the entry is keyed by content digest and renderer version,
            // so it can only ever be right. It goes when the cache evicts it.
            if let Err(e) = cache.set(&key, entry, None).await {
                tracing::debug!(error = %e, "readme: render cache write failed (non-fatal)");
            }
        }
        html
    }

    /// Record the README a registry client parsed out of a metadata document.
    ///
    /// Reads [`MetadataReadme`] off `PackageMetadata::extra`, so a registry
    /// type that carries no README costs one absent map lookup. A *linked*
    /// README (a URL, not text) is not followed here — that is an outbound
    /// request and belongs in the detached task, not on the resolve path.
    pub async fn record_from_metadata(
        &self,
        meta: &PackageMetadata,
        cfg: &ReadmeConfig,
    ) -> Result<RecordOutcome, CoreError> {
        let Some(found) = MetadataReadme::from_extra(&meta.extra) else {
            return Ok(RecordOutcome::Nothing);
        };
        let Some(content) = found.content else {
            return Ok(RecordOutcome::Nothing);
        };
        self.record(
            &meta.id.registry,
            &meta.id.name,
            &meta.id.version,
            ReadmeCapture {
                content,
                format: found.format,
                source: ReadmeSource::UpstreamMetadata,
                package_level: found.package_level,
            },
            cfg,
        )
        .await
    }

    /// Record the README read from a URL the metadata document linked to.
    ///
    /// [`ReadmeSource::UpstreamMetadata`], not `Archive`: the text came from the
    /// upstream's own answer about this version, and an operator asking where a
    /// document came from wants "the upstream said so", not "we opened the
    /// bytes". The link is followed by the caller, which is the layer with the
    /// HTTP client and the same-origin guard (RFC 0007 §7.4).
    pub async fn record_from_linked(
        &self,
        id: &crate::entities::PackageId,
        content: String,
        format: ReadmeFormat,
        cfg: &ReadmeConfig,
    ) -> Result<RecordOutcome, CoreError> {
        self.record(
            &id.registry,
            &id.name,
            &id.version,
            ReadmeCapture {
                content,
                format,
                source: ReadmeSource::UpstreamMetadata,
                package_level: false,
            },
            cfg,
        )
        .await
    }

    /// Record the README a publish request carried in its own metadata.
    ///
    /// `LocalPublish` rather than `UpstreamMetadata` because the difference is
    /// visible to an operator asking where a document came from, and on a hybrid
    /// registry both are possible for the same package name.
    pub async fn record_from_publish(
        &self,
        registry: &str,
        name: &str,
        version: &str,
        content: String,
        format: ReadmeFormat,
        cfg: &ReadmeConfig,
    ) -> Result<RecordOutcome, CoreError> {
        self.record(
            registry,
            name,
            version,
            ReadmeCapture {
                content,
                format,
                source: ReadmeSource::LocalPublish,
                package_level: false,
            },
            cfg,
        )
        .await
    }

    /// Record a README read out of the artifact on the single introspection
    /// pass (RFC 0007 §5.2).
    pub async fn record_from_archive(
        &self,
        registry: &str,
        name: &str,
        version: &str,
        content: String,
        format: ReadmeFormat,
        cfg: &ReadmeConfig,
    ) -> Result<RecordOutcome, CoreError> {
        self.record(
            registry,
            name,
            version,
            ReadmeCapture {
                content,
                format,
                source: ReadmeSource::Archive,
                package_level: false,
            },
            cfg,
        )
        .await
    }

    /// The one writer. Trims, caps, and refuses to rewrite identical text.
    async fn record(
        &self,
        registry: &str,
        name: &str,
        version: &str,
        capture: ReadmeCapture,
        cfg: &ReadmeConfig,
    ) -> Result<RecordOutcome, CoreError> {
        if !cfg.enabled {
            return Ok(RecordOutcome::Nothing);
        }
        // Whitespace-only is *nothing*, not a document. An empty panel and a
        // panel showing three blank lines look identical to a reader, and only
        // one of them is honest about there being no README.
        let trimmed = capture.content.trim();
        if trimmed.is_empty() {
            return Ok(RecordOutcome::Nothing);
        }
        let (content, truncated) = truncate_to(trimmed, cfg.max_bytes);
        let digest = readme_digest(&content);

        // An unchanged digest means the upstream did not edit anything, which is
        // the normal outcome of a TTL expiry. Rewriting the row would move
        // `extracted_at` and make the page report a re-read that changed nothing.
        if let Some(existing) = self.repo.get(registry, name, version).await? {
            if existing.digest == digest {
                return Ok(RecordOutcome::Unchanged);
            }
        }

        self.repo
            .upsert(PackageReadme {
                registry: registry.to_owned(),
                name: name.to_owned(),
                version: version.to_owned(),
                content,
                format: capture.format,
                source: capture.source,
                digest,
                truncated,
                package_level: capture.package_level,
                extracted_at: Utc::now(),
            })
            .await?;
        Ok(RecordOutcome::Stored)
    }
}

/// The render cache key for one document under one set of options.
///
/// Content-addressed, so nothing about *which package* is in it: two versions
/// with the same README are one entry, and so are two packages that copied one.
/// The renderer version and the options are in the key because both change the
/// output — a rendering made under `remote_images = "strip"` must never be
/// served to a registry configured to proxy them.
fn render_cache_key(digest: &str, format: ReadmeFormat, opts: &render::RenderOptions) -> String {
    let variant = readme_digest(&format!(
        "{}|{}|{}",
        format.as_str(),
        opts.remote_images.as_str(),
        opts.image_proxy_prefix.as_deref().unwrap_or("")
    ));
    format!(
        "readme-html:{}:{}:{digest}",
        sanitize::RENDERER_VERSION,
        &variant[..16]
    )
}

/// Cut `text` to at most `max_bytes` **bytes**, on a character boundary.
///
/// Bytes rather than characters because the cap exists to bound a database row
/// and the memory a render holds, and both are measured in bytes. Cutting on a
/// boundary matters: a `String` cannot hold half a code point, and a cap that
/// panicked on a README with an emoji in the wrong place would be a crash
/// triggered by publishing.
pub fn truncate_to(text: &str, max_bytes: usize) -> (String, bool) {
    if text.len() <= max_bytes {
        return (text.to_owned(), false);
    }
    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    (text[..end].to_owned(), true)
}

#[cfg(test)]
mod tests;
