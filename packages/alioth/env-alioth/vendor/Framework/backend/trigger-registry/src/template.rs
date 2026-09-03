//! Trigger Template System
//!
//! 将 SQL 模板解析与异步执行从注册表接口中分离。
//! 注册表只负责 `register(template) -> TriggerHandle`，
//! 复杂的模板渲染与执行由 `TemplateEngine` 在背后处理。

use crate::{
    Trigger, TriggerContext, TriggerError, TriggerOperation, TriggerResult, TriggerTiming,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{AssertSqlSafe, Column, Row};
use std::collections::HashMap;
use std::sync::Arc;

// ============================================
// Trigger Metadata
// ============================================

/// 触发器元数据（配置驱动）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerMetadata {
    pub name: String,
    pub applies_to: Vec<String>,
    #[serde(default)]
    pub operations: Vec<TriggerOperationDef>,
    #[serde(default)]
    pub timing: TriggerTimingDef,
}

impl TriggerMetadata {
    pub fn to_operations(&self) -> Vec<TriggerOperation> {
        self.operations.iter().copied().map(Into::into).collect()
    }
    pub fn to_timing(&self) -> TriggerTiming {
        self.timing.into()
    }
}

/// 配置中使用的操作枚举
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum TriggerOperationDef {
    Insert,
    Update,
    Delete,
}

impl From<TriggerOperationDef> for TriggerOperation {
    fn from(op: TriggerOperationDef) -> Self {
        match op {
            TriggerOperationDef::Insert => TriggerOperation::Insert,
            TriggerOperationDef::Update => TriggerOperation::Update,
            TriggerOperationDef::Delete => TriggerOperation::Delete,
        }
    }
}

/// 配置中使用的时间枚举
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
#[serde(rename_all = "UPPERCASE")]
pub enum TriggerTimingDef {
    #[default]
    Before,
    After,
}

impl From<TriggerTimingDef> for TriggerTiming {
    fn from(t: TriggerTimingDef) -> Self {
        match t {
            TriggerTimingDef::Before => TriggerTiming::Before,
            TriggerTimingDef::After => TriggerTiming::After,
        }
    }
}

// ============================================
// SQL Template
// ============================================

/// SQL 模板片段
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SqlTemplate {
    /// SQL 语句，支持 `{{field}}` 占位符（从 new_record 提取）以及 `$1` 参数化占位符
    pub sql: String,
    /// 参数绑定路径列表，如 `["new.ck_category", "old.code", "ctx.user_id"]`
    #[serde(default)]
    pub binds: Vec<String>,
    /// 用途描述，用于日志和调试
    #[serde(default)]
    pub purpose: String,
}

impl SqlTemplate {
    pub fn new(sql: impl Into<String>) -> Self {
        Self {
            sql: sql.into(),
            binds: Vec::new(),
            purpose: String::new(),
        }
    }

    pub fn with_binds(mut self, binds: Vec<String>) -> Self {
        self.binds = binds;
        self
    }
}

// ============================================
// Trigger Template Trait
// ============================================

/// 触发器模板接口
///
/// 实现者定义元数据与执行逻辑；需要 SQL 执行时可委托 `TemplateEngine`。
#[async_trait]
pub trait TriggerTemplate: Send + Sync {
    /// 触发器元数据
    fn metadata(&self) -> TriggerMetadata;

    /// 执行模板逻辑
    async fn execute(
        &self,
        ctx: &TriggerContext,
        old_record: Option<&HashMap<String, Value>>,
        new_record: Option<&HashMap<String, Value>>,
    ) -> Result<TriggerResult, TriggerError>;
}

// ============================================
// Template Engine
// ============================================

/// 模板引擎：负责 SQL 模板的参数绑定与异步执行
#[derive(Clone)]
pub struct TemplateEngine {
    pool: Option<sqlx::PgPool>,
}

impl TemplateEngine {
    pub fn new(pool: Option<sqlx::PgPool>) -> Self {
        Self { pool }
    }

