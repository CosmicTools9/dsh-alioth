use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::str::FromStr;

/// 模型角色（对齐 oh-my-pi 的角色化模型管理）。
///
/// 每个角色可独立配置模型与生成参数（`LLM_MODEL_<ROLE>` env），
/// 任务按角色选型组合不同模型共同完成。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ModelRole {
    /// 通用默认（= 现有 `LLM_MODEL` 语义，主模型档）
    Default,
    /// 深度推理（复杂分析/长程规划；缺省主模型 + high effort + 大 max_tokens）
    Slow,
    /// 高频任务执行（结构化提取/生成；缺省 flash 档 + low effort）
    Task,
    /// 视觉/多模态（预留槽位；缺省 flash 档，显式配置优先）
    Vision,
    /// 轻量咨询/建议（缺省 flash 档）
    Advisor,
}

impl ModelRole {
    /// env 变量名：LLM_MODEL_DEFAULT / LLM_MODEL_SLOW / …（Default 兼容 LLM_MODEL 别名）
    pub fn env_var(&self) -> &'static str {
        match self {
            Self::Default => "LLM_MODEL_DEFAULT",
            Self::Slow => "LLM_MODEL_SLOW",
            Self::Task => "LLM_MODEL_TASK",
            Self::Vision => "LLM_MODEL_VISION",
            Self::Advisor => "LLM_MODEL_ADVISOR",
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Slow => "slow",
            Self::Task => "task",
            Self::Vision => "vision",
            Self::Advisor => "advisor",
        }
    }
}

impl std::fmt::Display for ModelRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 角色绑定的模型 + 生成参数覆盖
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoleModel {
    pub model: String,
    pub generation_params: GenerationParams,
}

/// token 用量（provider 响应中的 prompt/completion tokens）
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// DeepSeek v4 / OpenAI o-series reasoning effort level.
///
/// Aligned with oh-my-pi's `Effort` enum (5 levels):
/// https://github.com/can1357/oh-my-pi — model-thinking.ts
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningEffort {
    Minimal,
    Low,
    Medium,
    High,
    #[serde(rename = "xhigh")]
    XHigh,
    /// DeepSeek v4 / Kimi K3 官方最高档 `max`（独立档位，非 xhigh 别名）
    #[serde(rename = "max")]
    Max,
}

impl std::fmt::Display for ReasoningEffort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Minimal => write!(f, "minimal"),
            Self::Low => write!(f, "low"),
            Self::Medium => write!(f, "medium"),
            Self::High => write!(f, "high"),
            Self::XHigh => write!(f, "xhigh"),
            Self::Max => write!(f, "max"),
        }
    }
}

