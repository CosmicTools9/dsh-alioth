//! Trigger-Aware CRUD Helpers for Gateway
//!
//! 为手写 API handler 提供统一的 Trigger Registry 接入封装，
//! 避免在每个 handler 中重复编写 trigger 调用逻辑。

use serde_json::Value;
use sqlx::{AssertSqlSafe, PgPool, Row};
use std::collections::HashMap;
use std::sync::OnceLock;
use trigger_registry::{TriggerContext, TriggerOperation, TriggerResult};

use crate::notification::service::NotificationService;

// 全局通知服务实例 — 在 Gateway main.rs 启动时注入
static GLOBAL_NOTIFICATION_SERVICE: OnceLock<NotificationService> = OnceLock::new();

/// 设置全局通知服务实例。
/// 由 Gateway main.rs 在启动时调用一次。
pub fn set_notification_service(svc: NotificationService) {
    let _ = GLOBAL_NOTIFICATION_SERVICE.set(svc);
}

fn get_notification_service() -> Option<&'static NotificationService> {
    GLOBAL_NOTIFICATION_SERVICE.get()
}

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
}

impl From<trigger_registry::TriggerError> for TriggerCrudError {
    fn from(err: trigger_registry::TriggerError) -> Self {
        TriggerCrudError::TriggerExecution(err.to_string())
    }
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

/// 执行 BEFORE INSERT 触发器，返回可能被修改的字段
pub async fn execute_before_insert(
    pool: &PgPool,
    table_name: &str,
    record: &mut HashMap<String, Value>,
    user_id: Option<i64>,
) -> Result<TriggerResult, TriggerCrudError> {
    let registry_arc = trigger_registry::init::get_smart_registry()
        .ok_or(TriggerCrudError::RegistryNotInitialized)?;

    let mut registry = registry_arc.write().await;
    let ctx = TriggerContext::new(table_name, TriggerOperation::Insert)
        .with_user(user_id)
        .with_pool(Some(pool.clone()))
        .with_app_container(trigger_registry::AppContainer::Gateway);

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
                .unwrap_or_else(|| "Operation blocked".to_string()),
        ));
    }

    // 应用触发器修改到 record
    for (key, value) in &result.modified_fields {
        record.insert(key.clone(), value.clone());
    }

    Ok(result)
}

/// 执行 AFTER INSERT 触发器及其 side effects，并自动触发数据变更通知。
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
        .with_pool(Some(pool.clone()))
        .with_app_container(trigger_registry::AppContainer::Gateway);

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
            .await
            .map_err(|e| TriggerCrudError::TriggerExecution(e.to_string()))?;
    }

    // 数据变更通知 — 如果全局通知服务已配置
    if let Some(svc) = get_notification_service() {
        if let Some(Value::Number(id_val)) = record.get("id") {
            if let Some(id) = id_val.as_i64() {
                svc.notify_data_change(table_name, id, "insert", record)
                    .await;
            }
        }
    }

    Ok(())
}

/// 执行 AFTER UPDATE 触发器及其 side effects，并自动触发数据变更通知。
pub async fn execute_after_update(
    pool: &PgPool,
    table_name: &str,
    old_record: &HashMap<String, Value>,
    new_record: &HashMap<String, Value>,
    user_id: Option<i64>,
) -> Result<(), TriggerCrudError> {
    let registry_arc = trigger_registry::init::get_smart_registry()
        .ok_or(TriggerCrudError::RegistryNotInitialized)?;

    let mut registry = registry_arc.write().await;
    let ctx = TriggerContext::new(table_name, TriggerOperation::Update)
        .with_user(user_id)
        .with_pool(Some(pool.clone()))
        .with_app_container(trigger_registry::AppContainer::Gateway);

    let result = registry
        .execute_after_triggers(
            table_name,
            TriggerOperation::Update,
            Some(old_record),
            Some(new_record),
            &ctx,
        )
        .await?;

    if !result.side_effects.is_empty() {
        let executor = trigger_registry::executor::SideEffectExecutor::new(pool.clone());
        executor
            .execute_all_in_transaction(&result.side_effects)
            .await
            .map_err(|e| TriggerCrudError::TriggerExecution(e.to_string()))?;
    }

    // 数据变更通知
    if let Some(svc) = get_notification_service() {
        if let Some(Value::Number(id_val)) = new_record.get("id") {
            if let Some(id) = id_val.as_i64() {
                svc.notify_data_change(table_name, id, "update", new_record)
                    .await;
            }
        }
    }

    Ok(())
}

