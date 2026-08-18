use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use batlehub_core::{entities::PackageReadme, error::CoreError, ports::ReadmeRepository};

/// No-op README repository — used where READMEs are not wanted at all.
pub struct NoopReadmeRepository;

impl NoopReadmeRepository {
    pub fn arc() -> Arc<dyn ReadmeRepository> {
        Arc::new(Self)
    }
}

#[async_trait]
impl ReadmeRepository for NoopReadmeRepository {
    async fn upsert(&self, _readme: PackageReadme) -> Result<(), CoreError> {
        Ok(())
    }

    async fn get(
        &self,
        _registry: &str,
        _name: &str,
        _version: &str,
    ) -> Result<Option<PackageReadme>, CoreError> {
        Ok(None)
    }

    async fn get_latest_with_readme(
        &self,
        _registry: &str,
        _name: &str,
        _exclude_versions: &[String],
    ) -> Result<Option<PackageReadme>, CoreError> {
        Ok(None)
    }

    async fn list_versions_with_readme(
        &self,
        _registry: &str,
        _name: &str,
    ) -> Result<Vec<String>, CoreError> {
        Ok(vec![])
    }

    async fn delete_for_version(
        &self,
        _registry: &str,
        _name: &str,
        _version: &str,
    ) -> Result<(), CoreError> {
        Ok(())
    }

    async fn delete_for_package(&self, _registry: &str, _name: &str) -> Result<(), CoreError> {
        Ok(())
    }
}

/// In-memory README repository for tests.
///
/// Mirrors the Postgres adapter's semantics, including the one that is easy to
/// get wrong: `get_latest_with_readme` orders by `extracted_at`, not by version
/// string, so a test that writes two rows in a known order gets a deterministic
/// answer without this store inventing a version ordering the real one does not
/// have.
pub struct InMemoryReadmeRepository {
    items: Mutex<Vec<PackageReadme>>,
}

impl InMemoryReadmeRepository {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            items: Mutex::new(Vec::new()),
        })
    }

    /// Every stored row, for tests that assert on what a page view did *not*
    /// write (RFC 0007 §4.4).
    pub fn all(&self) -> Vec<PackageReadme> {
        self.items.lock().unwrap().clone()
    }
}

#[async_trait]
impl ReadmeRepository for InMemoryReadmeRepository {
    async fn upsert(&self, readme: PackageReadme) -> Result<(), CoreError> {
        let mut items = self.items.lock().unwrap();
        items.retain(|r| {
            !(r.registry == readme.registry && r.name == readme.name && r.version == readme.version)
        });
        items.push(readme);
        Ok(())
    }

    async fn get(
        &self,
        registry: &str,
        name: &str,
        version: &str,
    ) -> Result<Option<PackageReadme>, CoreError> {
        Ok(self
            .items
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
        let items = self.items.lock().unwrap();
        let mut candidates: Vec<&PackageReadme> = items
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
            .items
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
        self.items
            .lock()
            .unwrap()
            .retain(|r| !(r.registry == registry && r.name == name && r.version == version));
        Ok(())
    }

    async fn delete_for_package(&self, registry: &str, name: &str) -> Result<(), CoreError> {
        self.items
            .lock()
            .unwrap()
            .retain(|r| !(r.registry == registry && r.name == name));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use batlehub_core::entities::{readme_digest, ReadmeFormat, ReadmeSource};
    use chrono::{Duration, Utc};

    fn readme(registry: &str, name: &str, version: &str, body: &str) -> PackageReadme {
        PackageReadme {
            registry: registry.into(),
            name: name.into(),
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

    #[tokio::test]
    async fn noop_repo_stores_nothing() {
        let repo = NoopReadmeRepository::arc();
        repo.upsert(readme("npm", "p", "1.0.0", "# hi"))
            .await
            .unwrap();
        assert!(repo.get("npm", "p", "1.0.0").await.unwrap().is_none());
        assert!(repo
            .list_versions_with_readme("npm", "p")
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn upsert_replaces_the_same_coordinate_only() {
        let repo = InMemoryReadmeRepository::new();
        repo.upsert(readme("npm", "p", "1.0.0", "one"))
            .await
            .unwrap();
        repo.upsert(readme("npm", "p", "1.0.0", "two"))
            .await
            .unwrap();
        repo.upsert(readme("npm", "p", "2.0.0", "three"))
            .await
            .unwrap();

        assert_eq!(
            repo.get("npm", "p", "1.0.0")
                .await
                .unwrap()
                .unwrap()
                .content,
            "two"
        );
        let mut versions = repo.list_versions_with_readme("npm", "p").await.unwrap();
        versions.sort();
        assert_eq!(versions, ["1.0.0", "2.0.0"]);
    }

    /// Each version keeps its own text. This is the whole point of the version
    /// key: a package-level store would show 2.x's README to a 1.x reader.
    #[tokio::test]
    async fn each_version_keeps_its_own_readme() {
        let repo = InMemoryReadmeRepository::new();
        repo.upsert(readme("npm", "p", "1.0.0", "the 1.x API"))
            .await
            .unwrap();
        repo.upsert(readme("npm", "p", "2.0.0", "the 2.x API"))
            .await
            .unwrap();
        assert_eq!(
            repo.get("npm", "p", "1.0.0")
                .await
                .unwrap()
                .unwrap()
                .content,
            "the 1.x API"
        );
        assert_eq!(
            repo.get("npm", "p", "2.0.0")
                .await
                .unwrap()
                .unwrap()
                .content,
            "the 2.x API"
        );
    }

    #[tokio::test]
    async fn latest_skips_the_versions_the_caller_excludes() {
        let repo = InMemoryReadmeRepository::new();
        let older = PackageReadme {
            extracted_at: Utc::now() - Duration::hours(1),
            ..readme("npm", "p", "1.0.0", "old")
        };
        repo.upsert(older).await.unwrap();
        repo.upsert(readme("npm", "p", "2.0.0", "new"))
            .await
            .unwrap();

        assert_eq!(
            repo.get_latest_with_readme("npm", "p", &[])
                .await
                .unwrap()
                .unwrap()
                .version,
            "2.0.0"
        );
        // A blocked or unlisted version is not eligible as a fallback source,
        // and the exclusion is the caller's to make.
        assert_eq!(
            repo.get_latest_with_readme("npm", "p", &["2.0.0".to_owned()])
                .await
                .unwrap()
                .unwrap()
                .version,
            "1.0.0"
        );
        assert!(repo
            .get_latest_with_readme("npm", "p", &["1.0.0".to_owned(), "2.0.0".to_owned()])
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn delete_is_scoped_to_what_it_names() {
        let repo = InMemoryReadmeRepository::new();
        repo.upsert(readme("npm", "p", "1.0.0", "a")).await.unwrap();
        repo.upsert(readme("npm", "p", "2.0.0", "b")).await.unwrap();
        repo.upsert(readme("npm", "q", "1.0.0", "c")).await.unwrap();

        repo.delete_for_version("npm", "p", "1.0.0").await.unwrap();
        assert!(repo.get("npm", "p", "1.0.0").await.unwrap().is_none());
        assert!(repo.get("npm", "p", "2.0.0").await.unwrap().is_some());

        repo.delete_for_package("npm", "p").await.unwrap();
        assert!(repo
            .list_versions_with_readme("npm", "p")
            .await
            .unwrap()
            .is_empty());
        // Another package's rows are untouched.
        assert!(repo.get("npm", "q", "1.0.0").await.unwrap().is_some());
    }
}