impl ReasoningEffort {
    /// Parse a string from the API / env config into the typed enum.
    /// Returns `None` for unrecognised values.
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "minimal" => Some(Self::Minimal),
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "high" => Some(Self::High),
            "xhigh" => Some(Self::XHigh),
            "max" => Some(Self::Max),
            _ => None,
        }
    }

    /// Returns the API-level string value for the reasoning effort.
    /// Returns `None` for variants that should not be sent.
    ///
    /// DeepSeek v4 / Kimi K3 官方档位 low/high/max：
    /// minimal → "low" (lowest available), xhigh → "high" (high 别名),
    /// medium → None (let API apply its default, preserving prefix cache).
    pub fn as_api_value(&self) -> Option<&'static str> {
        match self {
            Self::Minimal => Some("low"),
            Self::Low => Some("low"),
            Self::Medium => None, // default; omit to preserve prefix cache
            Self::High => Some("high"),
            Self::XHigh => Some("high"),
            Self::Max => Some("max"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmServiceConfig {
    pub provider: LlmProvider,
    pub api_key: String,
    pub model: String,
    pub flash_model: String,
    pub base_url: Option<String>,
    pub timeout_seconds: u64,
    pub max_retries: u32,
    pub generation_params: GenerationParams,
    /// 角色化模型表（default/slow/task/vision/advisor）。
    /// 未列出的角色在 from_env 时按缺省映射补齐；序列化缺省为空表。
    #[serde(default)]
    pub roles: HashMap<ModelRole, RoleModel>,
}

impl LlmServiceConfig {
    /// 从环境变量解析 LLM 配置
    ///
    /// 默认适配当前 `LLM_PROVIDER` 的模型名和 API 地址，
    /// 可通过 `LLM_MODEL` / `LLM_FLASH_MODEL` / `LLM_BASE_URL` 单独覆盖。
    /// 角色模型：`LLM_MODEL_DEFAULT/SLOW/TASK/VISION/ADVISOR`（见 [`ModelRole::env_var`]），
    /// 未配置的角色按缺省映射：slow→主模型档（high effort/16384 max_tokens），
    /// task/vision/advisor→flash 档（task 附 low effort）。
    pub fn from_env() -> Self {
        let provider = LlmProvider::from_env();
        let default_model = provider.default_model().to_string();
        let default_flash = provider.default_flash_model().to_string();
        let default_base_url = provider.default_base_url().to_string();
        let generation_params = GenerationParams {
            temperature: std::env::var("LLM_TEMPERATURE")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(1.0),
            max_tokens: std::env::var("LLM_MAX_TOKENS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(DEFAULT_MAX_TOKENS),
            top_p: 1.0,
            frequency_penalty: 0.0,
            presence_penalty: 0.0,
            reasoning_effort: std::env::var("LLM_REASONING_EFFORT")
                .ok()
                .and_then(|s| ReasoningEffort::parse(&s))
                .unwrap_or(ReasoningEffort::Medium),
            response_format: std::env::var("LLM_RESPONSE_FORMAT").ok(),
            thinking: std::env::var("LLM_THINKING").ok(),
            service_tier: std::env::var("LLM_SERVICE_TIER").ok(),
            reasoning_split: std::env::var("LLM_REASONING_SPLIT")
                .ok()
                .and_then(|s| s.parse().ok()),
        };
        let model = std::env::var("LLM_MODEL").unwrap_or(default_model);
        let flash_model = std::env::var("LLM_FLASH_MODEL").unwrap_or(default_flash);
        Self {
            provider,
            api_key: std::env::var("LLM_API_KEY").unwrap_or_default(),
            model: model.clone(),
            flash_model: flash_model.clone(),
            base_url: std::env::var("LLM_BASE_URL")
                .ok()
                .or(Some(default_base_url)),
            timeout_seconds: std::env::var("LLM_TIMEOUT_SECONDS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(300),
            max_retries: std::env::var("LLM_MAX_RETRIES")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(3),
            generation_params,
            roles: Self::resolve_roles(&model, &flash_model),
        }
    }

    /// 角色缺省映射 + env 显式覆盖（`LLM_MODEL_<ROLE>`）。
    /// 缺省：slow→主模型（high effort / 16384 max_tokens）、task→flash（low effort）、
    /// vision/advisor→flash；default→主模型（兼容 LLM_MODEL）。
    fn resolve_roles(model: &str, flash_model: &str) -> HashMap<ModelRole, RoleModel> {
        use ReasoningEffort::{High, Low};

        let mut roles = HashMap::new();
        // slow：深度推理，主模型档 + high effort + 大 max_tokens
        roles.insert(
            ModelRole::Slow,
            RoleModel {
                model: env_or(&ModelRole::Slow, model),
                generation_params: GenerationParams {
                    reasoning_effort: High,
                    max_tokens: 16384,
                    ..Default::default()
                },
            },
        );
        // task：高频执行，flash 档 + low effort
        roles.insert(
            ModelRole::Task,
            RoleModel {
                model: env_or(&ModelRole::Task, flash_model),
                generation_params: GenerationParams {
                    reasoning_effort: Low,
                    ..Default::default()
                },
            },
        );
        // vision：预留槽位，缺省 flash 档（显式 LLM_MODEL_VISION 优先）
        roles.insert(
            ModelRole::Vision,
            RoleModel {
                model: env_or(&ModelRole::Vision, flash_model),
                generation_params: GenerationParams::default(),
            },
        );
        // advisor：轻量咨询，缺省 flash 档
        roles.insert(
            ModelRole::Advisor,
            RoleModel {
                model: env_or(&ModelRole::Advisor, flash_model),
                generation_params: GenerationParams::default(),
            },
        );
        // default：主模型档（兼容 LLM_MODEL；LLM_MODEL_DEFAULT 显式优先）
        roles.insert(
            ModelRole::Default,
            RoleModel {
                model: env_or(&ModelRole::Default, model),
                generation_params: GenerationParams::default(),
            },
        );
        roles
    }
}

fn env_or(role: &ModelRole, fallback: &str) -> String {
    std::env::var(role.env_var()).unwrap_or_else(|_| fallback.to_string())
}

impl LlmProvider {
    /// 从环境变量 LLM_PROVIDER 解析，支持 kimi / minimax / glm / deepseek，默认 DeepSeek
    pub fn from_env() -> Self {
        match std::env::var("LLM_PROVIDER")
            .unwrap_or_default()
            .to_lowercase()
            .as_str()
        {
            "kimi" => Self::Kimi,
            "minimax" => Self::MiniMax,
            "glm" | "zhipu" => Self::Glm,
            _ => Self::DeepSeek,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::DeepSeek => "deepseek",
            Self::Kimi => "kimi",
            Self::MiniMax => "minimax",
            Self::Glm => "glm",
        }
    }

    /// 默认旗舰模型（Pro 档）
    pub fn default_model(&self) -> &'static str {
        match self {
            Self::DeepSeek => "deepseek-v4-pro",
            Self::Kimi => "k3-256k",
            Self::MiniMax => "MiniMax-M3",
            // 官方套餐概览（docs.bigmodel.cn/cn/coding-plan/overview）：
            // 所有 Coding Plan 套餐支持 GLM-5.3 / GLM-5.3-Flash（2026-08）
            Self::Glm => "glm-5.3",
        }
    }
    /// 默认经济模型（Flash 档）
    pub fn default_flash_model(&self) -> &'static str {
        match self {
            Self::DeepSeek => "deepseek-v4-flash",
            Self::Kimi => "kimi-for-coding",
            Self::MiniMax => "MiniMax-M2.7",
            Self::Glm => "glm-5.3-flash",
        }
    }

    /// 默认 API 地址
    pub fn default_base_url(&self) -> &'static str {
        match self {
            Self::DeepSeek => "https://api.deepseek.com",
            Self::Kimi => "https://api.kimi.com/coding",
            Self::MiniMax => "https://api.minimaxi.com",
            // Coding Plan 端点（OpenAI 兼容：{base}/chat/completions）
            Self::Glm => "https://open.bigmodel.cn/api/coding/paas/v4",
        }
    }
}

impl FromStr for LlmProvider {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "deepseek" => Ok(Self::DeepSeek),
            "kimi" => Ok(Self::Kimi),
            "minimax" => Ok(Self::MiniMax),
            "glm" | "zhipu" => Ok(Self::Glm),
            _ => Err(format!(
                "Unknown LLM provider '{}'. Supported: deepseek, kimi, minimax, glm",
                s
            )),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum LlmProvider {
    /// DeepSeek (OpenAI-compatible API)
    #[default]
    DeepSeek,
    /// Kimi / Moonshot (OpenAI-compatible API)
    Kimi,
    /// MiniMax (OpenAI-compatible API)
    MiniMax,
    /// GLM / 智谱（OpenAI-compatible API；Coding Plan 端点默认，
    /// 通用 API 经 base_url 覆盖 `https://open.bigmodel.cn/api/paas/v4`）
    Glm,
}

/// 默认生成预算（LLM_MAX_TOKENS / settings.max_tokens 未配置时生效）。
/// DeepSeek thinking mode 默认开启且 effort 实际为 high（medium→high 映射），
/// 思考与正文共享此预算——4096 会被高 effort 思考整段吃掉导致空回复
/// （fix-chat-ai-empty-reply），故与 slow 档位对齐取 16384。
pub const DEFAULT_MAX_TOKENS: u64 = 16384;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerationParams {
    pub temperature: f64,
    pub max_tokens: u64,
    pub top_p: f64,
    pub frequency_penalty: f64,
    pub presence_penalty: f64,
    /// DeepSeek reasoning effort (low / medium / high).
    #[serde(default = "default_reasoning_effort")]
    pub reasoning_effort: ReasoningEffort,
    /// OpenAI-compatible response_format, e.g. `"json_object"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_format: Option<String>,
    /// 思考模式控制:DeepSeek `enabled|disabled`、MiniMax-M3 `adaptive|disabled`、
    /// Kimi K2.7 固定 `enabled`。None = 不显式发送(各家默认)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<String>,
    /// MiniMax 专属:请求准入服务层级 standard|priority（默认 standard）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<String>,
    /// MiniMax-M3 专属:输出拆分开关。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_split: Option<bool>,
}

fn default_reasoning_effort() -> ReasoningEffort {
    ReasoningEffort::Medium
}

/// Get the max_tokens ceiling for a given task type (oh-my-pi inspired).
/// Returns a recommended upper bound, not a hard limit.
pub fn recommended_max_tokens(task: &str) -> u64 {
    match task {
        "ontology_planning" => 16384,
        "code_generation" => 32768,
        "semantic_repair" => 4096,
        "format_correction" => 8192,
        _ => 4096,
    }
}

impl GenerationParams {
    /// 将可选覆盖参数合并到此配置中，返回新实例。
    /// 用于 Harness 层按 TaskType 动态调节 temperature/effort/tokens。
    pub fn with_overrides(
        &self,
        temperature: Option<f64>,
        max_tokens: Option<u64>,
        reasoning_effort: Option<&str>,
        response_format: Option<&str>,
    ) -> Self {
        let mut p = self.clone();
        if let Some(t) = temperature {
            p.temperature = t;
        }
        if let Some(t) = max_tokens {
            p.max_tokens = t;
        }
        if let Some(re) = reasoning_effort {
            match re {
                "minimal" => p.reasoning_effort = ReasoningEffort::Minimal,
                "low" => p.reasoning_effort = ReasoningEffort::Low,
                "medium" => p.reasoning_effort = ReasoningEffort::Medium,
                "high" => p.reasoning_effort = ReasoningEffort::High,
                "xhigh" => p.reasoning_effort = ReasoningEffort::XHigh,
                "max" => p.reasoning_effort = ReasoningEffort::Max,
                _ => {}
            }
        }
        if let Some(rf) = response_format {
            p.response_format = Some(rf.to_string());
        }
        p
    }
}

impl Default for GenerationParams {
    fn default() -> Self {
        Self {
            temperature: 1.0,
            max_tokens: DEFAULT_MAX_TOKENS,
            top_p: 1.0,
            frequency_penalty: 0.0,
            presence_penalty: 0.0,
            reasoning_effort: ReasoningEffort::Medium,
            response_format: None,
            thinking: None,
            service_tier: None,
            reasoning_split: None,
        }
    }
}

/// LLM 工具定义（Function Calling Schema）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// LLM 工具调用请求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

/// LLM 响应（文本或工具调用）
#[derive(Debug, Clone)]
pub enum LlmResponse {
    Text(String),
    ToolCalls(Vec<ToolCall>),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_env<K: AsRef<std::ffi::OsStr>, V: AsRef<std::ffi::OsStr>>(
        key: K,
        val: Option<V>,
        f: impl FnOnce(),
    ) {
        let prev = std::env::var_os(key.as_ref());
        match val {
            Some(v) => std::env::set_var(key.as_ref(), v),
            None => std::env::remove_var(key.as_ref()),
        }
        f();
        match prev {
            Some(p) => std::env::set_var(key.as_ref(), p),
            None => std::env::remove_var(key.as_ref()),
        }
    }

    #[test]
    fn resolve_roles_maps_defaults_to_tiers() {
        let roles = LlmServiceConfig::resolve_roles("pro-model", "flash-model");
        assert_eq!(roles[&ModelRole::Default].model, "pro-model");
        assert_eq!(roles[&ModelRole::Slow].model, "pro-model");
        assert_eq!(roles[&ModelRole::Task].model, "flash-model");
        assert_eq!(roles[&ModelRole::Vision].model, "flash-model");
        assert_eq!(roles[&ModelRole::Advisor].model, "flash-model");
        // slow 高推理档 / task 低推理档
        assert_eq!(
            roles[&ModelRole::Slow].generation_params.reasoning_effort,
            ReasoningEffort::High
        );
        assert_eq!(roles[&ModelRole::Slow].generation_params.max_tokens, 16384);
        assert_eq!(
            roles[&ModelRole::Task].generation_params.reasoning_effort,
            ReasoningEffort::Low
        );
        assert_eq!(
            roles[&ModelRole::Default]
                .generation_params
                .reasoning_effort,
            ReasoningEffort::Medium
        );
    }

    #[test]
    fn resolve_roles_honors_env_override() {
        with_env("LLM_MODEL_TASK", Some("task-model"), || {
            let roles = LlmServiceConfig::resolve_roles("pro-model", "flash-model");
            assert_eq!(roles[&ModelRole::Task].model, "task-model");
            assert_eq!(roles[&ModelRole::Slow].model, "pro-model"); // 未覆盖
        });
        with_env("LLM_MODEL_VISION", Some("vision-model"), || {
            let roles = LlmServiceConfig::resolve_roles("pro-model", "flash-model");
            assert_eq!(roles[&ModelRole::Vision].model, "vision-model");
        });
    }

    #[test]
    fn role_env_var_names() {
        assert_eq!(ModelRole::Default.env_var(), "LLM_MODEL_DEFAULT");
        assert_eq!(ModelRole::Slow.env_var(), "LLM_MODEL_SLOW");
        assert_eq!(ModelRole::Task.env_var(), "LLM_MODEL_TASK");
        assert_eq!(ModelRole::Vision.env_var(), "LLM_MODEL_VISION");
        assert_eq!(ModelRole::Advisor.env_var(), "LLM_MODEL_ADVISOR");
    }
}