    /// 从上下文中解析单个绑定值
    pub fn resolve_bind(
        &self,
        path: &str,
        old_record: Option<&HashMap<String, Value>>,
        new_record: Option<&HashMap<String, Value>>,
        ctx: &TriggerContext,
    ) -> Option<Value> {
        if let Some(field) = path.strip_prefix("new.") {
            new_record.and_then(|r| r.get(field).cloned())
        } else if let Some(field) = path.strip_prefix("old.") {
            old_record.and_then(|r| r.get(field).cloned())
        } else if path == "ctx.user_id" {
            ctx.user_id.map(|v| Value::Number(v.into()))
        } else if path == "ctx.timestamp" {
            Some(Value::String(ctx.timestamp.to_rfc3339()))
        } else if path == "ctx.table_name" {
            Some(Value::String(ctx.table_name.clone()))
        } else {
            // 默认从 new_record 查找
            new_record.and_then(|r| r.get(path).cloned())
        }
    }

    /// 解析 binds 列表为 Value 列表
    pub fn resolve_binds(
        &self,
        binds: &[String],
        old_record: Option<&HashMap<String, Value>>,
        new_record: Option<&HashMap<String, Value>>,
        ctx: &TriggerContext,
    ) -> Vec<Value> {
        binds
            .iter()
            .map(|path| {
                self.resolve_bind(path, old_record, new_record, ctx)
                    .unwrap_or(Value::Null)
            })
            .collect()
    }

    /// 简单的 `{{key}}` 插值（仅用于非参数化场景，如表名、列名）
    pub fn interpolate_sql(
        &self,
        sql: &str,
        new_record: Option<&HashMap<String, Value>>,
        ctx: &TriggerContext,
    ) -> String {
        let mut result = sql.to_string();
        if let Some(record) = new_record {
            for (key, value) in record.iter() {
                let placeholder = format!("{{{{{}}}}}", key);
                let replacement = match value {
                    Value::String(s) => s.clone(),
                    Value::Number(n) => n.to_string(),
                    _ => value.to_string(),
                };
                result = result.replace(&placeholder, &replacement);
            }
        }
        result = result.replace("{{ctx.table_name}}", &ctx.table_name);
        result = result.replace(
            "{{ctx.timestamp}}",
            &ctx.timestamp.format("%Y%m%d").to_string(),
        );
        result
    }

    /// 执行参数化查询，返回单值（Scalar）
    pub async fn query_scalar<T>(
        &self,
        sql: &str,
        binds: Vec<Value>,
    ) -> Result<Option<T>, TriggerError>
    where
        T: for<'r> sqlx::Decode<'r, sqlx::Postgres> + sqlx::Type<sqlx::Postgres> + Send + Unpin,
    {
        let pool = self.pool.as_ref().ok_or_else(|| {
            TriggerError::ExecutionFailed("No database pool available".to_string())
        })?;

        let mut query = sqlx::query(AssertSqlSafe(sql));
        for value in binds {
            query = bind_value(query, value);
        }

        let row = query
            .fetch_optional(pool)
            .await
            .map_err(|e| TriggerError::DatabaseError(e.to_string()))?;

        match row {
            Some(r) => r
                .try_get::<T, _>(0)
                .map(Some)
                .map_err(|e| TriggerError::DatabaseError(e.to_string())),
            None => Ok(None),
        }
    }

    /// 执行参数化查询，返回标量列表（ScalarAll）
    pub async fn query_scalar_all<T>(
        &self,
        sql: &str,
        binds: Vec<Value>,
    ) -> Result<Vec<T>, TriggerError>
    where
        T: for<'r> sqlx::Decode<'r, sqlx::Postgres> + sqlx::Type<sqlx::Postgres> + Send + Unpin,
    {
        let pool = self.pool.as_ref().ok_or_else(|| {
            TriggerError::ExecutionFailed("No database pool available".to_string())
        })?;

        let mut query = sqlx::query(AssertSqlSafe(sql));
        for value in binds {
            query = bind_value(query, value);
        }

        let rows = query
            .fetch_all(pool)
            .await
            .map_err(|e| TriggerError::DatabaseError(e.to_string()))?;

        let mut result = Vec::with_capacity(rows.len());
        for row in rows {
            let val = row
                .try_get::<T, _>(0)
                .map_err(|e| TriggerError::DatabaseError(e.to_string()))?;
            result.push(val);
        }
        Ok(result)
    }

