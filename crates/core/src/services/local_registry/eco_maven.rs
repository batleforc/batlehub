use super::{CoreError, Identity, LocalRegistryService, PublishedPackage};

impl LocalRegistryService {
    /// Return all non-empty published versions for a Maven artifact (`groupId:artifactId`).
    /// Returns `CoreError::NotFound` when none are published.
    pub async fn get_maven_versions(
        &self,
        registry: &str,
        name: &str,
        identity: &Identity,
    ) -> Result<Vec<PublishedPackage>, CoreError> {
        self.load_visible_versions_or_not_found(registry, name, identity, "artifact")
            .await
    }
}
