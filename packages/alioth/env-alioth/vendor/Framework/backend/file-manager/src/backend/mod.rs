//! Storage backend abstraction.

use async_trait::async_trait;

use crate::error::FileError;

/// Abstract storage backend for file operations.
#[async_trait]
pub trait StorageBackend: Send + Sync {
    /// Store raw bytes. Returns the storage key.
    async fn put(&self, key: &str, data: Vec<u8>, content_type: &str) -> Result<String, FileError>;

    /// Retrieve file bytes by storage key.
    async fn get(&self, key: &str) -> Result<Vec<u8>, FileError>;

    /// Retrieve byte range `[start, end]` (inclusive) by storage key.
    /// 默认实现：整读 + 切片（正确但低效）；S3 用服务端 range、local 用文件 seek 覆盖。
    async fn get_range(&self, key: &str, start: u64, end: u64) -> Result<Vec<u8>, FileError> {
        let data = self.get(key).await?;
        let start = (start as usize).min(data.len());
        let end = (end as usize).min(data.len());
        Ok(data[start..=end].to_vec())
    }

    /// Move (rename) an object from `from` to `to`。默认实现 get+put+delete；
    /// local 用 fs::rename、S3 用 copy+delete 覆盖。
    async fn move_object(&self, from: &str, to: &str) -> Result<(), FileError> {
        let data = self.get(from).await?;
        let content_type = "application/octet-stream";
        self.put(to, data, content_type).await?;
        self.delete(from).await
    }

    /// Delete file by storage key.
    async fn delete(&self, key: &str) -> Result<(), FileError>;

    /// Generate a presigned URL for temporary access (expires_in_secs).
    async fn presigned_url(&self, key: &str, expires_in_secs: u64) -> Result<String, FileError>;
}

mod local;
pub use local::LocalBackend;

#[cfg(feature = "s3")]
mod s3;
#[cfg(feature = "s3")]
pub use s3::{S3Backend, S3Config};
