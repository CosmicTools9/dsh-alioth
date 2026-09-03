use sqlx::{AssertSqlSafe, PgPool, Row};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SerialError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("Sequence not found: {0}")]
    SequenceNotFound(String),
    #[error("Invalid sequence name: {0}")]
    InvalidSequenceName(String),
}

fn validate_seq_name(name: &str) -> Result<(), SerialError> {
    if name.is_empty() || name.len() > 63 {
        return Err(SerialError::InvalidSequenceName(name.to_string()));
    }
    let mut chars = name.chars();
    if let Some(first) = chars.next() {
        if first.is_ascii_digit() || !(first.is_ascii_alphabetic() || first == '_') {
            return Err(SerialError::InvalidSequenceName(name.to_string()));
        }
    }
    if chars.any(|c| !(c.is_ascii_alphanumeric() || c == '_')) {
        return Err(SerialError::InvalidSequenceName(name.to_string()));
    }
    Ok(())
}

/// Serial/business code generator backed by PostgreSQL sequences
#[derive(Debug, Clone, Default)]
pub struct SerialGenerator;

impl SerialGenerator {
    pub fn new() -> Self {
        Self
    }

    /// Fetch the next value from a named PostgreSQL sequence
    pub async fn next_sequence_value(
        &self,
        pool: &PgPool,
        sequence_name: &str,
    ) -> Result<i64, SerialError> {
        validate_seq_name(sequence_name)?;
        let query = format!("SELECT nextval('{}') as seq", sequence_name);
        let row = sqlx::query(AssertSqlSafe(query.as_str()))
            .fetch_one(pool)
            .await?;
        Ok(row.get::<i64, _>("seq"))
    }

    /// Fetch next value and format it with zero-padding
    pub async fn next_serial(
        &self,
        pool: &PgPool,
        sequence_name: &str,
        width: usize,
        pad_char: char,
    ) -> Result<String, SerialError> {
        let value = self.next_sequence_value(pool, sequence_name).await?;
        Ok(format!("{:0width$}", value, width = width).replace('0', &pad_char.to_string()))
    }

    /// Ensure a PostgreSQL sequence exists, creating it if necessary
    pub async fn ensure_sequence(
        &self,
        pool: &PgPool,
        sequence_name: &str,
        start: i64,
    ) -> Result<(), SerialError> {
        validate_seq_name(sequence_name)?;
        let query = format!(
            "CREATE SEQUENCE IF NOT EXISTS {} START {} MINVALUE 1",
            sequence_name, start
        );
        sqlx::query(AssertSqlSafe(query.as_str()))
            .execute(pool)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_serial_generator_new() {
        let _gen = SerialGenerator::new();
    }

    #[tokio::test]
    async fn test_ensure_sequence_idempotent() {
        // 必须指向测试库（aliothstudio_test），禁止连生产库
        let pool = PgPool::connect(&common::testing::test_database_url())
            .await
            .unwrap();
        let gen = SerialGenerator::new();
        gen.ensure_sequence(&pool, "test_idempotent_seq", 1)
            .await
            .unwrap();
        gen.ensure_sequence(&pool, "test_idempotent_seq", 1)
            .await
            .unwrap();
        let v = gen
            .next_sequence_value(&pool, "test_idempotent_seq")
            .await
            .unwrap();
        assert!(v >= 1);
    }
}
