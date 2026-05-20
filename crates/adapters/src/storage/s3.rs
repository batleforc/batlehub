use async_trait::async_trait;
use aws_config::BehaviorVersion;
use aws_sdk_s3::{Client, config::Builder as S3ConfigBuilder};
use bytes::Bytes;
use futures::stream;

use proxy_cache_config::schema::S3StorageConfig;
use proxy_cache_core::{
    error::CoreError,
    ports::{ByteStream, StorageBackend, StorageMeta, StoredArtifact},
};

pub struct S3StorageBackend {
    client: Client,
    bucket: String,
    prefix: String,
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

        Ok(Self { client, bucket: cfg.bucket.clone(), prefix })
    }

    fn object_key(&self, key: &str) -> String {
        if self.prefix.is_empty() {
            key.to_owned()
        } else {
            format!("{}{}", self.prefix, key)
        }
    }

    fn is_not_found(err: &aws_sdk_s3::Error) -> bool {
        matches!(
            err,
            aws_sdk_s3::Error::NoSuchKey(_) | aws_sdk_s3::Error::NotFound(_)
        )
    }
}

#[doc(hidden)]
impl S3StorageBackend {
    pub fn from_client(client: Client, bucket: String, prefix: String) -> Self {
        Self { client, bucket, prefix }
    }
}

#[async_trait]
impl StorageBackend for S3StorageBackend {
    async fn store(&self, key: &str, data: Bytes, meta: StorageMeta) -> Result<(), CoreError> {
        let obj_key = self.object_key(key);
        let len = data.len() as i64;

        let mut req = self
            .client
            .put_object()
            .bucket(&self.bucket)
            .key(&obj_key)
            .body(data.into())
            .content_length(len);

        if let Some(ref ct) = meta.content_type {
            req = req.content_type(ct);
        }

        req.send()
            .await
            .map_err(|e| CoreError::Storage(format!("S3 put_object {obj_key}: {e}")))?;

        tracing::debug!(key = %key, bytes = %len, "stored artifact in S3");
        Ok(())
    }

    async fn retrieve(&self, key: &str) -> Result<Option<StoredArtifact>, CoreError> {
        let obj_key = self.object_key(key);

        let resp = match self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(&obj_key)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                let sdk_err = e.into_service_error();
                let err_str = sdk_err.to_string();
                if Self::is_not_found(&sdk_err.into()) {
                    return Ok(None);
                }
                return Err(CoreError::Storage(format!("S3 get_object {obj_key}: {err_str}")));
            }
        };

        let size = resp.content_length().map(|n| n as u64);
        let content_type = resp.content_type().map(str::to_owned);

        let bytes = resp
            .body
            .collect()
            .await
            .map(|agg| agg.into_bytes())
            .map_err(|e| CoreError::Storage(format!("S3 read body {obj_key}: {e}")))?;

        let stream: ByteStream = Box::pin(stream::once(async move { Ok(bytes) }));

        Ok(Some(StoredArtifact {
            stream,
            meta: StorageMeta { size, content_type, ..Default::default() },
        }))
    }

    async fn exists(&self, key: &str) -> Result<bool, CoreError> {
        let obj_key = self.object_key(key);

        match self
            .client
            .head_object()
            .bucket(&self.bucket)
            .key(&obj_key)
            .send()
            .await
        {
            Ok(_) => Ok(true),
            Err(e) => {
                let sdk_err = e.into_service_error();
                let err_str = sdk_err.to_string();
                if Self::is_not_found(&sdk_err.into()) {
                    return Ok(false);
                }
                Err(CoreError::Storage(format!("S3 head_object {obj_key}: {err_str}")))
            }
        }
    }

    async fn delete(&self, key: &str) -> Result<(), CoreError> {
        let obj_key = self.object_key(key);

        match self
            .client
            .delete_object()
            .bucket(&self.bucket)
            .key(&obj_key)
            .send()
            .await
        {
            Ok(_) => Ok(()),
            Err(e) => {
                let sdk_err = e.into_service_error();
                let err_str = sdk_err.to_string();
                // NoSuchKey on delete is fine
                if Self::is_not_found(&sdk_err.into()) {
                    return Ok(());
                }
                Err(CoreError::Storage(format!("S3 delete_object {obj_key}: {err_str}")))
            }
        }
    }

    async fn stat_by_prefix(&self, prefix: &str) -> Result<(u64, u64), CoreError> {
        let s3_prefix = self.object_key(prefix);
        let mut count = 0u64;
        let mut total_bytes = 0u64;
        let mut continuation_token: Option<String> = None;

        loop {
            let mut req = self
                .client
                .list_objects_v2()
                .bucket(&self.bucket)
                .prefix(&s3_prefix);
            if let Some(ref token) = continuation_token {
                req = req.continuation_token(token);
            }
            let resp = req
                .send()
                .await
                .map_err(|e| CoreError::Storage(format!("S3 list_objects {s3_prefix}: {e}")))?;

            for obj in resp.contents() {
                count += 1;
                total_bytes += obj.size().unwrap_or(0) as u64;
            }

            let is_truncated = resp.is_truncated().unwrap_or(false);
            continuation_token = resp.next_continuation_token().map(str::to_owned);
            if !is_truncated {
                break;
            }
        }

        Ok((count, total_bytes))
    }

    async fn delete_by_prefix(&self, prefix: &str) -> Result<usize, CoreError> {
        use aws_sdk_s3::types::{Delete, ObjectIdentifier};

        let s3_prefix = self.object_key(prefix);
        let configured_prefix_len = self.prefix.len();
        let mut total = 0usize;
        let mut continuation_token: Option<String> = None;

        loop {
            let mut req = self
                .client
                .list_objects_v2()
                .bucket(&self.bucket)
                .prefix(&s3_prefix);
            if let Some(ref token) = continuation_token {
                req = req.continuation_token(token);
            }
            let resp = req
                .send()
                .await
                .map_err(|e| CoreError::Storage(format!("S3 list_objects {s3_prefix}: {e}")))?;

            let object_keys: Vec<ObjectIdentifier> = resp
                .contents()
                .iter()
                .filter_map(|o| o.key())
                .filter_map(|k| {
                    ObjectIdentifier::builder()
                        .key(k[configured_prefix_len..].to_owned())
                        .build()
                        .ok()
                })
                .collect();

            let batch_len = object_keys.len();
            if batch_len > 0 {
                let delete = Delete::builder()
                    .set_objects(Some(object_keys))
                    .build()
                    .map_err(|e| CoreError::Storage(format!("S3 delete build: {e}")))?;
                self.client
                    .delete_objects()
                    .bucket(&self.bucket)
                    .delete(delete)
                    .send()
                    .await
                    .map_err(|e| CoreError::Storage(format!("S3 delete_objects: {e}")))?;
                total += batch_len;
            }

            let is_truncated = resp.is_truncated().unwrap_or(false);
            continuation_token = resp.next_continuation_token().map(str::to_owned);
            if !is_truncated {
                break;
            }
        }

        Ok(total)
    }
}
