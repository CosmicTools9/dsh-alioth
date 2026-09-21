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
use sqlx::{AssertSqlSafe, PgConnection, PgPool, Row};
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

/// snake_case → camelCase。
///
/// 模型 DTO 统一标 `#[serde(rename_all = "camelCase")]` ⇒ `serde_json::to_value(&req)` 得到的键是**驼峰**
/// （`plan_name` → `planName`），而生成器产出的映射表用的是 **Rust 字段名（snake）**。
/// 两侧口径必须兼容：写入侧按驼峰回查、读回侧两种键都注入。
fn camel_case(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut upper = false;
    for c in name.chars() {
        if c == '_' {
            upper = true;
        } else if upper {
            out.extend(c.to_uppercase());
            upper = false;
        } else {
            out.push(c);
        }
    }
    out
}

/// 同 [`from_record`]，但先按 `column_map`（**物理列名 → DTO 字段名**）重命名键。
///
/// 通道的 RETURNING 以物理列名（L1）为键，而实体 DTO 字段名是业务名（L2）——
/// 未加映射时 `notice AS plan_name` 这类别名列会反序列化为 `None`（字段静默丢失）。
/// 生成器按模型 `SELECT_FIELDS` 派生产物常量 `_READ_COLUMN_MAP` 传入本函数。
pub fn from_record_mapped<T: serde::de::DeserializeOwned>(
    record: &HashMap<String, Value>,
    column_map: &[(&str, &str)],
) -> Result<T, serde_json::Error> {
    let mut renamed = record.clone();
    for (column, dto) in column_map {
        let Some(value) = record.get(*column) else {
            continue;
        };
        if column != dto {
            renamed.insert((*dto).to_string(), value.clone());
        }
        let camel_name = camel_case(dto);
        if camel_name != *dto {
            renamed.insert(camel_name, value.clone());
        }
    }
    serde_json::from_value(serde_json::to_value(&renamed)?)
}

/// 把 DTO / 实体（字段名为 L2 业务名）转为 **物理列名（L1）为键** 的 record。
///
/// `pairs` = `(字段名, 物理列名)`，由生成器按模型声明提供（产物中的 `_WRITE_COLUMN_MAP`）。
/// 未列入 `pairs` 的字段一律丢弃——例如聚合请求的 `items` 不是物理列，若混进 record，
/// `insert_core`/`update_core` 会取 `record.keys()` 当列名生成 `INSERT ... ("items")`
/// 而报「列不存在」（这两个 core 不做列集校验，键名正确性是调用方的责任）。
pub fn record_from_fields<T: serde::Serialize + ?Sized>(
    source: &T,
    pairs: &[(&str, &str)],
) -> Result<HashMap<String, Value>, serde_json::Error> {
    let Value::Object(map) = serde_json::to_value(source)? else {
        return Ok(HashMap::new());
    };
    let mut record = HashMap::with_capacity(pairs.len());
    for (field, column) in pairs {
        // serde `rename_all = "camelCase"` 的 DTO 序列化键是驼峰 ⇒ 未命中时按驼峰回查
        // （否则多词字段被静默丢弃：`plan_name` → 物理列 `notice` 永远写不进去）。
        let value = map.get(*field).or_else(|| map.get(&camel_case(field)));
        if let Some(value) = value {
            record.insert((*column).to_string(), value.clone());
        }
    }
    Ok(record)
}

/// 按主键取一行的 **物理列名** record（`to_jsonb(e)` 即 DB 列名，无需字段映射）。
///
/// UPDATE 的 `old_record`（触发器读旧值）与「行是否存在」判定共用本函数：一次往返替代
/// `get_by_id` + `to_record` 两段，且键名天然是物理列名（`to_record(&entity)` 会带入
/// L2 字段名，对 `update_core` 是错的——见 `record_from_fields` 的列名说明）。
pub async fn get_record(
    pool: &PgPool,
    table_name: &str,
    id: i64,
) -> Result<Option<HashMap<String, Value>>, TriggerCrudError> {
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
        Some(row) => Ok(Some(json_record_to_map(&row)?)),
        None => Ok(None),
    }
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

    // 模板落列 MUST 是本表真实列：模板注册在祖先表上，可能对本表写它不存在的列
    // （实测历史 DimensionVSortTemplate 写 v_sort、而 zc_id_scene 无该列 ⇒ 当时整条 INSERT 失败；
    //   该死模板已于 2026-09-15 退役，此处的落列过滤仍保留为通用防护）。
    // 按真实列集过滤并告警，避免单列不存在托垮整条写；模板/继承图错配须在模板侧另行修正。
    let col_types = crate::column_types::resolve(pool, table_name).await;
    for (key, value) in &result.modified_fields {
        if col_types.contains_key(key) {
            record.insert(key.clone(), value.clone());
        } else {
            log::warn!(
                "trigger modified field dropped: table={} field={} (column not found)",
                table_name,
                key
            );
        }
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
/// INSERT 的公共实现：BEFORE 触发器（读侧用 pool）+ 在 `executor` 上落 DML。
///
/// 事务化基础：pool 版与 tx 版共用本函数 —— `executor` 为调用方连接时，INSERT 即参与调用方事务。
/// 本函数不执行 AFTER 触发器与 side effects（见 `insert_with_triggers_tx`）。
async fn insert_core<'e, E>(
    executor: E,
    pool: &PgPool,
    table_name: &str,
    mut record: HashMap<String, Value>,
    user_id: Option<i64>,
) -> Result<HashMap<String, Value>, TriggerCrudError>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
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

    let row = query.fetch_one(executor).await?;
    Ok(json_record_to_map(&row)?)
}

