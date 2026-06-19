use async_trait::async_trait;
use aws_sdk_s3::types::{CompletedMultipartUpload, CompletedPart};
use bytes::{Bytes, BytesMut};
use futures::{stream, StreamExt};
use sha2::{Digest, Sha256};

use batlehub_core::{
    error::CoreError,
    ports::{ByteStream, StorageBackend, StorageMeta, StoreOutcome, StoredArtifact},
};

use super::{super::read_chunked, S3StorageBackend};

/// Multipart part size. Parts (except the last) must be ≥ 5 MiB per the S3 API;
/// 8 MiB keeps a comfortable margin while bounding peak memory to one part.
const PART_SIZE: usize = 8 * 1024 * 1024;

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

    /// Stream bytes to S3 while hashing them. Small objects go through a single
    /// `put_object`; once the buffered data crosses one part we switch to a
    /// multipart upload, flushing full parts as they accumulate so peak memory
    /// stays bounded to ~`PART_SIZE` regardless of artifact size.
    async fn store_streaming(
        &self,
        key: &str,
        mut stream: ByteStream,
        meta: StorageMeta,
    ) -> Result<StoreOutcome, CoreError> {
        let obj_key = self.object_key(key)?;
        let mut hasher = Sha256::new();
        let mut total: u64 = 0;
        let mut buf = BytesMut::with_capacity(PART_SIZE);

        let mut upload_id: Option<String> = None;
        let mut completed: Vec<CompletedPart> = Vec::new();
        let mut part_number = 1;

        // Helper closures can't borrow `self` across awaits cleanly, so the
        // multipart bookkeeping is inlined below.
        let result: Result<(), CoreError> = async {
            while let Some(chunk) = stream.next().await {
                let chunk = chunk?;
                hasher.update(&chunk);
                total += chunk.len() as u64;
                buf.extend_from_slice(&chunk);

                while buf.len() >= PART_SIZE {
                    if upload_id.is_none() {
                        let create = self
                            .client
                            .create_multipart_upload()
                            .bucket(&self.bucket)
                            .key(&obj_key)
                            .set_content_type(meta.content_type.clone())
                            .send()
                            .await
                            .map_err(|e| {
                                CoreError::Storage(format!("S3 create_multipart {obj_key}: {e}"))
                            })?;
                        upload_id = create.upload_id().map(str::to_owned);
                        if upload_id.is_none() {
                            return Err(CoreError::Storage(format!(
                                "S3 create_multipart {obj_key}: no upload id returned"
                            )));
                        }
                    }
                    let part = buf.split_to(PART_SIZE).freeze();
                    let resp = self
                        .client
                        .upload_part()
                        .bucket(&self.bucket)
                        .key(&obj_key)
                        .upload_id(upload_id.as_deref().unwrap())
                        .part_number(part_number)
                        .body(part.into())
                        .send()
                        .await
                        .map_err(|e| {
                            CoreError::Storage(format!(
                                "S3 upload_part {obj_key} #{part_number}: {e}"
                            ))
                        })?;
                    completed.push(
                        CompletedPart::builder()
                            .set_e_tag(resp.e_tag().map(str::to_owned))
                            .part_number(part_number)
                            .build(),
                    );
                    part_number += 1;
                }
            }

            match upload_id.as_deref() {
                // Whole object fit under one part: a single put is cheaper.
                None => {
                    let data = buf.freeze();
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
                }
                // Multipart in progress: flush the trailing remainder as the last
                // part (it may be < 5 MiB, which is allowed for the final part).
                Some(id) => {
                    if !buf.is_empty() {
                        let part = buf.split().freeze();
                        let resp = self
                            .client
                            .upload_part()
                            .bucket(&self.bucket)
                            .key(&obj_key)
                            .upload_id(id)
                            .part_number(part_number)
                            .body(part.into())
                            .send()
                            .await
                            .map_err(|e| {
                                CoreError::Storage(format!(
                                    "S3 upload_part {obj_key} #{part_number}: {e}"
                                ))
                            })?;
                        completed.push(
                            CompletedPart::builder()
                                .set_e_tag(resp.e_tag().map(str::to_owned))
                                .part_number(part_number)
                                .build(),
                        );
                    }
                    self.client
                        .complete_multipart_upload()
                        .bucket(&self.bucket)
                        .key(&obj_key)
                        .upload_id(id)
                        .multipart_upload(
                            CompletedMultipartUpload::builder()
                                .set_parts(Some(completed.clone()))
                                .build(),
                        )
                        .send()
                        .await
                        .map_err(|e| {
                            CoreError::Storage(format!("S3 complete_multipart {obj_key}: {e}"))
                        })?;
                }
            }
            Ok(())
        }
        .await;

        if let Err(e) = result {
            // Abort the multipart upload so we don't leak storage-billed parts.
            if let Some(id) = upload_id {
                let _ = self
                    .client
                    .abort_multipart_upload()
                    .bucket(&self.bucket)
                    .key(&obj_key)
                    .upload_id(id)
                    .send()
                    .await;
            }
            return Err(e);
        }

        tracing::debug!(key = %key, bytes = %total, "streamed artifact to S3");
        Ok(StoreOutcome {
            content_hash: hex::encode(hasher.finalize()),
            size: total,
        })
    }

    /// Server-side copy + delete — the object bytes never pass through us.
    async fn move_key(&self, from: &str, to: &str) -> Result<(), CoreError> {
        let from_key = self.object_key(from)?;
        let to_key = self.object_key(to)?;
        self.client
            .copy_object()
            .bucket(&self.bucket)
            .key(&to_key)
            .copy_source(format!("{}/{}", self.bucket, from_key))
            .send()
            .await
            .map_err(|e| {
                CoreError::Storage(format!("S3 copy_object {from_key} -> {to_key}: {e}"))
            })?;
        self.client
            .delete_object()
            .bucket(&self.bucket)
            .key(&from_key)
            .send()
            .await
            .map_err(|e| CoreError::Storage(format!("S3 delete_object {from_key}: {e}")))?;
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

        // Stream the body back in fixed chunks instead of buffering it whole. A
        // zero-length object still yields exactly one (empty) chunk so consumers
        // that expect at least one item behave as they did before streaming.
        let stream: ByteStream = if size == Some(0) {
            Box::pin(stream::once(async { Ok(Bytes::new()) }))
        } else {
            read_chunked(resp.body.into_async_read(), obj_key.clone())
        };

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
