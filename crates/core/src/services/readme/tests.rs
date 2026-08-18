use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use super::*;
use crate::entities::{PackageId, PackageMetadata};

/// A minimal in-memory `ReadmeRepository`, local to these tests.
///
/// `crates/adapters`'s is not reachable from here — `core` does not depend on
/// `adapters`, which is the dependency direction the whole crate layout rests
/// on — and these tests are about the service's rules, not the store's.
#[derive(Default)]
struct FakeRepo {
    rows: Mutex<Vec<PackageReadme>>,
    writes: Mutex<usize>,
}

impl FakeRepo {
    fn arc() -> Arc<Self> {
        Arc::new(Self::default())
    }
    fn writes(&self) -> usize {
        *self.writes.lock().unwrap()
    }
    fn only(&self) -> PackageReadme {
        let rows = self.rows.lock().unwrap();
        assert_eq!(rows.len(), 1, "expected exactly one row");
        rows[0].clone()
    }
}

#[async_trait]
impl ReadmeRepository for FakeRepo {
    async fn upsert(&self, readme: PackageReadme) -> Result<(), CoreError> {
        *self.writes.lock().unwrap() += 1;
        let mut rows = self.rows.lock().unwrap();
        rows.retain(|r| {
            !(r.registry == readme.registry && r.name == readme.name && r.version == readme.version)
        });
        rows.push(readme);
        Ok(())
    }

    async fn get(
        &self,
        registry: &str,
        name: &str,
        version: &str,
    ) -> Result<Option<PackageReadme>, CoreError> {
        Ok(self
            .rows
            .lock()
            .unwrap()
            .iter()
            .find(|r| r.registry == registry && r.name == name && r.version == version)
            .cloned())
    }

    async fn get_latest_with_readme(
        &self,
        registry: &str,
        name: &str,
        exclude_versions: &[String],
    ) -> Result<Option<PackageReadme>, CoreError> {
        let rows = self.rows.lock().unwrap();
        let mut candidates: Vec<&PackageReadme> = rows
            .iter()
            .filter(|r| {
                r.registry == registry && r.name == name && !exclude_versions.contains(&r.version)
            })
            .collect();
        candidates.sort_by_key(|r| std::cmp::Reverse(r.extracted_at));
        Ok(candidates.first().map(|r| (*r).clone()))
    }

    async fn list_versions_with_readme(
        &self,
        registry: &str,
        name: &str,
    ) -> Result<Vec<String>, CoreError> {
        Ok(self
            .rows
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.registry == registry && r.name == name)
            .map(|r| r.version.clone())
            .collect())
    }

    async fn delete_for_version(
        &self,
        registry: &str,
        name: &str,
        version: &str,
    ) -> Result<(), CoreError> {
        self.rows
            .lock()
            .unwrap()
            .retain(|r| !(r.registry == registry && r.name == name && r.version == version));
        Ok(())
    }

    async fn delete_for_package(&self, registry: &str, name: &str) -> Result<(), CoreError> {
        self.rows
            .lock()
            .unwrap()
            .retain(|r| !(r.registry == registry && r.name == name));
        Ok(())
    }

    async fn search(
        &self,
        _registries: &[String],
        _query: &str,
        _limit: u64,
    ) -> Result<Vec<crate::ports::ReadmeSearchHit>, CoreError> {
        Ok(vec![])
    }
}

fn cfg() -> ReadmeConfig {
    ReadmeConfig {
        registry_type: "npm".into(),
        ..ReadmeConfig::default()
    }
}

fn metadata_with(extra: serde_json::Value) -> PackageMetadata {
    PackageMetadata {
        id: PackageId::new("npm1", "express", "4.18.2"),
        published_at: None,
        download_url: None,
        checksum: None,
        is_signed: None,
        extra,
        cache_control: None,
    }
}

#[tokio::test]
async fn a_metadata_readme_is_stored_for_the_version_it_arrived_with() {
    let repo = FakeRepo::arc();
    let svc = ReadmeService::new(Arc::clone(&repo) as Arc<dyn ReadmeRepository>);

    let meta = metadata_with(serde_json::json!({
        "readme": MetadataReadme::text("# express", ReadmeFormat::Markdown),
    }));
    assert_eq!(
        svc.record_from_metadata(&meta, &cfg()).await.unwrap(),
        RecordOutcome::Stored
    );

    let row = repo.only();
    assert_eq!(row.registry, "npm1");
    assert_eq!(row.name, "express");
    assert_eq!(row.version, "4.18.2");
    assert_eq!(row.content, "# express");
    assert_eq!(row.format, ReadmeFormat::Markdown);
    assert_eq!(row.source, ReadmeSource::UpstreamMetadata);
    assert!(!row.package_level);
    assert!(!row.truncated);
    assert_eq!(row.digest, readme_digest("# express"));
}