/// 执行带触发器的 INSERT（事务内变体）：DML 落在 `conn` 上，参与调用方事务。
///
/// AFTER 触发器与 side effects **不在本函数执行** —— 调用方 MUST 在 `COMMIT` 之后执行
/// `execute_after_insert(pool, …)`；未提交时 AFTER 侧读不到本行，副作用也会落在事务之外。
pub async fn insert_with_triggers_tx(
    conn: &mut PgConnection,
    pool: &PgPool,
    table_name: &str,
    record: HashMap<String, Value>,
    user_id: Option<i64>,
) -> Result<HashMap<String, Value>, TriggerCrudError> {
    insert_core(&mut *conn, pool, table_name, record, user_id).await
}

/// 执行带触发器的 INSERT，返回生成的记录（HashMap 形式）
pub async fn insert_with_triggers(
    pool: &PgPool,
    table_name: &str,
    record: HashMap<String, Value>,
    user_id: Option<i64>,
) -> Result<HashMap<String, Value>, TriggerCrudError> {
    let result_map = insert_core(pool, pool, table_name, record, user_id).await?;
    execute_after_insert(pool, table_name, &result_map, user_id).await?;
    Ok(result_map)
}

/// 执行带触发器的 UPDATE，返回更新后的记录（HashMap 形式）
/// UPDATE 的公共实现：BEFORE 触发器（读侧用 pool）+ 在 `executor` 上落 DML。
///
/// 与 pool 版语义一致（含 blocked 判定与 modified_fields 合并）；两种入口共用。
async fn update_core<'e, E>(
    executor: E,
    pool: &PgPool,
    table_name: &str,
    id: i64,
    mut record: HashMap<String, Value>,
    old_record: &HashMap<String, Value>,
    user_id: Option<i64>,
) -> Result<HashMap<String, Value>, TriggerCrudError>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
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

    // 同 insert 侧口径：只落本表真实列，其余告警丢弃
    let col_types = crate::column_types::resolve(pool, table_name).await;
    for (key, value) in &before_result.modified_fields {
        if col_types.contains_key(key) {
            record.insert(key.clone(), value.clone());
        } else {
            log::warn!(
                "trigger modified field dropped: table={} field={} (column not found)",
                table_name,
                key
            );
        }
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

    let row = query.fetch_one(executor).await?;
    Ok(json_record_to_map(&row)?)
}

/// 执行带触发器的 UPDATE（事务内变体）：DML 落在 `conn` 上，参与调用方事务。
///
/// AFTER 触发器与 side effects **不在本函数执行** —— 调用方 MUST 在 `COMMIT` 之后执行
/// `execute_after_update(pool, …)`。
pub async fn update_with_triggers_tx(
    conn: &mut PgConnection,
    pool: &PgPool,
    table_name: &str,
    id: i64,
    record: HashMap<String, Value>,
    old_record: &HashMap<String, Value>,
    user_id: Option<i64>,
) -> Result<HashMap<String, Value>, TriggerCrudError> {
    update_core(
        &mut *conn, pool, table_name, id, record, old_record, user_id,
    )
    .await
}

/// 执行带触发器的 UPDATE，返回更新后的记录（HashMap 形式）
pub async fn update_with_triggers(
    pool: &PgPool,
    table_name: &str,
    id: i64,
    record: HashMap<String, Value>,
    old_record: &HashMap<String, Value>,
    user_id: Option<i64>,
) -> Result<HashMap<String, Value>, TriggerCrudError> {
    let result_map = update_core(pool, pool, table_name, id, record, old_record, user_id).await?;
    execute_after_update(pool, table_name, Some(old_record), &result_map, user_id).await?;
    Ok(result_map)
}

/// 执行带触发器的 DELETE（硬删除），返回是否成功
/// DELETE 的公共实现：一条 `DELETE … RETURNING` 取回被删行并在 `executor` 上执行。
///
/// 单语句同时取旧值（替代 SELECT + DELETE 两段往返）；返回 None = 该行不存在（未删除任何行）。
async fn delete_core<'e, E>(
    executor: E,
    table_name: &str,
    id: i64,
) -> Result<Option<HashMap<String, Value>>, TriggerCrudError>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let table = qualified_table_name(table_name)?;
    let sql = format!(
        "DELETE FROM {} AS e WHERE e.id = $1 RETURNING to_jsonb(e) AS record",
        table
    );
    let row = sqlx::query(AssertSqlSafe(sql.as_str()))
        .bind(id)
        .fetch_optional(executor)
        .await?;

    match row {
        Some(row) => Ok(Some(json_record_to_map(&row)?)),
        None => Ok(None),
    }
}

