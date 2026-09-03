//! Side Effect Executor for Trigger System
//!
//! Executes side effects produced by triggers, including INSERT, UPDATE, DELETE,
//! and raw SQL operations.

use crate::SideEffect;
use serde_json::Value;
use sqlx::{AssertSqlSafe, PgPool};
use std::collections::HashMap;

/// Error during side effect execution
#[derive(Debug, Clone)]
pub enum SideEffectError {
    DatabaseError(String),
    InvalidTable(String),
    MissingId(String),
    RawSqlError(String),
}

impl std::fmt::Display for SideEffectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SideEffectError::DatabaseError(msg) => write!(f, "Database error: {}", msg),
            SideEffectError::InvalidTable(table) => write!(f, "Invalid table: {}", table),
            SideEffectError::MissingId(msg) => write!(f, "Missing ID: {}", msg),
            SideEffectError::RawSqlError(msg) => write!(f, "Raw SQL error: {}", msg),
        }
    }
}

impl std::error::Error for SideEffectError {}

/// Validate PostgreSQL identifier to prevent SQL injection.
fn validate_pg_ident(ident: &str) -> Result<(), SideEffectError> {
    if ident.is_empty() || ident.len() > 63 {
        return Err(SideEffectError::InvalidTable(ident.to_string()));
    }
    let mut chars = ident.chars();
    if let Some(first) = chars.next() {
        if first.is_ascii_digit() || !(first.is_ascii_alphabetic() || first == '_') {
            return Err(SideEffectError::InvalidTable(ident.to_string()));
        }
    }
    if chars.any(|c| !(c.is_ascii_alphanumeric() || c == '_')) {
        return Err(SideEffectError::InvalidTable(ident.to_string()));
    }
    Ok(())
}

/// Executor for trigger side effects
#[derive(Debug, Clone)]
pub struct SideEffectExecutor {
    pool: PgPool,
}

impl SideEffectExecutor {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Execute a single side effect
    pub async fn execute(&self, effect: &SideEffect) -> Result<(), SideEffectError> {
        match effect {
            SideEffect::Insert { table, values } => self.execute_insert(table, values).await,
            SideEffect::Update { table, id, values } => {
                self.execute_update(table, *id, values).await
            }
            SideEffect::Delete { table, id } => self.execute_delete(table, *id).await,
            SideEffect::RawSql(sql) => self.execute_raw_sql(sql).await,
            SideEffect::RawSqlWithParams { sql, params } => {
                self.execute_raw_sql_with_params(sql, params).await
            }
        }
    }

    /// Execute multiple side effects in order
    pub async fn execute_all(&self, effects: &[SideEffect]) -> Result<(), SideEffectError> {
        for effect in effects {
            self.execute(effect).await?;
        }
        Ok(())
    }

    /// Execute multiple side effects in a transaction
    pub async fn execute_all_in_transaction(
        &self,
        effects: &[SideEffect],
    ) -> Result<(), SideEffectError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| SideEffectError::DatabaseError(e.to_string()))?;

        for effect in effects {
            match effect {
                SideEffect::Insert { table, values } => {
                    validate_pg_ident(table)?;
                    let (columns, placeholders, binds) = build_insert_parts(values);
                    let sql = format!(
                        "INSERT INTO isahl.{} ({}) VALUES ({})",
                        table, columns, placeholders
                    );
                    execute_with_binds(&mut *tx, &sql, binds).await?;
                }
                SideEffect::Update { table, id, values } => {
                    validate_pg_ident(table)?;
                    let (set_clause, binds) = build_update_parts(values);
                    let sql = format!(
                        "UPDATE isahl.{} SET {} WHERE id = ${}",
                        table,
                        set_clause,
                        binds.len() + 1
                    );
                    let mut all_binds = binds;
                    all_binds.push(Value::Number((*id).into()));
                    execute_with_binds(&mut *tx, &sql, all_binds).await?;
                }
                SideEffect::Delete { table, id } => {
                    validate_pg_ident(table)?;
                    let sql = format!("DELETE FROM isahl.{} WHERE id = $1", table);
                    execute_with_binds(&mut *tx, &sql, vec![Value::Number((*id).into())]).await?;
                }
                SideEffect::RawSql(sql) => {
                    sqlx::query(AssertSqlSafe(sql.as_str()))
                        .execute(&mut *tx)
                        .await
                        .map_err(|e| SideEffectError::RawSqlError(e.to_string()))?;
                }
                SideEffect::RawSqlWithParams { sql, params } => {
                    let mut query = sqlx::query(AssertSqlSafe(sql.as_str()));
                    for param in params {
                        query = bind_value(query, param.clone());
                    }
                    query
                        .execute(&mut *tx)
                        .await
                        .map_err(|e| SideEffectError::RawSqlError(e.to_string()))?;
                }
            }
        }

        tx.commit()
            .await
            .map_err(|e| SideEffectError::DatabaseError(e.to_string()))?;

        Ok(())
    }

    async fn execute_insert(
        &self,
        table: &str,
        values: &HashMap<String, Value>,
    ) -> Result<(), SideEffectError> {
        let (columns, placeholders, binds) = build_insert_parts(values);
        let sql = format!(
            "INSERT INTO isahl.{} ({}) VALUES ({})",
            table, columns, placeholders
        );

        execute_with_binds(&self.pool, &sql, binds).await
    }

    async fn execute_update(
        &self,
        table: &str,
        id: i64,
        values: &HashMap<String, Value>,
    ) -> Result<(), SideEffectError> {
        let (set_clause, binds) = build_update_parts(values);
        let sql = format!(
            "UPDATE isahl.{} SET {} WHERE id = ${}",
            table,
            set_clause,
            binds.len() + 1
        );

        let mut all_binds = binds;
        all_binds.push(Value::Number(id.into()));

        execute_with_binds(&self.pool, &sql, all_binds).await
    }

    async fn execute_delete(&self, table: &str, id: i64) -> Result<(), SideEffectError> {
        let sql = format!("DELETE FROM isahl.{} WHERE id = $1", table);
        execute_with_binds(&self.pool, &sql, vec![Value::Number(id.into())]).await
    }

    async fn execute_raw_sql(&self, sql: &str) -> Result<(), SideEffectError> {
        sqlx::query(AssertSqlSafe(sql))
            .execute(&self.pool)
            .await
            .map_err(|e| SideEffectError::RawSqlError(e.to_string()))?;
        Ok(())
    }

    async fn execute_raw_sql_with_params(
        &self,
        sql: &str,
        params: &[Value],
    ) -> Result<(), SideEffectError> {
        let mut query = sqlx::query(AssertSqlSafe(sql));
        for param in params {
            query = bind_value(query, param.clone());
        }
        query
            .execute(&self.pool)
            .await
            .map_err(|e| SideEffectError::RawSqlError(e.to_string()))?;
        Ok(())
    }
}

