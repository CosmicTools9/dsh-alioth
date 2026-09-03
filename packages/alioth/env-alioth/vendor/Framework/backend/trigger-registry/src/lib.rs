//! Trigger System for Alioth Meta Model
//!
//! This module implements SQL trigger logic in Rust for complete child tables
//! in the inheritance hierarchy. Since CREATE operations only occur on leaf tables,
//! triggers defined on parent tables are implemented here and applied to their
//! concrete child tables.

use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;

pub mod auxiliary;
pub mod bom;
pub mod business;
pub mod category;
pub mod config_driven;
pub mod cycle_detect;
pub mod dimension;
pub mod dynamic;
pub mod entity;
pub mod executor;
pub mod inheritance;
pub mod init;
pub mod lifecycle;
pub mod loader;
pub mod object;
pub mod operation;
pub mod product;
pub mod registry;
pub mod sort;
pub mod stock_materialization;
pub mod template;
pub mod version;

pub use loader::RegistryLoader;
pub use registry::TriggerRegistry;
pub use template::{SqlTemplate, TemplateEngine, TriggerHandle, TriggerMetadata, TriggerTemplate};

/// Application container context for trigger execution
/// Determines which user table and auth context to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AppContainer {
    /// Meta admin container — uses isahl_meta.meta_user
    #[default]
    Meta,
    /// Gateway business container — uses isahl_auth.auth_users
    Gateway,
}

/// Trigger execution context
#[derive(Debug, Clone)]
pub struct TriggerContext {
    pub table_name: String,
    pub operation: TriggerOperation,
    pub user_id: Option<i64>,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub pool: Option<sqlx::PgPool>,
    /// Application container context — Meta or Gateway
    pub app_container: AppContainer,
}

impl TriggerContext {
    pub fn new(table_name: impl Into<String>, operation: TriggerOperation) -> Self {
        Self {
            table_name: table_name.into(),
            operation,
            user_id: None,
            timestamp: chrono::Utc::now(),
            pool: None,
            app_container: AppContainer::Meta,
        }
    }

    pub fn with_user(mut self, user_id: Option<i64>) -> Self {
        self.user_id = user_id;
        self
    }

    pub fn with_pool(mut self, pool: Option<sqlx::PgPool>) -> Self {
        self.pool = pool;
        self
    }

    pub fn with_app_container(mut self, container: AppContainer) -> Self {
        self.app_container = container;
        self
    }
}

/// Database operation type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerOperation {
    Insert,
    Update,
    Delete,
}

impl std::fmt::Display for TriggerOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TriggerOperation::Insert => write!(f, "INSERT"),
            TriggerOperation::Update => write!(f, "UPDATE"),
            TriggerOperation::Delete => write!(f, "DELETE"),
        }
    }
}

/// Result of trigger execution
#[derive(Debug, Clone, Default)]
pub struct TriggerResult {
    /// Modified fields to apply to the record
    pub modified_fields: HashMap<String, Value>,
    /// Additional operations to perform (e.g., related inserts/updates/deletes)
    pub side_effects: Vec<SideEffect>,
    /// Whether the operation should be blocked
    pub blocked: bool,
    /// Block reason if blocked
    pub block_reason: Option<String>,
}

impl TriggerResult {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_modified_field(mut self, key: impl Into<String>, value: Value) -> Self {
        self.modified_fields.insert(key.into(), value);
        self
    }

    pub fn with_side_effect(mut self, effect: SideEffect) -> Self {
        self.side_effects.push(effect);
        self
    }

    pub fn blocked(reason: impl Into<String>) -> Self {
        Self {
            blocked: true,
            block_reason: Some(reason.into()),
            ..Default::default()
        }
    }
}

/// Side effect operation triggered by a trigger
#[derive(Debug, Clone)]
pub enum SideEffect {
    Insert {
        table: String,
        values: HashMap<String, Value>,
    },
    Update {
        table: String,
        id: i64,
        values: HashMap<String, Value>,
    },
    Delete {
        table: String,
        id: i64,
    },
    RawSql(String),
    /// Parameterized raw SQL with bind parameters (safer than RawSql)
    RawSqlWithParams {
        sql: String,
        params: Vec<Value>,
    },
}

/// Trait for all database triggers
#[async_trait]
pub trait Trigger: Send + Sync {
    /// Unique trigger name
    fn name(&self) -> &str;

    /// Tables this trigger applies to
    fn applies_to(&self) -> &[&str];

    /// Operations this trigger fires on
    fn operations(&self) -> &[TriggerOperation];