/// 执行 AFTER DELETE 触发器及其 side effects，并自动触发数据变更通知。
pub async fn execute_after_delete(
    pool: &PgPool,
    table_name: &str,
    old_record: &HashMap<String, Value>,
    user_id: Option<i64>,
) -> Result<(), TriggerCrudError> {
    let registry_arc = trigger_registry::init::get_smart_registry()
        .ok_or(TriggerCrudError::RegistryNotInitialized)?;

    let mut registry = registry_arc.write().await;
    let ctx = TriggerContext::new(table_name, TriggerOperation::Delete)
        .with_user(user_id)
        .with_pool(Some(pool.clone()))
        .with_app_container(trigger_registry::AppContainer::Gateway);

    let result = registry
        .execute_after_triggers(
            table_name,
            TriggerOperation::Delete,
            Some(old_record),
            None,
            &ctx,
        )
        .await?;

    if !result.side_effects.is_empty() {
        let executor = trigger_registry::executor::SideEffectExecutor::new(pool.clone());
        executor
            .execute_all_in_transaction(&result.side_effects)
            .await
            .map_err(|e| TriggerCrudError::TriggerExecution(e.to_string()))?;
    }

    // 数据变更通知
    if let Some(svc) = get_notification_service() {
        if let Some(Value::Number(id_val)) = old_record.get("id") {
            if let Some(id) = id_val.as_i64() {
                svc.notify_data_change(table_name, id, "delete", old_record)
                    .await;
            }
        }
    }

    Ok(())
}

/// 将输入值转换为 serde_json::Value
pub fn to_value<T: serde::Serialize>(v: T) -> Value {
    serde_json::to_value(v).unwrap_or(Value::Null)
}

/// 执行带触发器的 INSERT，返回生成的记录（HashMap 形式）
///
/// 这是 Gateway 手写 handler 接入 Trigger Registry 的推荐模式：
/// 1. 构建输入 record
/// 2. 调用 execute_before_insert
/// 3. 动态构建 INSERT 并执行
/// 4. 调用 execute_after_insert
pub async fn insert_with_triggers(
    pool: &PgPool,
    table_name: &str,
    mut record: HashMap<String, Value>,
    user_id: Option<i64>,
) -> Result<HashMap<String, Value>, TriggerCrudError> {
    // 1. BEFORE INSERT triggers
    let _before_result = execute_before_insert(pool, table_name, &mut record, user_id).await?;

    // 2. 构建 INSERT SQL
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

    let mut query = sqlx::query(AssertSqlSafe(sql.as_str()));
    for col in &columns {
        query = bind_json_value(query, record.get(col).unwrap_or(&Value::Null));
    }

    let row = query.fetch_one(pool).await?;

    // 3. 转换为 HashMap
    let result_map = json_record_to_map(&row)?;

    // 4. AFTER INSERT triggers + side effects
    execute_after_insert(pool, table_name, &result_map, user_id).await?;

    Ok(result_map)
}

/// 执行带触发器的 UPDATE，返回更新后的记录（HashMap 形式）
///
/// 1. 执行 BEFORE UPDATE triggers
/// 2. 动态构建 UPDATE 并执行
/// 3. 执行 AFTER UPDATE triggers + side effects
pub async fn update_with_triggers(
    pool: &PgPool,
    table_name: &str,
    id: i64,
    mut record: HashMap<String, Value>,
    old_record: &HashMap<String, Value>,
    user_id: Option<i64>,
) -> Result<HashMap<String, Value>, TriggerCrudError> {
    // 1. BEFORE UPDATE triggers
    let registry_arc = trigger_registry::init::get_smart_registry()
        .ok_or(TriggerCrudError::RegistryNotInitialized)?;

    let mut registry = registry_arc.write().await;
    let ctx = TriggerContext::new(table_name, TriggerOperation::Update)
        .with_user(user_id)
        .with_pool(Some(pool.clone()))
        .with_app_container(trigger_registry::AppContainer::Gateway);

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

    // 应用触发器修改
    for (key, value) in before_result.modified_fields {
        record.insert(key, value);
    }

    // 2. 构建 UPDATE SQL（排除 id 字段）
    let mut set_clauses: Vec<String> = Vec::new();
    let mut binds: Vec<&Value> = Vec::new();
    for (key, value) in record.iter() {
        if key != "id" {
            set_clauses.push(format!("{} = ${}", quote_identifier(key)?, binds.len() + 1));
            binds.push(value);
        }
    }
    let table = qualified_table_name(table_name)?;

    let sql = format!(
        "UPDATE {} AS e SET {} WHERE e.id = ${} RETURNING to_jsonb(e) AS record",
        table,
        set_clauses.join(", "),
        binds.len() + 1
    );

    let mut query = sqlx::query(AssertSqlSafe(sql.as_str()));
    for value in &binds {
        query = bind_json_value(query, value);
    }
    query = query.bind(id);

    let row = query.fetch_one(pool).await?;

    // 3. 转换为 HashMap
    let result_map = json_record_to_map(&row)?;

    // 4. AFTER UPDATE triggers + side effects
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
            .await
            .map_err(|e| TriggerCrudError::TriggerExecution(e.to_string()))?;
    }

    // 数据变更通知
    if let Some(svc) = get_notification_service() {
        if let Some(Value::Number(id_val)) = result_map.get("id") {
            if let Some(id) = id_val.as_i64() {
                svc.notify_data_change(table_name, id, "update", &result_map)
                    .await;
            }
        }
    }

    Ok(result_map)
}

