#[cfg(feature = "sbom")]
pub mod extractor;
pub mod fetcher;

#[cfg(feature = "sbom")]
pub use extractor::ArchiveSbomExtractor;
pub use fetcher::HttpSbomFetcher;
