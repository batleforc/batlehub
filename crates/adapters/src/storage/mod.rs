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