fn build_insert_parts(values: &HashMap<String, Value>) -> (String, String, Vec<Value>) {
    let mut columns = Vec::new();
    let mut placeholders = Vec::new();
    let mut binds = Vec::new();

    for (idx, (key, value)) in values.iter().enumerate() {
        columns.push(key.clone());
        placeholders.push(format!("${}", idx + 1));
        binds.push(value.clone());
    }

    (columns.join(", "), placeholders.join(", "), binds)
}

fn build_update_parts(values: &HashMap<String, Value>) -> (String, Vec<Value>) {
    let mut set_clauses = Vec::new();
    let mut binds = Vec::new();

    for (key, value) in values.iter() {
        if key == "updated_at" {
            continue;
        }
        let idx = set_clauses.len() + 1;
        set_clauses.push(format!("{} = ${}", key, idx));
        binds.push(value.clone());
    }

    set_clauses.push("updated_at = NOW()".to_string());

    (set_clauses.join(", "), binds)
}

async fn execute_with_binds(
    executor: impl sqlx::Executor<'_, Database = sqlx::Postgres>,
    sql: &str,
    binds: Vec<Value>,
) -> Result<(), SideEffectError> {
    let mut query = sqlx::query(AssertSqlSafe(sql));

    for value in binds {
        query = bind_value(query, value);
    }

    query
        .execute(executor)
        .await
        .map_err(|e| SideEffectError::DatabaseError(e.to_string()))?;

    Ok(())
}

fn bind_value<'a>(
    query: sqlx::query::Query<'a, sqlx::Postgres, sqlx::postgres::PgArguments>,
    value: Value,
) -> sqlx::query::Query<'a, sqlx::Postgres, sqlx::postgres::PgArguments> {
    match value {
        Value::Null => query.bind::<Option<String>>(None),
        Value::Bool(v) => query.bind(v),
        Value::Number(v) => {
            if let Some(i) = v.as_i64() {
                query.bind(i)
            } else if let Some(f) = v.as_f64() {
                query.bind(f)
            } else {
                query.bind(v.to_string())
            }
        }
        Value::String(v) => query.bind(v),
        Value::Array(v) => query.bind(serde_json::Value::Array(v)),
        Value::Object(v) => query.bind(serde_json::Value::Object(v)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_insert_parts() {
        let mut values = HashMap::new();
        values.insert("notice".to_string(), Value::String("Test".to_string()));
        values.insert("fk_entity".to_string(), Value::Number(123i64.into()));

        let (columns, _placeholders, binds) = build_insert_parts(&values);

        assert!(columns.contains("notice"));
        assert!(columns.contains("fk_entity"));
        assert_eq!(binds.len(), 2);
    }

    #[test]
    fn test_build_update_parts() {
        let mut values = HashMap::new();
        values.insert("notice".to_string(), Value::String("Updated".to_string()));
        values.insert(
            "updated_at".to_string(),
            Value::String("2024-01-01".to_string()),
        );

        let (set_clause, binds) = build_update_parts(&values);

        assert!(set_clause.contains("notice = $1"));
        assert_eq!(binds.len(), 1);
    }
}
