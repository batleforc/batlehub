#[cfg(feature = "storage-fs")]
pub mod filesystem;

#[cfg(feature = "storage-fs")]
pub use filesystem::FilesystemStorageBackend;

#[cfg(feature = "storage-s3")]
pub mod s3;

#[cfg(feature = "storage-s3")]
pub use s3::S3StorageBackend;

pub mod router;
pub use router::StorageRouter;

/// Rejects storage keys that could escape the storage root via path traversal.
///
/// Storage keys are built from untrusted package coordinates
/// (`{registry}/{name}/{version}[/…]`). Names legitimately contain `/`
/// (npm scopes like `@scope/name`, GitHub `owner/repo`), so `/` is allowed — but
/// a `..` path segment, an absolute (leading-`/`) key, a backslash, or a NUL byte
/// are not. This is the single chokepoint every storage backend funnels through,
/// so it protects all registry adapters regardless of their own input validation.
#[cfg(any(feature = "storage-fs", feature = "storage-s3"))]
pub(crate) fn ensure_safe_key(key: &str) -> Result<(), batlehub_core::error::CoreError> {
    use batlehub_core::error::CoreError;
    if key.is_empty() {
        return Err(CoreError::InvalidInput("empty storage key".into()));
    }
    if key.contains('\0') || key.contains('\\') {
        return Err(CoreError::InvalidInput(format!(
            "storage key {key:?} contains an illegal character"
        )));
    }
    if key.starts_with('/') {
        return Err(CoreError::InvalidInput(format!(
            "storage key {key:?} must not be absolute"
        )));
    }
    if key.split('/').any(|segment| segment == "..") {
        return Err(CoreError::InvalidInput(format!(
            "storage key {key:?} contains a path-traversal segment"
        )));
    }
    Ok(())
}

#[cfg(all(test, any(feature = "storage-fs", feature = "storage-s3")))]
mod ensure_safe_key_tests {
    use super::ensure_safe_key;

    #[test]
    fn accepts_legitimate_keys() {
        for key in [
            "artifact:npm/@scope/name/1.2.3",
            "local:maven/com.example:lib/1.0.0/lib.jar",
            "cargo/tokio/1.38.0",
        ] {
            assert!(ensure_safe_key(key).is_ok(), "should accept {key}");
        }
    }

    #[test]
    fn rejects_traversal_and_absolute_keys() {
        for key in [
            "",
            "local:maven/../../../../etc/passwd/1.0",
            "/etc/passwd",
            "..",
            "a/../b",
            "a/..",
            "a\\..\\b",
            "with\0nul",
        ] {
            assert!(ensure_safe_key(key).is_err(), "should reject {key:?}");
        }
    }
}
