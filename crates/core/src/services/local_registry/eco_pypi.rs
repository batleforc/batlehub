use super::{CoreError, Identity, LocalRegistryService};
use crate::services::escaping::{escape_html, percent_encode_path_segment};

impl LocalRegistryService {
    /// Build a PyPI Simple API HTML page listing all versions of `package_name`
    /// published in this local registry, formatted so `pip` can parse it.
    ///
    /// `base_url` is the registry's public base as seen by the requesting client
    /// (see [`LocalRegistryService::get_npm_packument`]).
    ///
    /// **Every interpolated value is escaped**, and the filename is
    /// percent-encoded before it becomes a path segment. None of the four is
    /// trustworthy: `filename` and `sha256` are read back from the publisher's
    /// `index_metadata`, `package_name` is only normalised (which does not
    /// remove `<`), and `base_url` derives from the request's `Host`. This
    /// document is served as `text/html` from the console's own origin, so an
    /// unescaped value here is stored XSS against an operator's session — and,
    /// because `pip` follows absolute `href`s out of a Simple index, an injected
    /// second anchor is an artifact-source substitution that bypasses the cache,
    /// the block list and the whole rule chain.
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
            let href = escape_html(&format!(
                "{}/packages/{}#sha256={}",
                base,
                percent_encode_path_segment(&filename),
                percent_encode_path_segment(sha256),
            ));
            let text = escape_html(&filename);
            links.push_str(&format!("    <a href=\"{href}\">{text}</a>\n"));
        }
        let package_name = escape_html(package_name);
        Ok(format!(
            "<!DOCTYPE html>\n<html>\n  <head><title>Links for {package_name}</title></head>\n  <body>\n    <h1>Links for {package_name}</h1>\n{links}  </body>\n</html>\n"
        ))
    }
}
