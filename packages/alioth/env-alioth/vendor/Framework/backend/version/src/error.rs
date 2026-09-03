use thiserror::Error;

#[derive(Error, Debug)]
pub enum VersionError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Invalid operation: {0}")]
    InvalidOperation(String),
}

pub type VersionResult<T> = Result<T, VersionError>;
