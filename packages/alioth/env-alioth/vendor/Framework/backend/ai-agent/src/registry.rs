use std::collections::HashMap;
use std::sync::Arc;

use crate::agents::{
    build_default_registry, Agent, AgentCapability, AgentConfig, ConfirmationLevel, DbAccessLevel,
    Modality, ToolDefinition,
};

/// DB 行对内置 Agent 配置的**稀疏覆盖**（只承载「显式给出」的键 + 物理列事实）。
///
/// 语义要点：DB 行是*部分*配置——`settings IS NULL` 或 settings 缺键 MUST NOT
/// 抹掉内置值。否则 `available_tools` / `system_prompt` / `allowed_schemas` 会被
/// 清空，授予面在运行时空转（upgrade-chat-ai-tool-surface E6 前置修复）。
#[derive(Debug, Clone, Default)]
struct AgentOverlay {
    name: Option<String>,
    color: Option<String>,
    icon: Option<String>,
    category: Option<String>,
    sort_order: Option<i32>,
    user_selectable: Option<bool>,
    subject_id: Option<i64>,
    system_prompt: Option<String>,
    capabilities: Option<Vec<AgentCapability>>,
    available_tools: Option<Vec<ToolDefinition>>,
    input_schema: Option<serde_json::Value>,
    output_schema: Option<serde_json::Value>,
    db_access_level: Option<DbAccessLevel>,
    allowed_schemas: Option<Vec<String>>,
    required_confirmation_level: Option<ConfirmationLevel>,
    max_execution_steps: Option<u32>,
    supported_modalities: Option<Vec<Modality>>,
}

/// settings 中显式给出且类型匹配的键值；缺键 / 类型不符 → None（= 不覆盖内置值）。
fn parse_setting<T: serde::de::DeserializeOwned>(
    settings: Option<&serde_json::Value>,
    key: &str,
) -> Option<T> {
    settings
        .and_then(|s| s.get(key))
        .and_then(|v| serde_json::from_value(v.clone()).ok())
}

impl AgentOverlay {
    /// 由物理列 + settings JSON 构造。
    /// 键存在但类型不符 → 视为缺键（不静默抹掉内置值）。
    fn from_row(
        subject_id: i64,
        notice: Option<String>,
        color: Option<String>,
        public: Option<bool>,
        settings: Option<&serde_json::Value>,
    ) -> Self {
        let value = |key: &str| settings.and_then(|s| s.get(key)).cloned();
        Self {
            name: notice.filter(|n| !n.trim().is_empty()),
            color: color.filter(|c| !c.trim().is_empty()),
            icon: value("icon")
                .and_then(|v| v.as_str().map(str::to_string))
                .filter(|v| !v.is_empty() && v != "Bot"),
            category: value("category")
                .and_then(|v| v.as_str().map(str::to_string))
                .filter(|v| !v.is_empty() && v != "general"),
            sort_order: parse_setting(settings, "sort_order"),
            user_selectable: public,
            subject_id: Some(subject_id),
            system_prompt: parse_setting(settings, "system_prompt"),
            capabilities: parse_setting(settings, "capabilities"),
            available_tools: parse_setting(settings, "available_tools"),
            input_schema: parse_setting(settings, "input_schema"),
            output_schema: parse_setting(settings, "output_schema"),
            db_access_level: parse_setting(settings, "db_access_level"),
            allowed_schemas: parse_setting(settings, "allowed_schemas"),
            required_confirmation_level: parse_setting(settings, "required_confirmation_level"),
            max_execution_steps: parse_setting(settings, "max_execution_steps"),
            supported_modalities: parse_setting(settings, "supported_modalities"),
        }
    }