/// A registry type that carries no README costs one absent map lookup and
/// writes nothing — the common case across most of the catalogue.
#[tokio::test]
async fn metadata_without_a_readme_key_writes_nothing() {
    let repo = FakeRepo::arc();
    let svc = ReadmeService::new(Arc::clone(&repo) as Arc<dyn ReadmeRepository>);

    let meta = metadata_with(serde_json::json!({ "resolved_version": "4.18.2" }));
    assert_eq!(
        svc.record_from_metadata(&meta, &cfg()).await.unwrap(),
        RecordOutcome::Nothing
    );
    assert_eq!(repo.writes(), 0);
}

/// `extra` is registry-specific and best-effort. A client that wrote something
/// unexpected costs a missing README, not a failed resolve on a package
/// manager's request path.
#[tokio::test]
async fn a_malformed_readme_key_is_ignored_rather_than_erroring() {
    let repo = FakeRepo::arc();
    let svc = ReadmeService::new(Arc::clone(&repo) as Arc<dyn ReadmeRepository>);

    let meta = metadata_with(serde_json::json!({ "readme": "just a string" }));
    assert_eq!(
        svc.record_from_metadata(&meta, &cfg()).await.unwrap(),
        RecordOutcome::Nothing
    );
    assert_eq!(repo.writes(), 0);
}

/// A linked README is a URL, and following it is an outbound request that
/// belongs in the detached task — not on the resolve path a package manager is
/// waiting on.
#[tokio::test]
async fn a_linked_readme_is_not_followed_here() {
    let repo = FakeRepo::arc();
    let svc = ReadmeService::new(Arc::clone(&repo) as Arc<dyn ReadmeRepository>);

    let meta = metadata_with(serde_json::json!({
        "readme": MetadataReadme::linked("https://open-vsx.org/api/x/y/1/file/README.md",
                                         ReadmeFormat::Markdown),
    }));
    assert_eq!(
        svc.record_from_metadata(&meta, &cfg()).await.unwrap(),
        RecordOutcome::Nothing
    );
    assert_eq!(repo.writes(), 0);
}

/// An empty panel and a panel showing three blank lines look identical to a
/// reader, and only one of them is honest about there being no README.
#[tokio::test]
async fn whitespace_is_nothing_not_a_document() {
    let repo = FakeRepo::arc();
    let svc = ReadmeService::new(Arc::clone(&repo) as Arc<dyn ReadmeRepository>);

    let meta = metadata_with(serde_json::json!({
        "readme": MetadataReadme::text("   \n\t\n  ", ReadmeFormat::Markdown),
    }));
    assert_eq!(
        svc.record_from_metadata(&meta, &cfg()).await.unwrap(),
        RecordOutcome::Nothing
    );
    assert_eq!(repo.writes(), 0);
}

/// Metadata TTLs expire far more often than upstreams edit a published README.
/// Rewriting an identical row would move `extracted_at` and make the page claim
/// a re-read that changed nothing.
#[tokio::test]
async fn an_unchanged_readme_is_not_rewritten() {
    let repo = FakeRepo::arc();
    let svc = ReadmeService::new(Arc::clone(&repo) as Arc<dyn ReadmeRepository>);
    let meta = metadata_with(serde_json::json!({
        "readme": MetadataReadme::text("# express", ReadmeFormat::Markdown),
    }));

    assert_eq!(
        svc.record_from_metadata(&meta, &cfg()).await.unwrap(),
        RecordOutcome::Stored
    );
    assert_eq!(
        svc.record_from_metadata(&meta, &cfg()).await.unwrap(),
        RecordOutcome::Unchanged
    );
    assert_eq!(repo.writes(), 1);
}

/// npm permits editing a published README, so a genuinely different text does
/// replace the row — and `extracted_at` moves, which is what the page reports.
#[tokio::test]
async fn edited_text_replaces_the_row() {
    let repo = FakeRepo::arc();
    let svc = ReadmeService::new(Arc::clone(&repo) as Arc<dyn ReadmeRepository>);

    svc.record_from_metadata(
        &metadata_with(serde_json::json!({
            "readme": MetadataReadme::text("first", ReadmeFormat::Markdown),
        })),
        &cfg(),
    )
    .await
    .unwrap();
    assert_eq!(
        svc.record_from_metadata(
            &metadata_with(serde_json::json!({
                "readme": MetadataReadme::text("second", ReadmeFormat::Markdown),
            })),
            &cfg(),
        )
        .await
        .unwrap(),
        RecordOutcome::Stored
    );

    assert_eq!(repo.only().content, "second");
    assert_eq!(repo.writes(), 2);
}