/// 执行带触发器的 DELETE（事务内变体，硬删除）：DML 落在 `conn` 上，参与调用方事务。
///
/// AFTER 触发器与 side effects **不在本函数执行** —— 调用方 MUST 在 `COMMIT` 之后执行
/// `execute_after_delete(pool, …)`。
pub async fn delete_with_triggers_tx(
    conn: &mut PgConnection,
    table_name: &str,
    id: i64,
) -> Result<bool, TriggerCrudError> {
    Ok(delete_core(&mut *conn, table_name, id).await?.is_some())
}

/// 执行带触发器的 DELETE（硬删除），返回是否成功
pub async fn delete_with_triggers(
    pool: &PgPool,
    table_name: &str,
    id: i64,
    user_id: Option<i64>,
) -> Result<bool, TriggerCrudError> {
    let Some(old_record) = delete_core(pool, table_name, id).await? else {
        return Ok(false);
    };

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
    use super::{
        from_record, from_record_mapped, qualified_table_name, quote_identifier, record_from_fields,
    };

    #[derive(Debug, PartialEq, serde::Deserialize)]
    struct AliasedDto {
        plan_name: Option<String>,
        code: Option<String>,
    }

    #[test]
    fn record_from_fields_reads_camel_case_serde_keys() {
        // DTO 标 `rename_all = "camelCase"` ⇒ `serde_json::to_value(&req)` 的键是 `planName`，
        // 而映射表用 Rust 字段名 `plan_name` ⇒ 未做驼峰回查时物理列 `notice` 被静默丢弃
        // （生成产物真机实测：create 后 notice 为空、plan_name 反序列化为 None）。
        #[derive(Debug, serde::Serialize)]
        #[serde(rename_all = "camelCase")]
        struct CamelReq {
            plan_name: Option<String>,
            code: Option<String>,
        }
        let req = CamelReq {
            plan_name: Some("计划A".to_string()),
            code: Some("C-1".to_string()),
        };
        let record =
            record_from_fields(&req, &[("plan_name", "notice"), ("code", "code")]).unwrap();
        assert_eq!(
            record.get("notice"),
            Some(&serde_json::Value::String("计划A".to_string())),
            "驼峰序列化键 MUST 回查到物理列 notice"
        );
        assert_eq!(
            record.get("code"),
            Some(&serde_json::Value::String("C-1".to_string()))
        );
    }

    #[test]
    fn from_record_mapped_restores_aliased_dto_fields() {
        // 通道 RETURNING 以**物理列名**为键（notice），实体 DTO 字段名是 L2 业务名（plan_name）
        // ⇒ 不经列→DTO 映射，别名字段会静默反序列化为 None（生成产物真机实测缺陷）。
        let mut record = std::collections::HashMap::new();
        record.insert(
            "notice".to_string(),
            serde_json::Value::String("计划A".to_string()),
        );
        record.insert(
            "code".to_string(),
            serde_json::Value::String("C-1".to_string()),
        );

        let plain: AliasedDto = from_record(&record).unwrap();
        assert_eq!(plain.plan_name, None, "基线：未映射时别名字段丢失");

        let mapped: AliasedDto =
            from_record_mapped(&record, &[("notice", "plan_name"), ("code", "code")]).unwrap();
        assert_eq!(mapped.plan_name.as_deref(), Some("计划A"));
        assert_eq!(mapped.code.as_deref(), Some("C-1"));

        // 模型 DTO 标 `rename_all = "camelCase"` ⇒ serde 期望驼峰键，助手 MUST 两种键都提供
        #[derive(Debug, PartialEq, serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct CamelDto {
            plan_name: Option<String>,
        }
        let camel: CamelDto = from_record_mapped(&record, &[("notice", "plan_name")]).unwrap();
        assert_eq!(camel.plan_name.as_deref(), Some("计划A"));
    }

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

    /// `record_from_fields`：键必须是物理列名（L1），未列入映射的字段被剔除。
    /// 聚合请求的 `items` 不是物理列 —— 混进 record 会让 INSERT 报「列不存在」
    /// （`insert_core` 直接取 `record.keys()` 作列名，不做列集校验）。
    #[test]
    fn record_from_fields_maps_dto_names_to_physical_columns() {
        #[derive(serde::Serialize)]
        struct Req {
            title: String,
            items: Vec<i64>,
            absent: Option<i64>,
        }

        let record = record_from_fields(
            &Req {
                title: "t".into(),
                items: vec![1, 2],
                absent: None,
            },
            &[("title", "notice"), ("absent", "fk_absent")],
        )
        .expect("record");

        assert_eq!(record.get("notice"), Some(&serde_json::json!("t")));
        assert_eq!(record.get("fk_absent"), Some(&serde_json::Value::Null));
        assert!(
            !record.contains_key("title"),
            "键须为物理列名，非 DTO 字段名"
        );
        assert!(!record.contains_key("items"), "未列入映射的字段必须剔除");
        assert_eq!(record.len(), 2);
    }
}
