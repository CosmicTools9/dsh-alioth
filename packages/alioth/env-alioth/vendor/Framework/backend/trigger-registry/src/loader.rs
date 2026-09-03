//! Registry Loader — 从 YAML/JSON/数据库加载触发器定义
//!
//! 提供 `RegistryLoader`：小接口 `register(template) -> Arc<dyn Trigger>`，
//! 大行为（配置解析、模板实例化、异步预热）在背后完成。

use crate::{
    registry::TriggerRegistry,
    template::{ConfigTriggerTemplate, TriggerHandle, TriggerMetadata, TriggerTemplate},
    Trigger,
};
use serde_json::Value;
use sqlx::{AssertSqlSafe, PgPool, Row};
use std::collections::HashMap;
use std::sync::Arc;

// ============================================
// Errors
// ============================================

#[derive(Debug, Clone)]
pub enum LoaderError {
    Io(String),
    Parse(String),
    Db(String),
    Validation(String),
}

impl std::fmt::Display for LoaderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoaderError::Io(msg) => write!(f, "Loader IO error: {}", msg),
            LoaderError::Parse(msg) => write!(f, "Loader parse error: {}", msg),
            LoaderError::Db(msg) => write!(f, "Loader DB error: {}", msg),
            LoaderError::Validation(msg) => write!(f, "Loader validation error: {}", msg),
        }
    }
}

impl std::error::Error for LoaderError {}

impl From<std::io::Error> for LoaderError {
    fn from(e: std::io::Error) -> Self {
        LoaderError::Io(e.to_string())
    }
}

impl From<serde_json::Error> for LoaderError {
    fn from(e: serde_json::Error) -> Self {
        LoaderError::Parse(e.to_string())
    }
}

impl From<yaml_serde::Error> for LoaderError {
    fn from(e: yaml_serde::Error) -> Self {
        LoaderError::Parse(e.to_string())
    }
}

impl From<sqlx::Error> for LoaderError {
    fn from(e: sqlx::Error) -> Self {
        LoaderError::Db(e.to_string())
    }
}

// ============================================
// Registry Loader
// ============================================

/// 注册表加载器
///
/// 负责从多种来源（内置模板、YAML、JSON、DB plugin 表）加载触发器定义，
/// 实例化为 `TriggerHandle` 并汇入 `TriggerRegistry`。
pub struct RegistryLoader {
    registry: TriggerRegistry,
    /// 已注册模板句柄（便于热重载时做 diff）
    handles: Vec<Arc<dyn Trigger>>,
    /// 来源标记 -> 句柄索引（用于按来源卸载）
    source_index: HashMap<String, Vec<usize>>,
}

impl Default for RegistryLoader {
    fn default() -> Self {
        Self::new()
    }
}

impl RegistryLoader {
    pub fn new() -> Self {
        Self {
            registry: TriggerRegistry::new(),
            handles: Vec::new(),
            source_index: HashMap::new(),
        }
    }

    /// 注册单个模板，返回标准 `Arc<dyn Trigger>`（底层为 TriggerHandle）
    ///
    /// 这是核心小接口：`register(template) -> TriggerHandle`
    pub fn register(
        &mut self,
        source: impl Into<String>,
        template: Arc<dyn TriggerTemplate>,
    ) -> Arc<dyn Trigger> {
        let handle = TriggerHandle::from_template(template);
        let source = source.into();

        let idx = self.handles.len();
        self.handles.push(handle.clone());
        self.source_index.entry(source).or_default().push(idx);
        self.registry.register(handle.clone());

        handle
    }

    /// 批量注册内置模板
    pub fn register_batch(
        &mut self,
        source: impl Into<String>,
        templates: Vec<Arc<dyn TriggerTemplate>>,
    ) -> Vec<Arc<dyn Trigger>> {
        let source = source.into();
        templates
            .into_iter()
            .map(|t| self.register(source.clone(), t))
            .collect()
    }

    // ============================================
    // YAML / JSON 文件加载
    // ============================================

    /// 从 YAML 文件加载触发器定义
    pub fn load_from_yaml(&mut self, path: &str) -> Result<Vec<Arc<dyn Trigger>>, LoaderError> {
        let content = std::fs::read_to_string(path)?;
        let defs: Vec<ConfigTriggerTemplate> = yaml_serde::from_str(&content)?;
        Ok(self.register_batch(
            format!("yaml:{}", path),
            defs.into_iter()
                .map(|d| Arc::new(d) as Arc<dyn TriggerTemplate>)
                .collect(),
        ))
    }

    /// 从 JSON 文件加载触发器定义
    pub fn load_from_json(&mut self, path: &str) -> Result<Vec<Arc<dyn Trigger>>, LoaderError> {
        let content = std::fs::read_to_string(path)?;
        let defs: Vec<ConfigTriggerTemplate> = serde_json::from_str(&content)?;
        Ok(self.register_batch(
            format!("json:{}", path),
            defs.into_iter()
                .map(|d| Arc::new(d) as Arc<dyn TriggerTemplate>)
                .collect(),
        ))
    }