#[tokio::test]
async fn a_disabled_registry_stores_nothing() {
    let repo = FakeRepo::arc();
    let svc = ReadmeService::new(Arc::clone(&repo) as Arc<dyn ReadmeRepository>);
    let off = ReadmeConfig {
        enabled: false,
        ..cfg()
    };

    let meta = metadata_with(serde_json::json!({
        "readme": MetadataReadme::text("# express", ReadmeFormat::Markdown),
    }));
    assert_eq!(
        svc.record_from_metadata(&meta, &off).await.unwrap(),
        RecordOutcome::Nothing
    );
    assert_eq!(repo.writes(), 0);
}

/// Truncation is recorded and surfaced, never silent.
#[tokio::test]
async fn an_oversized_readme_is_truncated_and_flagged() {
    let repo = FakeRepo::arc();
    let svc = ReadmeService::new(Arc::clone(&repo) as Arc<dyn ReadmeRepository>);
    let small = ReadmeConfig {
        max_bytes: 8,
        ..cfg()
    };

    let meta = metadata_with(serde_json::json!({
        "readme": MetadataReadme::text("0123456789abcdef", ReadmeFormat::Markdown),
    }));
    svc.record_from_metadata(&meta, &small).await.unwrap();

    let row = repo.only();
    assert_eq!(row.content, "01234567");
    assert!(row.truncated);
    // The digest describes what is *stored*, so the render cache cannot serve a
    // rendering of the full text under a truncated row's key.
    assert_eq!(row.digest, readme_digest("01234567"));
}

/// npm's document-root README is attributed to the version `dist-tags.latest`
/// names, and the flag travels so the panel can say which it is showing.
#[tokio::test]
async fn a_package_level_readme_keeps_its_label() {
    let repo = FakeRepo::arc();
    let svc = ReadmeService::new(Arc::clone(&repo) as Arc<dyn ReadmeRepository>);

    let meta = metadata_with(serde_json::json!({
        "readme": MetadataReadme::text("# express", ReadmeFormat::Markdown).package_level(),
    }));
    svc.record_from_metadata(&meta, &cfg()).await.unwrap();
    assert!(repo.only().package_level);
}

#[tokio::test]
async fn publish_and_archive_captures_record_where_they_came_from() {
    let repo = FakeRepo::arc();
    let svc = ReadmeService::new(Arc::clone(&repo) as Arc<dyn ReadmeRepository>);

    svc.record_from_publish(
        "cargo1",
        "mylib",
        "1.0.0",
        "# mylib".into(),
        ReadmeFormat::Markdown,
        &cfg(),
    )
    .await
    .unwrap();
    assert_eq!(
        repo.get("cargo1", "mylib", "1.0.0")
            .await
            .unwrap()
            .unwrap()
            .source,
        ReadmeSource::LocalPublish
    );

    svc.record_from_archive(
        "cargo1",
        "other",
        "1.0.0",
        "from the crate".into(),
        ReadmeFormat::Plain,
        &cfg(),
    )
    .await
    .unwrap();
    assert_eq!(
        repo.get("cargo1", "other", "1.0.0")
            .await
            .unwrap()
            .unwrap()
            .source,
        ReadmeSource::Archive
    );
}

/// A `String` cannot hold half a code point, so a cap that cut mid-character
/// would be a panic triggered by publishing a README with an emoji in the wrong
/// place.
#[test]
fn truncation_lands_on_a_character_boundary() {
    // "aé" is 3 bytes: 'a' then a 2-byte 'é'.
    let (out, truncated) = truncate_to("aé", 2);
    assert_eq!(out, "a");
    assert!(truncated);

    let (out, truncated) = truncate_to("aé", 3);
    assert_eq!(out, "aé");
    assert!(!truncated);

    // A cap smaller than the first character yields nothing rather than panicking.
    let (out, truncated) = truncate_to("é", 1);
    assert_eq!(out, "");
    assert!(truncated);
}

