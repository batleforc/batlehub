use super::{CoreError, Identity, LocalRegistryService, PublishedPackage};

impl LocalRegistryService {
    /// Return all locally published versions of a NuGet package.
    pub async fn get_nuget_versions(
        &self,
        registry: &str,
        name: &str,
        identity: &Identity,
    ) -> Result<Vec<PublishedPackage>, CoreError> {
        self.load_visible_versions_or_not_found(registry, name, identity, "NuGet package")
            .await
    }
}