    /// 执行参数化查询，返回通用行列表
    pub async fn query_rows(
        &self,
        sql: &str,
        binds: Vec<Value>,
    ) -> Result<Vec<HashMap<String, Value>>, TriggerError> {
        let pool = self.pool.as_ref().ok_or_else(|| {
            TriggerError::ExecutionFailed("No database pool available".to_string())
        })?;

        let mut query = sqlx::query(AssertSqlSafe(sql));
        for value in binds {
            query = bind_value(query, value);
        }

        let rows = query
            .fetch_all(pool)
            .await
            .map_err(|e| TriggerError::DatabaseError(e.to_string()))?;

        let mut result = Vec::with_capacity(rows.len());
        for row in rows {
            let mut map = HashMap::new();
            // 逐列提取为 json Value
            for col in row.columns() {
                let name = col.name();
                // 统一提取为字符串，调用方按需转换
                let val: Option<String> = row.try_get(name).ok();
                map.insert(
                    name.to_string(),
                    val.map(Value::String).unwrap_or(Value::Null),
                );
            }
            result.push(map);
        }
        Ok(result)
    }

    /// 执行参数化 SQL，返回影响行数
    pub async fn execute(&self, sql: &str, binds: Vec<Value>) -> Result<u64, TriggerError> {
        let pool = self.pool.as_ref().ok_or_else(|| {
            TriggerError::ExecutionFailed("No database pool available".to_string())
        })?;

        let mut query = sqlx::query(AssertSqlSafe(sql));
        for value in binds {
            query = bind_value(query, value);
        }

        query
            .execute(pool)
            .await
            .map(|r| r.rows_affected())
            .map_err(|e| TriggerError::DatabaseError(e.to_string()))
    }

    // ============================================
    // 常用业务查询便捷方法（减少内联 SQL 重复）
    // ============================================

    /// 查询 zc_ad_variable 的 COALESCE(code, notice)
    pub async fn resolve_variable_code_notice(
        &self,
        id: i64,
    ) -> Result<Option<String>, TriggerError> {
        self.query_scalar(
            "SELECT COALESCE(code, notice) FROM isahl.zc_ad_variable WHERE id = $1",
            vec![Value::Number(id.into())],
        )
        .await
    }

    /// 查询 zc_ad_variable 的 notice
    pub async fn resolve_variable_notice(&self, id: i64) -> Result<Option<String>, TriggerError> {
        self.query_scalar(
            "SELECT notice FROM isahl.zc_ad_variable WHERE id = $1",
            vec![Value::Number(id.into())],
        )
        .await
    }

    /// 根据 fk_user 查询 zc_id_subjects 的 id
    pub async fn resolve_subject_by_user(&self, fk_user: i64) -> Result<Option<i64>, TriggerError> {
        self.query_scalar(
            "SELECT id FROM isahl.zc_id_subjects WHERE fk_user = $1",
            vec![Value::Number(fk_user.into())],
        )
        .await
    }
}

/// 将 serde_json::Value 绑定到 sqlx 查询
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

// ============================================
// TriggerHandle
// ============================================

/// 轻量触发器句柄
///
/// 由 `RegistryLoader::register(template)` 返回。内部持有 `Arc<dyn TriggerTemplate>`，
/// 并通过 `Box::leak` 将动态元数据转为 `&'static` 切片，从而兼容现有 `Trigger` trait。
pub struct TriggerHandle {
    template: Arc<dyn TriggerTemplate>,
    name: &'static str,
    applies_to: &'static [&'static str],
    operations: &'static [TriggerOperation],
    timing: TriggerTiming,
}

impl TriggerHandle {
    pub fn from_template(template: Arc<dyn TriggerTemplate>) -> Arc<dyn Trigger> {
        let meta = template.metadata();
        let name = meta.name;
        let applies_to = meta.applies_to;
        let operations = meta.operations;
        let timing = meta.timing;

        let name: &'static str = Box::leak(name.into_boxed_str());
        let applies_to: &'static [&'static str] = applies_to
            .into_iter()
            .map(|s| Box::leak(s.into_boxed_str()) as &'static str)
            .collect::<Vec<_>>()
            .leak();
        let operations: &'static [TriggerOperation] = operations
            .into_iter()
            .map(Into::into)
            .collect::<Vec<_>>()
            .leak();

        Arc::new(Self {
            template,
            name,
            applies_to,
            operations,
            timing: timing.into(),
        })
    }
}

