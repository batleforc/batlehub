use super::{CoreError, Identity, LocalRegistryService};

impl LocalRegistryService {
    /// Build a PyPI Simple API HTML page listing all versions of `package_name`
    /// published in this local registry, formatted so `pip` can parse it.
    pub async fn get_pypi_simple_page(
        &self,
        registry: &str,
        package_name: &str,
        base_url: &str,
        identity: &Identity,
    ) -> Result<String, CoreError> {
        let versions = self
            .load_visible_versions_or_not_found(registry, package_name, identity, "pypi package")
            .await?;
        let base = base_url.trim_end_matches('/');
        let mut links = String::new();
        for pkg in &versions {
            let filename = pkg
                .index_metadata
                .get("filename")
                .and_then(|v| v.as_str())
                .map(str::to_owned)
                .unwrap_or_else(|| format!("{}-{}.tar.gz", pkg.name, pkg.version));
            let sha256 = pkg
                .index_metadata
                .get("sha256")
                .and_then(|v| v.as_str())
                .unwrap_or(&pkg.checksum);
            let url = format!("{base}/proxy/{registry}/packages/{filename}#sha256={sha256}");
            links.push_str(&format!("    <a href=\"{url}\">{filename}</a>\n"));
        }
        Ok(format!(
            "<!DOCTYPE html>\n<html>\n  <head><title>Links for {package_name}</title></head>\n  <body>\n    <h1>Links for {package_name}</h1>\n{links}  </body>\n</html>\n"
        ))
    }
}
