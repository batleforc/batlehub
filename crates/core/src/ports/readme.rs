use async_trait::async_trait;

use crate::entities::PackageReadme;
use crate::error::CoreError;

/// Durable storage for the README of a version this instance **holds bytes for
/// or hosts**.
///
/// Nothing here holds a row for a version the instance only knows about from an
/// upstream document: that answer is derived on read from the cached document
/// (RFC 0007 §5.6). A row written because somebody looked at a page would have
/// nothing that ever deletes it — deletion keys on a version being deleted, and
/// a version never held here is never deleted.
#[async_trait]
pub trait ReadmeRepository: Send + Sync {
    /// Store or replace the README for one coordinate.
    async fn upsert(&self, readme: PackageReadme) -> Result<(), CoreError>;

    /// The README for an exact coordinate, or `None` when none is stored.
    async fn get(
        &self,
        registry: &str,
        name: &str,
        version: &str,
    ) -> Result<Option<PackageReadme>, CoreError>;

    /// The newest version of this package that has a README, excluding the
    /// versions the caller names.
    ///
    /// `exclude_versions` is how the fallback rule keeps blocked and unlisted
    /// versions from becoming a fallback source (RFC 0007 §4.4): the repository
    /// knows nothing about firewall state, so the caller — which does — hands
    /// the exclusions down rather than the store growing a policy opinion.
    ///
    /// "Newest" is decided by the caller's ordering, not the store's: version
    /// strings do not sort as versions in SQL. The store returns candidates
    /// newest-*recorded* first and the service picks.
    async fn get_latest_with_readme(
        &self,
        registry: &str,
        name: &str,
        exclude_versions: &[String],
    ) -> Result<Option<PackageReadme>, CoreError>;

    /// One `(version, state)` pair per version of this package that has a
    /// stored README.
    ///
    /// One query for the whole version table, rather than a probe per row: the
    /// detail response carries a `readme` state for every version it lists, and
    /// a lookup per version would be N round trips for a page load.
    ///
    /// Versions absent from the result have no stored README; whether that is
    /// [`ReadmeState::None`] or [`ReadmeState::Unknown`] is a question about the
    /// registry kind and what bytes are held, which the caller answers.
    async fn list_versions_with_readme(
        &self,
        registry: &str,
        name: &str,
    ) -> Result<Vec<String>, CoreError>;

    /// Remove the README for one version, on local delete.
    async fn delete_for_version(
        &self,
        registry: &str,
        name: &str,
        version: &str,
    ) -> Result<(), CoreError>;

    /// Remove every README for a package, on package delete.
    async fn delete_for_package(&self, registry: &str, name: &str) -> Result<(), CoreError>;

    /// Packages whose README matches `query`, best first (RFC 0007-bis §4.3).
    ///
    /// On the port rather than in a service because the query is Postgres
    /// full-text search — `websearch_to_tsquery`, `ts_rank_cd`, `ts_headline` —
    /// and there is no way to express that once for two backends. The in-memory
    /// implementation answers with a substring match, which is the honest
    /// double: the tests that matter for *ranking* run against real Postgres
    /// (`pg_readmes.rs`), and the in-memory one exists so the web suite can
    /// exercise the endpoint's shape.
    ///
    /// `registries` is the caller's accessible set, applied here rather than
    /// after: a search must not read rows it will then have to discard, or the
    /// `limit` would silently mean something different for different callers.
    ///
    /// One row per package, not per version. A reader searching for a phrase
    /// wants the package that says it, and a package whose README repeats across
    /// forty patch releases would otherwise fill the page by itself.
    async fn search(
        &self,
        registries: &[String],
        query: &str,
        limit: u64,
    ) -> Result<Vec<ReadmeSearchHit>, CoreError>;
}

/// One package whose README matched a search.
#[derive(Debug, Clone, PartialEq)]
pub struct ReadmeSearchHit {
    pub registry: String,
    pub name: String,
    /// The version whose README matched best. Shown nowhere yet; kept because
    /// "which version says this" is the obvious next question and discarding it
    /// here would mean a second query to answer it.
    pub version: String,
    /// The matched fragment, as **plain text**.
    ///
    /// Plain because it is rendered as text and must never reach `v-html`: the
    /// README panel's is a deliberate, tested, single-component boundary
    /// (RFC 0007 §6.5), and a search snippet would be a *second* place where
    /// package-authored content reaches the page, reached by a much cheaper path
    /// — no navigation, just a query (RFC 0007-bis §7.4).
    pub snippet: String,
    /// `ts_rank_cd`, or a stand-in from the in-memory double. Comparable within
    /// one result set and meaningless across two.
    pub rank: f32,
}

/// Fetching one image a stored README pointed at (RFC 0007-bis §5.1).
///
/// A port rather than a direct call into the HTTP client for the ordinary
/// reason — `core` owns no `reqwest` — and for one specific to this feature: the
/// implementation's first guard is `ssrf::ensure_public_url`, which refuses
/// loopback, so an in-process test against a local mock server would be refused
/// by the very thing it is trying to exercise. The endpoint's behaviour is
/// tested through a fake here; the guard itself is tested where it lives.
///
/// **The caller never supplies the URL.** It comes out of a README this instance
/// stored, resolved by index, which is what stops this being an open image
/// proxy — see [`crate::services::readme::render::image_urls`].
#[async_trait]
pub trait ReadmeImageFetcher: Send + Sync {
    /// The image at `url`, or `None` when there simply is not one.
    ///
    /// `Ok(None)` covers every ordinary miss: a `404`, a `Content-Type` that is
    /// not an allow-listed image, a body over `max_bytes`. All of them end the
    /// same way for the reader — the chip the `strip` policy would have shown —
    /// so none of them is an error. `Err` is for a request that could not be
    /// made or a connection that broke mid-body.
    async fn fetch(
        &self,
        url: &str,
        max_bytes: usize,
    ) -> Result<Option<crate::services::readme::image::FetchedImage>, CoreError>;
}