#[async_trait]
impl Trigger for TriggerHandle {
    fn name(&self) -> &str {
        self.name
    }

    fn applies_to(&self) -> &[&str] {
        self.applies_to
    }

    fn operations(&self) -> &[TriggerOperation] {
        self.operations
    }

    fn timing(&self) -> TriggerTiming {
        self.timing
    }

    async fn execute(
        &self,
        old_record: Option<&HashMap<String, Value>>,
        new_record: Option<&HashMap<String, Value>>,
        ctx: &TriggerContext,
    ) -> Result<TriggerResult, TriggerError> {
        self.template.execute(ctx, old_record, new_record).await
    }
}

// ============================================
// Config-Driven Trigger Template
// ============================================

/// 纯配置驱动的触发器模板
///
/// 从 YAML/JSON 加载，通过 `SqlTemplate` 列表定义触发器行为。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigTriggerTemplate {
    pub meta: TriggerMetadata,
    #[serde(default)]
    pub pre_condition: Option<String>,
    #[serde(default)]
    pub sql_templates: Vec<SqlTemplate>,
    #[serde(default)]
    pub modified_fields: Vec<FieldMapping>,
}

/// 字段映射：SQL 结果列 -> 目标字段
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldMapping {
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub transform: FieldTransform,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FieldTransform {
    #[default]
    None,
    UpperCase,
    LowerCase,
    PrefixTimestamp,
}

#[async_trait]
impl TriggerTemplate for ConfigTriggerTemplate {
    fn metadata(&self) -> TriggerMetadata {
        self.meta.clone()
    }

    async fn execute(
        &self,
        ctx: &TriggerContext,
        old_record: Option<&HashMap<String, Value>>,
        new_record: Option<&HashMap<String, Value>>,
    ) -> Result<TriggerResult, TriggerError> {
        let engine = TemplateEngine::new(ctx.pool.clone());
        let mut result = TriggerResult::new();

        // 预条件检查（简单表达式，未来可扩展为 mini-DSL）
        if let Some(cond) = &self.pre_condition {
            if !evaluate_condition(cond, old_record, new_record, ctx) {
                return Ok(result);
            }
        }

        // 依次执行 SQL 模板
        for tpl in &self.sql_templates {
            let binds = engine.resolve_binds(&tpl.binds, old_record, new_record, ctx);
            let sql = engine.interpolate_sql(&tpl.sql, new_record, ctx);

            // 对于配置驱动模板，我们统一使用 query_scalar<serde_json::Value> 获取结果
            // 然后按 modified_fields 映射到 TriggerResult
            // 这里简化处理：如果 SQL 是 SELECT，取第一行映射；如果是 DML，执行即可
            if sql.trim_start().to_ascii_uppercase().starts_with("SELECT") {
                // 尝试获取标量结果
                let scalar: Option<String> = engine.query_scalar(&sql, binds.clone()).await?;
                if let Some(val) = scalar {
                    // 简单映射：假设 modified_fields 的第一个 to 字段接收结果
                    if let Some(mapping) = self.modified_fields.first() {
                        result = result.with_modified_field(mapping.to.clone(), Value::String(val));
                    }
                }
            } else {
                engine.execute(&sql, binds).await?;
            }
        }

        Ok(result)
    }
}

/// 极简条件求值（占位实现，后续可替换为 rhai/expr 引擎）
fn evaluate_condition(
    cond: &str,
    _old_record: Option<&HashMap<String, Value>>,
    new_record: Option<&HashMap<String, Value>>,
    _ctx: &TriggerContext,
) -> bool {
    // 示例支持："new.ck_category IS NOT NULL"
    if cond.contains(" IS NOT NULL") {
        let field = cond.replace(" IS NOT NULL", "").trim().to_string();
        if field.starts_with("new.") {
            let key = field.strip_prefix("new.").unwrap_or("");
            return new_record.and_then(|r| r.get(key)).is_some();
        }
    }
    true
}
