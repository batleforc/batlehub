pub mod backend;

use aws_config::BehaviorVersion;
use aws_sdk_s3::{config::Builder as S3ConfigBuilder, Client};

use batlehub_config::schema::S3StorageConfig;
use batlehub_core::error::CoreError;

pub struct S3StorageBackend {
    pub(super) client: Client,
    pub(super) bucket: String,
    pub(super) prefix: String,
}

impl S3StorageBackend {
    pub async fn new(cfg: &S3StorageConfig) -> Result<Self, CoreError> {
        let mut loader = aws_config::defaults(BehaviorVersion::latest())
            .region(aws_config::Region::new(cfg.region.clone()));

        if let Some(ref url) = cfg.endpoint_url {
            loader = loader.endpoint_url(url);
        }

        let sdk_config = loader.load().await;

        let mut s3_cfg_builder = S3ConfigBuilder::from(&sdk_config);
        if cfg.force_path_style.unwrap_or(false) {
            s3_cfg_builder = s3_cfg_builder.force_path_style(true);
        }

        let client = Client::from_conf(s3_cfg_builder.build());
        let prefix = cfg.prefix.clone().unwrap_or_default();

        tracing::info!(
            bucket = %cfg.bucket,
            region = %cfg.region,
            endpoint = ?cfg.endpoint_url,
            "S3 storage backend initialised"
        );

        client
            .head_bucket()
            .bucket(&cfg.bucket)
            .send()
            .await
            .map_err(|e| {
                CoreError::Storage(format!(
                    "S3 bucket '{}' is not reachable: {}",
                    cfg.bucket, e
                ))
            })?;

        tracing::info!(bucket = %cfg.bucket, "S3 bucket reachability check passed");

        Ok(Self {
            client,
            bucket: cfg.bucket.clone(),
            prefix,
        })
    }

    pub(super) fn object_key(&self, key: &str) -> Result<String, batlehub_core::error::CoreError> {
        crate::storage::ensure_safe_key(key)?;
        Ok(if self.prefix.is_empty() {
            key.to_owned()
        } else {
            format!("{}{}", self.prefix, key)
        })
    }

    pub(super) fn is_not_found(err: &aws_sdk_s3::Error) -> bool {
        matches!(
            err,
            aws_sdk_s3::Error::NoSuchKey(_) | aws_sdk_s3::Error::NotFound(_)
        )
    }
}

#[doc(hidden)]
impl S3StorageBackend {
    pub fn from_client(client: Client, bucket: String, prefix: String) -> Self {
        Self {
            client,
            bucket,
            prefix,
        }
    }
}