    /// Whether this trigger runs before or after the operation
    fn timing(&self) -> TriggerTiming;

    /// Execute the trigger logic
    async fn execute(
        &self,
        old_record: Option<&HashMap<String, Value>>,
        new_record: Option<&HashMap<String, Value>>,
        ctx: &TriggerContext,
    ) -> Result<TriggerResult, TriggerError>;
}

/// Trigger timing
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerTiming {
    Before,
    After,
}

/// Trigger execution error
#[derive(Debug, Clone)]
pub enum TriggerError {
    ExecutionFailed(String),
    ValidationFailed(String),
    DatabaseError(String),
}

impl std::fmt::Display for TriggerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TriggerError::ExecutionFailed(msg) => write!(f, "Trigger execution failed: {}", msg),
            TriggerError::ValidationFailed(msg) => write!(f, "Validation failed: {}", msg),
            TriggerError::DatabaseError(msg) => write!(f, "Database error: {}", msg),
        }
    }
}

impl std::error::Error for TriggerError {}

/// Helper trait for matching table names against inheritance patterns
pub trait TableMatcher {
    /// Check if a table name matches this trigger's target pattern
    fn matches_table(&self, table_name: &str, target_tables: &[&str]) -> bool;
}

impl TableMatcher for dyn Trigger {
    fn matches_table(&self, table_name: &str, target_tables: &[&str]) -> bool {
        target_tables.contains(&table_name)
    }
}

/// Utility functions for trigger implementations
pub mod utils {
    use super::*;
    use crc32fast::Hasher;

    /// Compute CRC32 hash as hex string
    pub fn crc32_hex(input: &str) -> String {
        let mut hasher = Hasher::new();
        hasher.update(input.as_bytes());
        format!("{:08x}", hasher.finalize())
    }

    /// Generate o_number from id, name, and timestamp
    pub fn generate_o_number(
        id: i64,
        name: Option<&str>,
        timestamp: &chrono::DateTime<chrono::Utc>,
    ) -> String {
        let time_str = timestamp.format("%Y%m%d_%H%M%S%3f").to_string();
        let input = format!("{}-{}-{}", id, name.unwrap_or(""), time_str);
        let crc = crc32_hex(&input);
        format!("{}_{}", time_str, crc)
    }

    /// Get a field value from a record
    pub fn get_field<T: serde::de::DeserializeOwned>(
        record: &HashMap<String, Value>,
        field: &str,
    ) -> Option<T> {
        record
            .get(field)
            .and_then(|v| serde_json::from_value(v.clone()).ok())
    }

    /// Get an i64 id field from a record, handling both numeric and string forms.
    ///
    /// Necessary because `serde_zuid` serializes `i64` IDs as JSON strings to avoid
    /// JavaScript precision loss, so `get_field::<i64>` fails on such values.
    pub fn get_id_field(record: &HashMap<String, Value>, field: &str) -> Option<i64> {
        record.get(field).and_then(|v| match v {
            Value::Number(n) => n.as_i64(),
            Value::String(s) => s.parse().ok(),
            _ => None,
        })
    }

    /// Set a field value in a result
    pub fn set_field(result: &mut TriggerResult, key: impl Into<String>, value: impl Into<Value>) {
        result.modified_fields.insert(key.into(), value.into());
    }

    /// Check if an operation is an insert
    pub fn is_insert(op: TriggerOperation) -> bool {
        op == TriggerOperation::Insert
    }

    /// Check if an operation is an update
    pub fn is_update(op: TriggerOperation) -> bool {
        op == TriggerOperation::Update
    }

    /// Check if an operation is a delete
    pub fn is_delete(op: TriggerOperation) -> bool {
        op == TriggerOperation::Delete
    }
}

#[cfg(test)]
mod tests {
    use super::utils::*;
    use super::*;

    #[test]
    fn test_crc32_hex() {
        let hash = crc32_hex("test");
        assert_eq!(hash.len(), 8);
        // CRC32 of "test" is d87f7e0c
        assert_eq!(hash, "d87f7e0c");
    }

    #[test]
    fn test_generate_o_number() {
        let ts = chrono::DateTime::parse_from_rfc3339("2024-01-01T00:00:00.000Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let on = generate_o_number(1, Some("name"), &ts);
        assert_eq!(on.len(), 27);
    }

    #[test]
    fn test_trigger_result_builder() {
        let result =
            TriggerResult::new().with_modified_field("notice", Value::String("test".to_string()));

        assert_eq!(
            result.modified_fields.get("notice"),
            Some(&Value::String("test".to_string()))
        );
    }
}
