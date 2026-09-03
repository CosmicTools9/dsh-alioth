//! File manager error types.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum FileError {
    #[error("Storage backend error: {0}")]
    StorageBackend(String),

    #[error("File not found: {0}")]
    NotFound(i64),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Checksum mismatch")]
    ChecksumMismatch,

    #[error("Invalid storage key: {0}")]
    InvalidKey(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

impl From<FileError> for common::AliothError {
    fn from(e: FileError) -> Self {
        match e {
            FileError::NotFound(id) => common::AliothError::NotFound(format!("File(id={})", id)),
            FileError::Database(err) => common::AliothError::Database(err.to_string()),
            _ => common::AliothError::Internal(format!("FileManager: {}", e)),
        }
    }
}
