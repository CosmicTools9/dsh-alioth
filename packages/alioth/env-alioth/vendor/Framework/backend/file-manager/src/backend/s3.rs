//! S3-compatible object storage backend.

use async_trait::async_trait;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::Client as S3Client;
use std::time::Duration;

use super::StorageBackend;
use crate::error::FileError;

/// Configuration for S3-compatible storage.
#[derive(Debug, Clone)]
pub struct S3Config {
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    pub access_key: String,
    pub secret_key: String,
    pub cdn_domain: Option<String>,
    /// path-style 寻址（`{endpoint}/{bucket}/{key}`）：MinIO/Ceph/自建 S3 必须；
    /// 云厂商默认 virtual-hosted（`{bucket}.{endpoint}`）。minio provider 默认 true。
    pub force_path_style: bool,
}

/// S3-compatible storage backend using `aws-sdk-s3`.
pub struct S3Backend {
    client: S3Client,
    bucket: String,
    cdn_domain: Option<String>,
}

impl S3Backend {
    /// Create a new S3Backend from configuration.
    pub async fn new(config: &S3Config) -> Result<Self, FileError> {
        use aws_sdk_s3::config::{BehaviorVersion, Credentials, Region};

        let credentials = Credentials::new(
            &config.access_key,
            &config.secret_key,
            None,
            None,
            "file-manager",
        );

        let shared_config = aws_config::defaults(BehaviorVersion::latest())
            .region(Region::new(config.region.clone()))
            .credentials_provider(credentials)
            .endpoint_url(&config.endpoint)
            .load()
            .await;

        let client = S3Client::from_conf(
            aws_sdk_s3::config::Builder::from(&shared_config)
                .force_path_style(config.force_path_style)
                .build(),
        );

        Ok(Self {
            client,
            bucket: config.bucket.clone(),
            cdn_domain: config.cdn_domain.clone(),
        })
    }

    fn validate_key(key: &str) -> Result<(), FileError> {
        if key.is_empty() || key.contains("..") {
            return Err(FileError::InvalidKey(key.to_string()));
        }
        Ok(())
    }
}

#[async_trait]
impl StorageBackend for S3Backend {
    async fn put(&self, key: &str, data: Vec<u8>, content_type: &str) -> Result<String, FileError> {
        Self::validate_key(key)?;

        let body = ByteStream::from(data);
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .body(body)
            .content_type(content_type)
            .send()
            .await
            .map_err(|e| FileError::StorageBackend(format!("S3 put: {}", e)))?;

        Ok(key.to_string())
    }

    async fn get(&self, key: &str) -> Result<Vec<u8>, FileError> {
        Self::validate_key(key)?;

        let output = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(|e| FileError::StorageBackend(format!("S3 get: {}", e)))?;

        let data = output
            .body
            .collect()
            .await
            .map_err(|e| FileError::StorageBackend(format!("S3 collect body: {}", e)))?
            .into_bytes()
            .to_vec();

        Ok(data)
    }

    async fn get_range(&self, key: &str, start: u64, end: u64) -> Result<Vec<u8>, FileError> {
        Self::validate_key(key)?;

        // S3 原生 range 参数：服务端裁剪，不整读
        let output = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .range(format!("bytes={start}-{end}"))
            .send()
            .await
            .map_err(|e| FileError::StorageBackend(format!("S3 get_range: {}", e)))?;

        let data = output
            .body
            .collect()
            .await
            .map_err(|e| FileError::StorageBackend(format!("S3 collect range body: {}", e)))?
            .into_bytes()
            .to_vec();

        Ok(data)
    }

    async fn move_object(&self, from: &str, to: &str) -> Result<(), FileError> {
        Self::validate_key(from)?;
        Self::validate_key(to)?;

        self.client
            .copy_object()
            .bucket(&self.bucket)
            .copy_source(format!("{}/{}", self.bucket, from))
            .key(to)
            .send()
            .await
            .map_err(|e| FileError::StorageBackend(format!("S3 copy: {}", e)))?;

        self.delete(from).await
    }

    async fn delete(&self, key: &str) -> Result<(), FileError> {
        Self::validate_key(key)?;

        self.client
            .delete_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(|e| FileError::StorageBackend(format!("S3 delete: {}", e)))?;

        Ok(())
    }

    async fn presigned_url(&self, key: &str, expires_in_secs: u64) -> Result<String, FileError> {
        Self::validate_key(key)?;

        // Use CDN domain if configured
        if let Some(cdn) = &self.cdn_domain {
            return Ok(format!("{}/{}", cdn.trim_end_matches('/'), key));
        }

        use aws_sdk_s3::presigning::PresigningConfig;
        let presign_config = PresigningConfig::builder()
            .expires_in(Duration::from_secs(expires_in_secs))
            .build()
            .map_err(|e| FileError::StorageBackend(format!("Presign config: {}", e)))?;

        let req = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .presigned(presign_config)
            .await
            .map_err(|e| FileError::StorageBackend(format!("S3 presign: {}", e)))?;

        Ok(req.uri().to_string())
    }
}
