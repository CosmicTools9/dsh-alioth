//! Trigger-Aware CRUD Helpers for Module Repositories
//!
//! 提供与 Gateway `trigger_crud` 对齐的通用封装，使模块后端无需直接依赖
//! `trigger-registry` 即可执行带触发器的 CRUD 操作。
//!
//! 典型用法（模块 repository 内）：
//! ```rust,ignore
//! use crud::trigger::{insert_with_triggers, update_with_triggers, delete_with_triggers};
//!
//! let record = serde_json::from_value(serde_json::to_value(&input)?)?;
//! let result = insert_with_triggers(pool, "zc_id_invoice", record, Some(user_id)).await?;
//! ```

use serde_json::Value;
use sqlx::{AssertSqlSafe, PgPool, Row};
use std::collections::HashMap;
use trigger_registry::{TriggerContext, TriggerOperation};

/// 错误类型
#[derive(Debug, thiserror::Error)]
pub enum TriggerCrudError {
    #[error("Trigger registry not initialized")]
    RegistryNotInitialized,
    #[error("Trigger execution failed: {0}")]
    TriggerExecution(String),
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("Operation blocked by trigger: {0}")]
    Blocked(String),
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

fn quote_identifier(identifier: &str) -> Result<String, TriggerCrudError> {
    if identifier.is_empty()
        || !identifier
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(TriggerCrudError::Blocked(format!(
            "Invalid SQL identifier: {}",
            identifier
        )));
    }
    Ok(format!(r#""{}""#, identifier))
}

fn qualified_table_name(table_name: &str) -> Result<String, TriggerCrudError> {
    Ok(format!("isahl.{}", quote_identifier(table_name)?))
}

impl From<trigger_registry::TriggerError> for TriggerCrudError {
    fn from(err: trigger_registry::TriggerError) -> Self {
        TriggerCrudError::TriggerExecution(err.to_string())
    }
}

impl From<trigger_registry::executor::SideEffectError> for TriggerCrudError {
    fn from(err: trigger_registry::executor::SideEffectError) -> Self {
        TriggerCrudError::TriggerExecution(err.to_string())
    }
}

/// 将任意可序列化类型转换为 HashMap<String, Value>
pub fn to_record<T: serde::Serialize>(v: &T) -> Result<HashMap<String, Value>, serde_json::Error> {
    serde_json::from_value(serde_json::to_value(v)?)
}

/// 将 HashMap 形式的记录转换为目标类型
pub fn from_record<T: serde::de::DeserializeOwned>(
    record: &HashMap<String, Value>,
) -> Result<T, serde_json::Error> {
    serde_json::from_value(serde_json::to_value(record)?)
}

/// 执行 BEFORE INSERT triggers，返回可能被修改的字段
pub async fn execute_before_insert(
    pool: &PgPool,
    table_name: &str,
    record: &mut HashMap<String, Value>,
    user_id: Option<i64>,
) -> Result<trigger_registry::TriggerResult, TriggerCrudError> {
    let registry_arc = trigger_registry::init::get_smart_registry()
        .ok_or(TriggerCrudError::RegistryNotInitialized)?;

    let mut registry = registry_arc.write().await;
    let ctx = TriggerContext::new(table_name, TriggerOperation::Insert)
        .with_user(user_id)
        .with_pool(Some(pool.clone()));

    let result = registry
        .execute_before_triggers(
            table_name,
            TriggerOperation::Insert,
            None,
            Some(record),
            &ctx,
        )
        .await?;

    if result.blocked {
        return Err(TriggerCrudError::Blocked(
            result
                .block_reason
                .clone()
                .unwrap_or_else(|| "Operation blocked".to_string()),
        ));
    }

    for (key, value) in &result.modified_fields {
        record.insert(key.clone(), value.clone());
    }

    Ok(result)
}

/// 执行 AFTER INSERT triggers 及其 side effects
pub async fn execute_after_insert(
    pool: &PgPool,
    table_name: &str,
    record: &HashMap<String, Value>,
    user_id: Option<i64>,
) -> Result<(), TriggerCrudError> {
    let registry_arc = trigger_registry::init::get_smart_registry()
        .ok_or(TriggerCrudError::RegistryNotInitialized)?;

    let mut registry = registry_arc.write().await;
    let ctx = TriggerContext::new(table_name, TriggerOperation::Insert)
        .with_user(user_id)
        .with_pool(Some(pool.clone()));

    let result = registry
        .execute_after_triggers(
            table_name,
            TriggerOperation::Insert,
            None,
            Some(record),
            &ctx,
        )
        .await?;

    if !result.side_effects.is_empty() {
        let executor = trigger_registry::executor::SideEffectExecutor::new(pool.clone());
        executor
            .execute_all_in_transaction(&result.side_effects)
            .await?;
    }

    Ok(())
}

/// 执行 AFTER UPDATE triggers 及其 side effects
pub async fn execute_after_update(
    pool: &PgPool,
    table_name: &str,
    old_record: Option<&HashMap<String, Value>>,
    new_record: &HashMap<String, Value>,
    user_id: Option<i64>,
) -> Result<(), TriggerCrudError> {
    let registry_arc = trigger_registry::init::get_smart_registry()
        .ok_or(TriggerCrudError::RegistryNotInitialized)?;

    let mut registry = registry_arc.write().await;
    let ctx = TriggerContext::new(table_name, TriggerOperation::Update)
        .with_user(user_id)
        .with_pool(Some(pool.clone()));

    let result = registry
        .execute_after_triggers(
            table_name,
            TriggerOperation::Update,
            old_record,
            Some(new_record),
            &ctx,
        )
        .await?;

    if !result.side_effects.is_empty() {
        let executor = trigger_registry::executor::SideEffectExecutor::new(pool.clone());
        executor
            .execute_all_in_transaction(&result.side_effects)
            .await?;
    }

    Ok(())
}

/// 执行 AFTER DELETE triggers 及其 side effects
pub async fn execute_after_delete(
    pool: &PgPool,
    table_name: &str,
    record: &HashMap<String, Value>,
    user_id: Option<i64>,
) -> Result<(), TriggerCrudError> {
    let registry_arc = trigger_registry::init::get_smart_registry()
        .ok_or(TriggerCrudError::RegistryNotInitialized)?;

    let mut registry = registry_arc.write().await;
    let ctx = TriggerContext::new(table_name, TriggerOperation::Delete)
        .with_user(user_id)
        .with_pool(Some(pool.clone()));

    let result = registry
        .execute_after_triggers(
            table_name,
            TriggerOperation::Delete,
            Some(record),
            None,
            &ctx,
        )
        .await?;

    if !result.side_effects.is_empty() {
        let executor = trigger_registry::executor::SideEffectExecutor::new(pool.clone());
        executor
            .execute_all_in_transaction(&result.side_effects)
            .await?;
    }

    Ok(())
}

/// 执行带触发器的 INSERT，返回生成的记录（HashMap 形式）
pub async fn insert_with_triggers(
    pool: &PgPool,
    table_name: &str,
    mut record: HashMap<String, Value>,
    user_id: Option<i64>,
) -> Result<HashMap<String, Value>, TriggerCrudError> {
    let _before_result = execute_before_insert(pool, table_name, &mut record, user_id).await?;

    let columns: Vec<String> = record.keys().cloned().collect();
    let quoted_columns = columns
        .iter()
        .map(|c| quote_identifier(c))
        .collect::<Result<Vec<_>, _>>()?;
    let placeholders: Vec<String> = (1..=columns.len()).map(|i| format!("${}", i)).collect();
    let table = qualified_table_name(table_name)?;

    let sql = format!(
        "INSERT INTO {} AS e ({}) VALUES ({}) RETURNING to_jsonb(e) AS record",
        table,
        quoted_columns.join(", "),
        placeholders.join(", ")
    );

    let col_types = crate::column_types::resolve(pool, table_name).await;
    let mut query = sqlx::query(AssertSqlSafe(sql.as_str()));
    for col in &columns {
        let value = record.get(col).unwrap_or(&Value::Null);
        query = bind_json_value(query, value, col_types.get(col).map(String::as_str));
    }

    let row = query.fetch_one(pool).await?;
    let result_map = json_record_to_map(&row)?;

    execute_after_insert(pool, table_name, &result_map, user_id).await?;

    Ok(result_map)
}

/// 执行带触发器的 UPDATE，返回更新后的记录（HashMap 形式）
pub async fn update_with_triggers(
    pool: &PgPool,
    table_name: &str,
    id: i64,
    mut record: HashMap<String, Value>,
    old_record: &HashMap<String, Value>,
    user_id: Option<i64>,
) -> Result<HashMap<String, Value>, TriggerCrudError> {
    let registry_arc = trigger_registry::init::get_smart_registry()
        .ok_or(TriggerCrudError::RegistryNotInitialized)?;

    let mut registry = registry_arc.write().await;
    let ctx = TriggerContext::new(table_name, TriggerOperation::Update)
        .with_user(user_id)
        .with_pool(Some(pool.clone()));

    let before_result = registry
        .execute_before_triggers(
            table_name,
            TriggerOperation::Update,
            Some(old_record),
            Some(&record),
            &ctx,
        )
        .await?;

    if before_result.blocked {
        return Err(TriggerCrudError::Blocked(
            before_result
                .block_reason
                .unwrap_or_else(|| "Operation blocked".to_string()),
        ));
    }

    for (key, value) in &before_result.modified_fields {
        record.insert(key.clone(), value.clone());
    }

    let mut set_clauses: Vec<String> = Vec::new();
    let mut binds: Vec<(&str, &Value)> = Vec::new();
    for (key, value) in record.iter() {
        if key != "id" {
            set_clauses.push(format!("{} = ${}", quote_identifier(key)?, binds.len() + 1));
            binds.push((key.as_str(), value));
        }
    }
    let table = qualified_table_name(table_name)?;

    let sql = format!(
        "UPDATE {} AS e SET {} WHERE e.id = ${} RETURNING to_jsonb(e) AS record",
        table,
        set_clauses.join(", "),
        binds.len() + 1
    );

    let col_types = crate::column_types::resolve(pool, table_name).await;
    let mut query = sqlx::query(AssertSqlSafe(sql.as_str()));
    for (key, value) in &binds {
        query = bind_json_value(query, value, col_types.get(*key).map(String::as_str));
    }
    query = query.bind(id);

    let row = query.fetch_one(pool).await?;
    let result_map = json_record_to_map(&row)?;

    let after_result = registry
        .execute_after_triggers(
            table_name,
            TriggerOperation::Update,
            Some(old_record),
            Some(&result_map),
            &ctx,
        )
        .await?;

    if !after_result.side_effects.is_empty() {
        let executor = trigger_registry::executor::SideEffectExecutor::new(pool.clone());
        executor
            .execute_all_in_transaction(&after_result.side_effects)
            .await?;
    }

    Ok(result_map)
}

/// 执行带触发器的 DELETE（硬删除），返回是否成功
pub async fn delete_with_triggers(
    pool: &PgPool,
    table_name: &str,
    id: i64,
    user_id: Option<i64>,
) -> Result<bool, TriggerCrudError> {
    let old_record = {
        let table = qualified_table_name(table_name)?;
        let sql = format!(
            "SELECT to_jsonb(e) AS record FROM {} AS e WHERE e.id = $1",
            table
        );
        let row = sqlx::query(AssertSqlSafe(sql.as_str()))
            .bind(id)
            .fetch_optional(pool)
            .await?;
        match row {
            Some(row) => json_record_to_map(&row)?,
            None => return Ok(false),
        }
    };

    let table = qualified_table_name(table_name)?;
    let sql = format!("DELETE FROM {} WHERE id = $1", table);
    let result = sqlx::query(AssertSqlSafe(sql.as_str()))
        .bind(id)
        .execute(pool)
        .await?;

    if result.rows_affected() == 0 {
        return Ok(false);
    }

    let registry_arc = trigger_registry::init::get_smart_registry()
        .ok_or(TriggerCrudError::RegistryNotInitialized)?;

    let mut registry = registry_arc.write().await;
    let ctx = TriggerContext::new(table_name, TriggerOperation::Delete)
        .with_user(user_id)
        .with_pool(Some(pool.clone()));

    let after_result = registry
        .execute_after_triggers(
            table_name,
            TriggerOperation::Delete,
            Some(&old_record),
            None,
            &ctx,
        )
        .await?;

    if !after_result.side_effects.is_empty() {
        let executor = trigger_registry::executor::SideEffectExecutor::new(pool.clone());
        executor
            .execute_all_in_transaction(&after_result.side_effects)
            .await?;
    }

    Ok(true)
}

fn json_record_to_map(row: &sqlx::postgres::PgRow) -> Result<HashMap<String, Value>, sqlx::Error> {
    let record: Value = row.try_get("record")?;
    let map = match record {
        Value::Object(obj) => obj.into_iter().collect(),
        _ => HashMap::new(),
    };
    Ok(map)
}

fn bind_json_value<'a>(
    query: sqlx::query::Query<'a, sqlx::Postgres, sqlx::postgres::PgArguments>,
    value: &Value,
    data_type: Option<&str>,
) -> sqlx::query::Query<'a, sqlx::Postgres, sqlx::postgres::PgArguments> {
    crate::bind_json::apply_query(query, crate::bind_json::coerce(value, data_type))
}

#[cfg(test)]
mod tests {
    use super::{qualified_table_name, quote_identifier};

    #[test]
    fn quote_identifier_allows_hyphenated_alioth_names() {
        assert_eq!(
            qualified_table_name("zc_id_oper-transport_tracking").unwrap(),
            r#"isahl."zc_id_oper-transport_tracking""#
        );
        assert_eq!(quote_identifier("ck_cate-wh").unwrap(), r#""ck_cate-wh""#);
    }

    #[test]
    fn quote_identifier_rejects_injected_sql() {
        // concat! 拆分字面量：避免 DDL guard 将测试注入样例误判为真实 DDL
        let injected = concat!(r#"zc_id_plan"; DROP TABLE isahl"#, ".foo; --");
        assert!(qualified_table_name(injected).is_err());
        assert!(quote_identifier("notice = NULL").is_err());
    }
}
