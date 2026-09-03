//! Local filesystem storage backend.

use async_trait::async_trait;
use std::path::PathBuf;

use super::StorageBackend;
use crate::error::FileError;

/// Local filesystem storage backend.
///
/// Writes files under `base_path` and generates URLs from `base_url`.
pub struct LocalBackend {
    base_path: PathBuf,
    base_url: String,
}

impl LocalBackend {
    pub fn new(base_path: impl Into<PathBuf>, base_url: impl Into<String>) -> Self {
        Self {
            base_path: base_path.into(),
            base_url: base_url.into(),
        }
    }

    fn resolve_path(&self, key: &str) -> PathBuf {
        self.base_path.join(key)
    }

    fn validate_key(key: &str) -> Result<(), FileError> {
        if key.is_empty()
            || key.starts_with('/') // 绝对路径：PathBuf::join 会替换 base_path
            || key.contains("..")
        {
            return Err(FileError::InvalidKey(key.to_string()));
        }
        // Only allow alphanumeric, '/', '_', '-', '.'
        if !key
            .chars()
            .all(|c| c.is_alphanumeric() || c == '/' || c == '_' || c == '-' || c == '.')
        {
            return Err(FileError::InvalidKey(key.to_string()));
        }
        Ok(())
    }
}

#[async_trait]
impl StorageBackend for LocalBackend {
    async fn put(
        &self,
        key: &str,
        data: Vec<u8>,
        _content_type: &str,
    ) -> Result<String, FileError> {
        Self::validate_key(key)?;
        let path = self.resolve_path(key);

        // Create parent directories
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        tokio::fs::write(&path, &data).await?;
        Ok(key.to_string())
    }

    async fn get(&self, key: &str) -> Result<Vec<u8>, FileError> {
        Self::validate_key(key)?;
        let path = self.resolve_path(key);

        if !path.exists() {
            return Err(FileError::NotFound(0)); // 0 indicates unknown file ID, key not found
        }

        Ok(tokio::fs::read(&path).await?)
    }

    async fn get_range(&self, key: &str, start: u64, end: u64) -> Result<Vec<u8>, FileError> {
        Self::validate_key(key)?;
        let path = self.resolve_path(key);

        let mut file = tokio::fs::File::open(&path)
            .await
            .map_err(|_| FileError::NotFound(0))?;
        use tokio::io::{AsyncReadExt, AsyncSeekExt};
        file.seek(std::io::SeekFrom::Start(start)).await?;
        let len = end.saturating_sub(start) + 1;
        let mut buf = Vec::with_capacity(len as usize);
        file.take(len).read_to_end(&mut buf).await?;
        Ok(buf)
    }

    async fn move_object(&self, from: &str, to: &str) -> Result<(), FileError> {
        Self::validate_key(from)?;
        Self::validate_key(to)?;
        let from_path = self.resolve_path(from);
        let to_path = self.resolve_path(to);
        if let Some(parent) = to_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::rename(&from_path, &to_path).await?;
        Ok(())
    }

    async fn delete(&self, key: &str) -> Result<(), FileError> {
        Self::validate_key(key)?;
        let path = self.resolve_path(key);

        if path.exists() {
            tokio::fs::remove_file(&path).await?;
        }
        Ok(())
    }

    async fn presigned_url(&self, key: &str, _expires_in_secs: u64) -> Result<String, FileError> {
        Self::validate_key(key)?;
        // Local backend: serve via base_url + key
        Ok(format!("{}/{}", self.base_url.trim_end_matches('/'), key))
    }
}
