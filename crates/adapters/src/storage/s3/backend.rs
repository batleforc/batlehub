use async_trait::async_trait;
use bytes::Bytes;
use futures::stream;

use batlehub_core::{
    error::CoreError,
    ports::{ByteStream, StorageBackend, StorageMeta, StoredArtifact},
};

use super::S3StorageBackend;

#[async_trait]
impl StorageBackend for S3StorageBackend {
    async fn store(&self, key: &str, data: Bytes, meta: StorageMeta) -> Result<(), CoreError> {
        let obj_key = self.object_key(key)?;
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
        let obj_key = self.object_key(key)?;

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
                return Err(CoreError::Storage(format!(
                    "S3 get_object {obj_key}: {err_str}"
                )));
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
            meta: StorageMeta {
                size,
                content_type,
                ..Default::default()
            },
        }))
    }

    async fn exists(&self, key: &str) -> Result<bool, CoreError> {
        let obj_key = self.object_key(key)?;

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
                Err(CoreError::Storage(format!(
                    "S3 head_object {obj_key}: {err_str}"
                )))
            }
        }
    }

    async fn delete(&self, key: &str) -> Result<(), CoreError> {
        let obj_key = self.object_key(key)?;

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
                Err(CoreError::Storage(format!(
                    "S3 delete_object {obj_key}: {err_str}"
                )))
            }
        }
    }

    async fn stat_by_prefix(&self, prefix: &str) -> Result<(u64, u64), CoreError> {
        let s3_prefix = self.object_key(prefix)?;
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

    async fn list_keys(&self, prefix: &str) -> Result<Vec<String>, CoreError> {
        let s3_prefix = self.object_key(prefix)?;
        let prefix_strip_len = self.prefix.len();
        let mut keys = Vec::new();
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
                if let Some(k) = obj.key() {
                    // Strip the backend prefix to get the logical key.
                    keys.push(k[prefix_strip_len..].to_owned());
                }
            }

            let is_truncated = resp.is_truncated().unwrap_or(false);
            continuation_token = resp.next_continuation_token().map(str::to_owned);
            if !is_truncated {
                break;
            }
        }
        Ok(keys)
    }

    async fn delete_by_prefix(&self, prefix: &str) -> Result<usize, CoreError> {
        use aws_sdk_s3::types::{Delete, ObjectIdentifier};

        let s3_prefix = self.object_key(prefix)?;
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