#[test]
fn text_within_the_cap_is_untouched_and_unflagged() {
    let (out, truncated) = truncate_to("# hello", 4096);
    assert_eq!(out, "# hello");
    assert!(!truncated);
}

/// The `extra` channel round-trips through JSON, because that is how it travels
/// from a registry client to this service.
#[test]
fn the_extra_channel_round_trips() {
    let found = MetadataReadme::text("# hi", ReadmeFormat::Markdown).package_level();
    let extra = serde_json::json!({ "resolved_version": "1.0.0", "readme": found.clone() });
    assert_eq!(MetadataReadme::from_extra(&extra), Some(found));

    // An `extra` with no `readme` key, and a null `extra`, both answer "none".
    assert_eq!(
        MetadataReadme::from_extra(&serde_json::json!({ "x": 1 })),
        None
    );
    assert_eq!(MetadataReadme::from_extra(&serde_json::Value::Null), None);
}

// ── The fallback rule (RFC 0007 §4.4) ─────────────────────────────────────────

async fn seed(svc: &ReadmeService, version: &str, body: &str) {
    svc.record_from_publish(
        "npm1",
        "express",
        version,
        body.to_owned(),
        ReadmeFormat::Markdown,
        &cfg(),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn an_exact_hit_is_not_a_fallback() {
    let repo = FakeRepo::arc();
    let svc = ReadmeService::new(Arc::clone(&repo) as Arc<dyn ReadmeRepository>);
    seed(&svc, "4.18.2", "# 4.18.2").await;

    let answer = svc
        .get_for_version("npm1", "express", "4.18.2", &[])
        .await
        .unwrap()
        .expect("found");
    assert_eq!(answer.readme.content, "# 4.18.2");
    assert!(!answer.is_fallback);
}

/// A package whose 2.0.0-rc1 ships no README is better served by 1.4.2's than
/// by an empty panel — as long as the page says which it is showing.
#[tokio::test]
async fn a_version_without_one_falls_back_to_the_newest_that_has_one() {
    let repo = FakeRepo::arc();
    let svc = ReadmeService::new(Arc::clone(&repo) as Arc<dyn ReadmeRepository>);
    seed(&svc, "1.4.2", "# 1.4.2").await;

    let answer = svc
        .get_for_version("npm1", "express", "2.0.0-rc1", &[])
        .await
        .unwrap()
        .expect("fallback found");
    assert_eq!(answer.readme.version, "1.4.2");
    assert!(answer.is_fallback);
}

/// A blocked or unlisted version is not eligible as a fallback source: a
/// blocked version serves no README at all, and substituting one in silently
/// would route round the block.
#[tokio::test]
async fn an_ineligible_version_is_never_the_fallback() {
    let repo = FakeRepo::arc();
    let svc = ReadmeService::new(Arc::clone(&repo) as Arc<dyn ReadmeRepository>);
    seed(&svc, "1.0.0", "# fine").await;
    seed(&svc, "1.5.0", "# blocked").await;

    let answer = svc
        .get_for_version("npm1", "express", "2.0.0", &["1.5.0".to_owned()])
        .await
        .unwrap()
        .expect("fallback found");
    assert_eq!(answer.readme.version, "1.0.0");
    assert!(answer.is_fallback);
}

#[tokio::test]
async fn nothing_stored_anywhere_is_no_answer() {
    let repo = FakeRepo::arc();
    let svc = ReadmeService::new(Arc::clone(&repo) as Arc<dyn ReadmeRepository>);
    assert!(svc
        .get_for_version("npm1", "express", "1.0.0", &[])
        .await
        .unwrap()
        .is_none());
}

/// Asking for a version's own README is not a substitution, so the requested
/// version being in the exclusion list must not hide its own document.
#[tokio::test]
async fn the_requested_version_answers_for_itself_even_when_excluded() {
    let repo = FakeRepo::arc();
    let svc = ReadmeService::new(Arc::clone(&repo) as Arc<dyn ReadmeRepository>);
    seed(&svc, "1.0.0", "# mine").await;

    let answer = svc
        .get_for_version("npm1", "express", "1.0.0", &["1.0.0".to_owned()])
        .await
        .unwrap()
        .expect("its own README");
    assert!(!answer.is_fallback);
}

/// And it must not become its own fallback either: with nothing else stored,
/// a version that has none gets no answer rather than its own empty row.
#[tokio::test]
async fn a_version_is_never_its_own_fallback() {
    let repo = FakeRepo::arc();
    let svc = ReadmeService::new(Arc::clone(&repo) as Arc<dyn ReadmeRepository>);
    seed(&svc, "1.0.0", "# only this").await;

    let answer = svc
        .get_for_version("npm1", "express", "2.0.0", &[])
        .await
        .unwrap()
        .unwrap();
    assert_eq!(answer.readme.version, "1.0.0");
    assert!(answer.is_fallback);
}

// ── The render cache (RFC 0007 §5.3) ──────────────────────────────────────────

/// A `CacheStore` that counts what it is asked for, so the tests can assert on
/// hits rather than on timing.
#[derive(Default)]
struct CountingCache {
    entries: Mutex<std::collections::HashMap<String, crate::ports::CacheEntry>>,
    reads: Mutex<usize>,
    writes: Mutex<usize>,
}

#[async_trait]
impl crate::ports::CacheStore for CountingCache {
    async fn get(&self, key: &str) -> Result<Option<crate::ports::CacheEntry>, CoreError> {
        *self.reads.lock().unwrap() += 1;
        Ok(self.entries.lock().unwrap().get(key).cloned())
    }
    async fn set(
        &self,
        key: &str,
        entry: crate::ports::CacheEntry,
        _ttl: Option<std::time::Duration>,
    ) -> Result<(), CoreError> {
        *self.writes.lock().unwrap() += 1;
        self.entries.lock().unwrap().insert(key.to_owned(), entry);
        Ok(())
    }
    async fn invalidate(&self, key: &str) -> Result<(), CoreError> {
        self.entries.lock().unwrap().remove(key);
        Ok(())
    }
    async fn get_stale(&self, key: &str) -> Result<Option<crate::ports::CacheEntry>, CoreError> {
        Ok(self.entries.lock().unwrap().get(key).cloned())
    }
}

fn stored(version: &str, body: &str) -> PackageReadme {
    PackageReadme {
        registry: "npm1".into(),
        name: "express".into(),
        version: version.into(),
        content: body.into(),
        format: ReadmeFormat::Markdown,
        source: ReadmeSource::UpstreamMetadata,
        digest: readme_digest(body),
        truncated: false,
        package_level: false,
        extracted_at: Utc::now(),
    }
}

/// Two versions with an identical README — the common case for a patch release
/// — render once, because the cache is keyed by content and not by coordinate.
#[tokio::test]
async fn identical_text_renders_once_however_many_versions_carry_it() {
    let cache = Arc::new(CountingCache::default());
    let svc = ReadmeService::new(FakeRepo::arc() as Arc<dyn ReadmeRepository>)
        .with_cache(Arc::clone(&cache) as Arc<dyn crate::ports::CacheStore>);
    let opts = render::RenderOptions::default();

    let first = svc.render_cached(&stored("1.0.0", "# same"), &opts).await;
    let second = svc.render_cached(&stored("1.0.1", "# same"), &opts).await;

    assert_eq!(first, second);
    assert!(first.contains("<h1"), "{first}");
    assert_eq!(*cache.writes.lock().unwrap(), 1, "rendered twice");
}

/// A rendering made under one image policy must never be served to a registry
/// configured for the other — the output genuinely differs.
#[tokio::test]
async fn the_image_policy_is_part_of_the_cache_key() {
    let cache = Arc::new(CountingCache::default());
    let svc = ReadmeService::new(FakeRepo::arc() as Arc<dyn ReadmeRepository>)
        .with_cache(Arc::clone(&cache) as Arc<dyn crate::ports::CacheStore>);
    let row = stored("1.0.0", "![badge](https://img.shields.io/x.svg)");

    let stripped = svc
        .render_cached(&row, &render::RenderOptions::default())
        .await;
    let proxied = svc
        .render_cached(
            &row,
            &render::RenderOptions {
                remote_images: crate::services::RemoteImagePolicy::Proxy,
                image_proxy_prefix: Some("https://hub.example.com/i?url=".into()),
            },
        )
        .await;

    assert!(!stripped.contains("<img"), "{stripped}");
    assert!(proxied.contains("<img"), "{proxied}");
    assert_eq!(*cache.writes.lock().unwrap(), 2);
}

/// A service with no cache still renders. A missing cache costs a render per
/// read, not a wrong answer.
#[tokio::test]
async fn rendering_works_with_no_cache_at_all() {
    let svc = ReadmeService::new(FakeRepo::arc() as Arc<dyn ReadmeRepository>);
    let html = svc
        .render_cached(&stored("1.0.0", "# hi"), &render::RenderOptions::default())
        .await;
    assert!(html.contains("<h1"), "{html}");
}