    /// 从目录批量加载 `.yaml` / `.yml` / `.json` 配置
    pub fn load_from_dir(&mut self, dir: &str) -> Result<Vec<Arc<dyn Trigger>>, LoaderError> {
        let mut results = Vec::new();
        let entries = std::fs::read_dir(dir)?;

        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            let path_str = path.to_string_lossy();

            if let Some(ext) = path.extension() {
                let ext = ext.to_string_lossy().to_lowercase();
                let batch = match ext.as_str() {
                    "yaml" | "yml" => self.load_from_yaml(&path_str)?,
                    "json" => self.load_from_json(&path_str)?,
                    _ => continue,
                };
                results.extend(batch);
            }
        }

        Ok(results)
    }

    // ============================================
    // Database plugin 表加载
    // ============================================

    /// 从数据库 `isahl_meta.meta_plugins`（或类似表）加载触发器定义。
    ///
    /// 期望表结构（最小集）：
    /// ```sql
    /// CREATE TABLE isahl_meta.meta_plugins (
    ///     id BIGSERIAL PRIMARY KEY,
    ///     name TEXT NOT NULL,
    ///     applies_to TEXT[] NOT NULL,
    ///     operations TEXT[] NOT NULL,
    ///     timing TEXT NOT NULL,
    ///     config JSONB NOT NULL,  -- 包含 sql_templates, pre_condition, modified_fields
    ///     active BOOLEAN DEFAULT TRUE,
    ///     deleted_at TIMESTAMPTZ
    /// );
    /// ```
    pub async fn load_from_db(
        &mut self,
        pool: &PgPool,
        table: Option<&str>,
    ) -> Result<Vec<Arc<dyn Trigger>>, LoaderError> {
        let table = table.unwrap_or("isahl_meta.meta_plugins");

        let sql = format!(
            r#"SELECT name, applies_to, operations, timing, config
               FROM {}
               WHERE active = TRUE AND deleted_at IS NULL"#,
            table
        );
        let rows = sqlx::query(AssertSqlSafe(&sql[..])).fetch_all(pool).await?;

        let mut templates: Vec<Arc<dyn TriggerTemplate>> = Vec::new();

        for row in rows {
            let name: String = row.try_get("name")?;
            let applies_to: Vec<String> = row.try_get("applies_to")?;
            let operations: Vec<String> = row.try_get("operations")?;
            let timing: String = row.try_get("timing")?;
            let config: Value = row.try_get("config")?;

            let meta = TriggerMetadata {
                name,
                applies_to,
                operations: operations
                    .into_iter()
                    .filter_map(|s| match s.to_uppercase().as_str() {
                        "INSERT" => Some(crate::template::TriggerOperationDef::Insert),
                        "UPDATE" => Some(crate::template::TriggerOperationDef::Update),
                        "DELETE" => Some(crate::template::TriggerOperationDef::Delete),
                        _ => None,
                    })
                    .collect(),
                timing: match timing.to_uppercase().as_str() {
                    "AFTER" => crate::template::TriggerTimingDef::After,
                    _ => crate::template::TriggerTimingDef::Before,
                },
            };

            let sql_templates: Vec<crate::template::SqlTemplate> =
                serde_json::from_value(config.get("sql_templates").cloned().unwrap_or(Value::Null))
                    .unwrap_or_default();
            let pre_condition: Option<String> = config
                .get("pre_condition")
                .and_then(|v| v.as_str().map(String::from));
            let modified_fields: Vec<crate::template::FieldMapping> = serde_json::from_value(
                config
                    .get("modified_fields")
                    .cloned()
                    .unwrap_or(Value::Null),
            )
            .unwrap_or_default();

            templates.push(Arc::new(ConfigTriggerTemplate {
                meta,
                pre_condition,
                sql_templates,
                modified_fields,
            }));
        }

        Ok(self.register_batch(format!("db:{}", table), templates))
    }

    // ============================================
    // 导出与访问
    // ============================================

    /// 消费 loader，返回构建好的 `TriggerRegistry`
    pub fn into_registry(self) -> TriggerRegistry {
        self.registry
    }

    /// 获取内部注册表的不可变引用
    pub fn registry(&self) -> &TriggerRegistry {
        &self.registry
    }

    /// 获取已注册句柄数量
    pub fn len(&self) -> usize {
        self.handles.len()
    }

    pub fn is_empty(&self) -> bool {
        self.handles.is_empty()
    }

    /// 获取所有已注册句柄的引用
    pub fn handles(&self) -> &[Arc<dyn Trigger>] {
        &self.handles
    }
}

// ============================================
// Helper: load value from JSON / YAML config
// ============================================

/// 将 JSON Value 解析为 ConfigTriggerTemplate（用于数据库原始 JSONB 的二次解析）
pub fn parse_trigger_config(
    meta: TriggerMetadata,
    config: Value,
) -> Result<ConfigTriggerTemplate, LoaderError> {
    let sql_templates: Vec<crate::template::SqlTemplate> =
        serde_json::from_value(config.get("sql_templates").cloned().unwrap_or(Value::Null))
            .map_err(|e| LoaderError::Parse(e.to_string()))?;
    let pre_condition: Option<String> = config
        .get("pre_condition")
        .and_then(|v| v.as_str().map(String::from));
    let modified_fields: Vec<crate::template::FieldMapping> = serde_json::from_value(
        config
            .get("modified_fields")
            .cloned()
            .unwrap_or(Value::Null),
    )
    .map_err(|e| LoaderError::Parse(e.to_string()))?;

    Ok(ConfigTriggerTemplate {
        meta,
        pre_condition,
        sql_templates,
        modified_fields,
    })
}