    /// 应用到内置配置：仅 `Some` 字段写入（缺键保留内置值）。
    /// 显式空数组（如 `"available_tools": []`）是**明确的撤销**，照写。
    fn apply(&self, cfg: &mut AgentConfig) {
        if let Some(v) = &self.name {
            cfg.name = v.clone();
        }
        if let Some(v) = &self.color {
            cfg.color = v.clone();
        }
        if let Some(v) = &self.icon {
            cfg.icon = v.clone();
        }
        if let Some(v) = &self.category {
            cfg.category = v.clone();
        }
        if let Some(v) = self.sort_order {
            cfg.sort_order = v;
        }
        if let Some(v) = self.user_selectable {
            cfg.user_selectable = v;
        }
        if let Some(v) = self.subject_id {
            cfg.subject_id = Some(v);
        }
        if let Some(v) = &self.system_prompt {
            cfg.system_prompt = v.clone();
        }
        if let Some(v) = &self.capabilities {
            cfg.capabilities = v.clone();
        }
        if let Some(v) = &self.available_tools {
            cfg.available_tools = v.clone();
        }
        if let Some(v) = &self.input_schema {
            cfg.input_schema = Some(v.clone());
        }
        if let Some(v) = &self.output_schema {
            cfg.output_schema = Some(v.clone());
        }
        if let Some(v) = &self.db_access_level {
            cfg.db_access_level = v.clone();
        }
        if let Some(v) = &self.allowed_schemas {
            cfg.allowed_schemas = v.clone();
        }
        if let Some(v) = &self.required_confirmation_level {
            cfg.required_confirmation_level = v.clone();
        }
        if let Some(v) = self.max_execution_steps {
            cfg.max_execution_steps = v;
        }
        if let Some(v) = &self.supported_modalities {
            cfg.supported_modalities = v.clone();
        }
    }
}

/// Agent 注册表
///
/// 运行时维护所有可用 Agent，支持动态注册和查询。
/// 内置 Agent 为硬编码实现；数据库配置通过 `load_configs_from_db` 加载后合并。
#[derive(Clone)]
pub struct AgentRegistry {
    agents: Arc<HashMap<String, Box<dyn Agent>>>,
    /// 从数据库加载的 Agent 配置稀疏覆盖（只覆盖显式给出的键）
    db_configs: Arc<HashMap<String, AgentOverlay>>,
}

impl Default for AgentRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentRegistry {
    pub fn new() -> Self {
        Self {
            agents: Arc::new(build_default_registry()),
            db_configs: Arc::new(HashMap::new()),
        }
    }

    /// 从自定义注册表创建（用于测试或扩展）
    pub fn from_map(agents: HashMap<String, Box<dyn Agent>>) -> Self {
        Self {
            agents: Arc::new(agents),
            db_configs: Arc::new(HashMap::new()),
        }
    }

    /// 从数据库加载 Agent 配置（覆盖或扩展内置 Agent 的 display 字段）
    pub async fn load_configs_from_db(&mut self, pool: &sqlx::PgPool) -> Result<(), String> {
        let rows = sqlx::query_as::<
            _,
            (
                i64,
                String,
                Option<String>,
                Option<String>,
                Option<bool>,
                Option<serde_json::Value>,
            ),
        >(
            r#"SELECT
                 a.id,
                 a.code,
                 a.notice,
                 a.t_color_,
                 (a.settings->>'public')::boolean as public,
                 a.settings as config
               FROM isahl."zc_id_empl-agent" a
               WHERE a.deleted_at IS NULL
               ORDER BY a.id ASC"#,
        )
        .fetch_all(pool)
        .await
        .map_err(|e| format!("Failed to load agent configs: {}", e))?;

        let mut configs = HashMap::new();
        for (subject_id, code, notice, color, public, config_json) in rows {
            configs.insert(
                code,
                AgentOverlay::from_row(subject_id, notice, color, public, config_json.as_ref()),
            );
        }

        self.db_configs = Arc::new(configs);
        Ok(())
    }

    /// 获取指定 Agent
    pub fn get(&self, code: &str) -> Option<&dyn Agent> {
        self.agents.get(code).map(|b| b.as_ref())
    }

    /// 获取通用回退 Agent
    pub fn fallback(&self) -> &dyn Agent {
        self.agents
            .get("general")
            .expect("general agent must exist")
            .as_ref()
    }

    /// 获取合并后的 Agent 配置（DB 行按显式键稀疏覆盖内置配置）
    pub fn merged_config(&self, code: &str) -> Option<AgentConfig> {
        self.agents.get(code).map(|agent| {
            let mut cfg = agent.config().clone();
            if let Some(overlay) = self.db_configs.get(code) {
                overlay.apply(&mut cfg);
            }
            cfg
        })
    }

    /// 列出所有可让用户手动选择的 Agent
    pub fn list_selectable(&self) -> Vec<AgentConfig> {
        let mut configs: Vec<AgentConfig> = self
            .agents
            .keys()
            .filter(|code| {
                self.merged_config(code)
                    .map(|c| c.user_selectable)
                    .unwrap_or(false)
            })
            .filter_map(|code| self.merged_config(code))
            .collect();
        configs.sort_by_key(|c| c.sort_order);
        configs
    }

