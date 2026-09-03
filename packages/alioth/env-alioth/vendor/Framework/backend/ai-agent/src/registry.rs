use std::collections::HashMap;
use std::sync::Arc;

use crate::agents::{build_default_registry, Agent, AgentConfig};

/// Agent 注册表
///
/// 运行时维护所有可用 Agent，支持动态注册和查询。
/// 内置 Agent 为硬编码实现；数据库配置通过 `load_configs_from_db` 加载后合并。
#[derive(Clone)]
pub struct AgentRegistry {
    agents: Arc<HashMap<String, Box<dyn Agent>>>,
    /// 从数据库加载的 Agent 配置（覆盖内置配置的 display 字段）
    db_configs: Arc<HashMap<String, AgentConfig>>,
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
                String,
                Option<String>,
                Option<String>,
                Option<bool>,
                Option<serde_json::Value>,
            ),
        >(
            r#"SELECT
                 a.code,
                 a.notice,
                 a.t_color_,
                 COALESCE((a.settings->>'public')::boolean, false) as public,
                 a.settings as config
               FROM isahl."zc_id_empl-agent" a
               WHERE a.deleted_at IS NULL
               ORDER BY a.id ASC"#,
        )
        .fetch_all(pool)
        .await
        .map_err(|e| format!("Failed to load agent configs: {}", e))?;

        let mut configs = HashMap::new();
        for (code, notice, color, public, config_json) in rows {
            let mut cfg = AgentConfig {
                code: code.clone(),
                name: notice.unwrap_or_default(),
                color: color.unwrap_or_default(),
                sort_order: 0,
                user_selectable: public.unwrap_or(false),
                ..Default::default()
            };
            // 解析 config JSONB
            if let Some(json) = config_json {
                if let Ok(parsed) = serde_json::from_value::<AgentConfig>(json.clone()) {
                    // 合并解析结果（保持 code/notice 等物理列优先）
                    cfg.system_prompt = parsed.system_prompt;
                    cfg.capabilities = parsed.capabilities;
                    cfg.available_tools = parsed.available_tools;
                    cfg.input_schema = parsed.input_schema;
                    cfg.output_schema = parsed.output_schema;
                    cfg.db_access_level = parsed.db_access_level;
                    cfg.allowed_schemas = parsed.allowed_schemas;
                    cfg.required_confirmation_level = parsed.required_confirmation_level;
                    cfg.max_execution_steps = parsed.max_execution_steps;
                    cfg.supported_modalities = parsed.supported_modalities;
                    if !parsed.icon.is_empty() && parsed.icon != "Bot" {
                        cfg.icon = parsed.icon;
                    }
                    if !parsed.category.is_empty() && parsed.category != "general" {
                        cfg.category = parsed.category;
                    }
                }
            }
            configs.insert(code, cfg);
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

    /// 获取合并后的 Agent 配置（数据库配置优先覆盖内置配置的 display 字段）
    pub fn merged_config(&self, code: &str) -> Option<AgentConfig> {
        self.agents.get(code).map(|agent| {
            let mut cfg = agent.config().clone();
            if let Some(db_cfg) = self.db_configs.get(code) {
                cfg.name = db_cfg.name.clone();
                cfg.description = db_cfg.description.clone();
                cfg.icon = db_cfg.icon.clone();
                cfg.color = db_cfg.color.clone();
                cfg.category = db_cfg.category.clone();
                cfg.sort_order = db_cfg.sort_order;
                cfg.user_selectable = db_cfg.user_selectable;
                cfg.system_prompt = db_cfg.system_prompt.clone();
                cfg.available_tools = db_cfg.available_tools.clone();
                cfg.db_access_level = db_cfg.db_access_level.clone();
                cfg.allowed_schemas = db_cfg.allowed_schemas.clone();
                cfg.required_confirmation_level = db_cfg.required_confirmation_level.clone();
                cfg.max_execution_steps = db_cfg.max_execution_steps;
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