/// 执行带触发器的 DELETE（硬删除），返回是否成功
///
/// 1. 获取旧记录
/// 2. 执行 DELETE
/// 3. 执行 AFTER DELETE triggers + side effects
pub async fn delete_with_triggers(
    pool: &PgPool,
    table_name: &str,
    id: i64,
    user_id: Option<i64>,
) -> Result<bool, TriggerCrudError> {
    // 获取旧记录
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

    // 执行 DELETE
    let table = qualified_table_name(table_name)?;
    let sql = format!("DELETE FROM {} WHERE id = $1", table);
    let result = sqlx::query(AssertSqlSafe(sql.as_str()))
        .bind(id)
        .execute(pool)
        .await?;

    if result.rows_affected() == 0 {
        return Ok(false);
    }

    // AFTER DELETE triggers
    let registry_arc = trigger_registry::init::get_smart_registry()
        .ok_or(TriggerCrudError::RegistryNotInitialized)?;

    let mut registry = registry_arc.write().await;
    let ctx = TriggerContext::new(table_name, TriggerOperation::Delete)
        .with_user(user_id)
        .with_pool(Some(pool.clone()))
        .with_app_container(trigger_registry::AppContainer::Gateway);

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
            .await
            .map_err(|e| TriggerCrudError::TriggerExecution(e.to_string()))?;
    }

    // 数据变更通知
    if let Some(svc) = get_notification_service() {
        svc.notify_data_change(table_name, id, "delete", &old_record)
            .await;
    }

    Ok(true)
}

/// 将 serde_json::Value 绑定到 sqlx 查询
fn bind_json_value<'a>(
    query: sqlx::query::Query<'a, sqlx::Postgres, sqlx::postgres::PgArguments>,
    value: &Value,
) -> sqlx::query::Query<'a, sqlx::Postgres, sqlx::postgres::PgArguments> {
    match value {
        Value::Null => query.bind::<Option<String>>(None),
        Value::Bool(v) => query.bind(*v),
        Value::Number(v) => {
            if let Some(i) = v.as_i64() {
                query.bind(i)
            } else if let Some(f) = v.as_f64() {
                query.bind(f)
            } else {
                query.bind(v.to_string())
            }
        }
        Value::String(v) => query.bind(v.clone()),
        Value::Array(v) => query.bind(serde_json::Value::Array(v.clone())),
        Value::Object(v) => query.bind(serde_json::Value::Object(v.clone())),
    }
}

fn json_record_to_map(row: &sqlx::postgres::PgRow) -> Result<HashMap<String, Value>, sqlx::Error> {
    let record: Value = row.try_get("record")?;
    let map = match record {
        Value::Object(obj) => obj.into_iter().collect(),
        _ => HashMap::new(),
    };
    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::{qualified_table_name, quote_identifier};

    #[test]
    fn quote_identifier_allows_hyphenated_alioth_names() {
        assert_eq!(
            qualified_table_name("zc_id_msgs-review").unwrap(),
            r#"isahl."zc_id_msgs-review""#
        );
        assert_eq!(quote_identifier("ck_cate-wh").unwrap(), r#""ck_cate-wh""#);
    }

    #[test]
    fn quote_identifier_rejects_injected_sql() {
        assert!(qualified_table_name(r#"zc_id_plan"; DROP TABLE isahl.foo; --"#).is_err());
        assert!(quote_identifier("notice = NULL").is_err());
    }
}