    /// 列出所有 Agent 配置
    pub fn list_all_configs(&self) -> Vec<AgentConfig> {
        let mut configs: Vec<AgentConfig> = self
            .agents
            .keys()
            .filter_map(|code| self.merged_config(code))
            .collect();
        configs.sort_by_key(|c| c.sort_order);
        configs
    }

    /// 获取所有 Agent 的 code 列表
    pub fn codes(&self) -> Vec<String> {
        let mut codes: Vec<String> = self.agents.keys().cloned().collect();
        for code in self.db_configs.keys() {
            if !codes.contains(code) {
                codes.push(code.clone());
            }
        }
        codes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::ExecutionTarget;

    fn tool(name: &str) -> ToolDefinition {
        ToolDefinition {
            name: name.to_string(),
            description: String::new(),
            parameters: serde_json::json!({}),
            execution_target: ExecutionTarget::Backend,
        }
    }

    fn builtin_config() -> AgentConfig {
        AgentConfig {
            name: "内置名".into(),
            system_prompt: "内置提示".into(),
            available_tools: vec![tool("query_sql")],
            allowed_schemas: vec!["isahl".into()],
            max_execution_steps: 5,
            ..Default::default()
        }
    }

    #[test]
    fn settings_null_row_keeps_builtin_tool_grants_and_prompt() {
        // 实测 dev 库 7 个 agent 行 settings IS NULL：MUST NOT 抹掉内置授予面
        let overlay = AgentOverlay::from_row(7, Some("通用助手".into()), None, None, None);
        let mut cfg = builtin_config();
        overlay.apply(&mut cfg);
        assert_eq!(cfg.name, "通用助手");
        assert_eq!(cfg.subject_id, Some(7));
        assert_eq!(
            cfg.available_tools.len(),
            1,
            "settings 缺 available_tools MUST 保留内置授予"
        );
        assert_eq!(cfg.system_prompt, "内置提示");
        assert_eq!(cfg.allowed_schemas, vec!["isahl".to_string()]);
        assert_eq!(cfg.max_execution_steps, 5);
    }

    #[test]
    fn partial_settings_row_overrides_only_present_keys() {
        let settings = serde_json::json!({ "system_prompt": "DB 提示" });
        let overlay = AgentOverlay::from_row(9, None, None, Some(true), Some(&settings));
        let mut cfg = builtin_config();
        overlay.apply(&mut cfg);
        assert_eq!(cfg.system_prompt, "DB 提示");
        assert!(cfg.user_selectable);
        assert_eq!(cfg.name, "内置名", "未给出的键 MUST 保留内置值");
        assert_eq!(cfg.available_tools.len(), 1);
    }

    #[test]
    fn explicit_empty_tool_list_revokes_grants() {
        let settings = serde_json::json!({ "available_tools": [] });
        let overlay = AgentOverlay::from_row(1, None, None, None, Some(&settings));
        let mut cfg = builtin_config();
        overlay.apply(&mut cfg);
        assert!(
            cfg.available_tools.is_empty(),
            "显式空数组是明确撤销，照写（不回落内置）"
        );
    }

    #[test]
    fn misplaced_type_is_treated_as_absent_key() {
        let settings =
            serde_json::json!({ "max_execution_steps": "不是数字", "available_tools": 42 });
        let overlay = AgentOverlay::from_row(1, None, None, None, Some(&settings));
        let mut cfg = builtin_config();
        overlay.apply(&mut cfg);
        assert_eq!(cfg.max_execution_steps, 5);
        assert_eq!(cfg.available_tools.len(), 1);
    }

    #[test]
    fn builtin_grants_survive_registry_merge_for_settings_null_rows() {
        // 端到端（无 DB）：内置 general 的授予面经 overlay(None) 后不变
        let registry = AgentRegistry::new();
        let builtin = registry
            .agents
            .get("general")
            .expect("general builtin")
            .config()
            .clone();
        let mut merged = builtin.clone();
        AgentOverlay::from_row(3, None, None, None, None).apply(&mut merged);
        assert_eq!(merged.available_tools.len(), builtin.available_tools.len());
        assert_eq!(merged.system_prompt, builtin.system_prompt);
        assert_eq!(merged.subject_id, Some(3));
    }
}
