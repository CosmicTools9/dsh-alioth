use async_trait::async_trait;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Agent 执行结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentResult {
    pub content: String,
    /// 结构化数据（如表单字段、分析结果、流程步骤等）
    pub structured: Option<serde_json::Value>,
    /// 是否需要用户确认/补充
    pub requires_input: bool,
    /// 建议的下一步操作
    pub suggested_actions: Vec<String>,

    // --- 新增 ---
    /// 生成此结果的 Agent 编码
    pub agent_code: String,
    /// 结果置信度（0.0-1.0）
    pub confidence: f64,
    /// Token 消耗统计
    pub token_usage: Option<TokenUsage>,
    /// 路由决策信息（供审计与调试）
    pub routing_info: Option<RoutingInfo>,
}

impl AgentResult {
    /// 便捷构造器（内置 Agent 使用）
    pub fn new(
        agent_code: impl Into<String>,
        content: impl Into<String>,
        structured: Option<serde_json::Value>,
        requires_input: bool,
        suggested_actions: Vec<String>,
    ) -> Self {
        Self {
            agent_code: agent_code.into(),
            content: content.into(),
            structured,
            requires_input,
            suggested_actions,
            confidence: 0.9,
            token_usage: None,
            routing_info: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingInfo {
    pub agent_code: String,
    pub confidence: f64,
    pub reason: String,
    pub level: String, // "l1_rule" | "l2_llm" | "fallback"
}

/// Agent 能力标签
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum AgentCapability {
    /// 多模态信息理解（文本、图片、表格、文档）
    MultimodalUnderstanding,
    /// 表单识别与自动填写
    FormFilling,
    /// 数据分析与可视化建议
    DataAnalysis,
    /// 业务流程设计
    FlowDesign,
    /// 仿真与假设验证
    Simulation,
    /// 单据自动化处理
    DocumentAutomation,
    /// 通用对话
    GeneralConversation,
}

/// 工具定义（Function Calling Schema）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value, // JSON Schema
    /// 执行目标：backend（后端执行）或 frontend（前端执行）
    pub execution_target: ExecutionTarget,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionTarget {
    Backend,
    Frontend,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolChoice {
    Auto,     // LLM 自行决定是否调用
    Required, // 强制至少调用一次
    None,     // 禁止调用
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DbAccessLevel {
    None,             // 不访问数据库
    ReadOnly,         // 只读（SELECT）
    SchemaRestricted, // 白名单 schema 内读写
    ReadWrite,        // 全权限（危险，仅内部 Agent）
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfirmationLevel {
    None,     // 自动执行
    Low,      // 简单确认提示
    High,     // 需显式确认
    Critical, // 需二次验证（如 MFA、审批流）
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Modality {
    Text,
    Image,
    Document,
    Audio,
    Table,
}

/// Agent 配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    // --- 已有字段 ---
    pub code: String,
    pub name: String,
    pub description: String,
    pub capabilities: Vec<AgentCapability>,
    /// 系统提示词
    pub system_prompt: String,
    /// 专用模型配置（覆盖全局默认）
    pub model_override: Option<String>,
    /// 专用生成参数（覆盖全局默认）
    pub generation_params: Option<llm::GenerationParams>,
    /// 是否允许用户显式选择
    pub user_selectable: bool,
    /// 排序权重（越大越靠前）
    pub sort_order: i32,

    // --- 新增字段 ---
    /// UI 图标（Lucide 图标名）
    pub icon: String,
    /// UI 颜色（来自 t_color_ 物理列）
    pub color: String,
    /// 分类
    pub category: String,

    /// 输入 JSON Schema
    pub input_schema: Option<serde_json::Value>,
    /// 输出 JSON Schema
    pub output_schema: Option<serde_json::Value>,

    /// 可用工具声明
    pub available_tools: Vec<ToolDefinition>,
    /// 工具选择策略
    pub tool_choice: ToolChoice,

    /// 数据库访问级别
    pub db_access_level: DbAccessLevel,
    /// 允许访问的 schema 白名单
    pub allowed_schemas: Vec<String>,

    /// 需要用户确认级别
    pub required_confirmation_level: ConfirmationLevel,
    /// 最大执行步数
    pub max_execution_steps: u32,

    /// 支持的多模态类型
    pub supported_modalities: Vec<Modality>,

    /// 路由评分权重（用于 KeywordStrategy）
    pub routing_weights: RoutingWeights,

    /// 默认是否需要用户输入/确认
    #[serde(default)]
    pub requires_input_default: bool,
    /// 建议的下一步操作列表
    #[serde(default)]
    pub suggested_actions: Vec<String>,
}

/// Agent 路由评分权重
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingWeights {
    /// 关键词匹配得分
    pub keyword_match: f64,
    /// 正则模式匹配得分
    pub pattern_match: f64,
    /// 页面上下文建议加成
    pub page_context_bonus: f64,
    /// 最大可能得分（用于标准化）
    pub max_possible_score: f64,
    /// 最低通过阈值（0-1）
    pub threshold: f64,
}

impl Default for RoutingWeights {
    fn default() -> Self {
        Self {
            keyword_match: 1.0,
            pattern_match: 2.0,
            page_context_bonus: 3.0,
            max_possible_score: 10.0,
            threshold: 0.5,
        }
    }
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            code: String::new(),
            name: String::new(),
            description: String::new(),
            capabilities: vec![],
            system_prompt: String::new(),
            model_override: None,
            generation_params: None,
            user_selectable: true,
            sort_order: 0,
            icon: "Bot".to_string(),
            color: "#6366f1".to_string(),
            category: "general".to_string(),
            input_schema: None,
            output_schema: None,
            available_tools: vec![],
            tool_choice: ToolChoice::Auto,
            db_access_level: DbAccessLevel::None,
            allowed_schemas: vec![],
            required_confirmation_level: ConfirmationLevel::None,
            max_execution_steps: 5,
            supported_modalities: vec![Modality::Text],
            routing_weights: RoutingWeights::default(),
            requires_input_default: false,
            suggested_actions: vec![],
        }
    }
}

/// Agent 执行上下文
#[derive(Debug, Clone)]
pub struct AgentContext {
    pub session_id: i64,
    pub user_id: Option<i64>,
    pub locale: String,
    pub user_message: String,
    pub conversation_history: Vec<(String, String)>, // (role, content)
    pub page_context: Option<serde_json::Value>,
    pub attachments: Vec<Attachment>,

    // --- 新增 ---
    /// 会话级 Agent 状态机（工作流步骤、已收集字段等）
    pub session_state: Option<serde_json::Value>,
    /// 当前工作流步骤标识
    pub workflow_step: Option<String>,
    /// 目标表单 Schema（FormFillingAgent 使用）
    pub form_schema: Option<serde_json::Value>,
    /// 可用数据表目录（DataAnalysisAgent 使用）
    pub schema_catalog: Option<serde_json::Value>,
    /// 业务规则快照（SimulationAgent / DocumentAutomationAgent 使用）
    pub business_rules: Option<serde_json::Value>,
}

impl Default for AgentContext {
    fn default() -> Self {
        Self {
            session_id: 0,
            user_id: None,
            locale: "zh-CN".to_string(),
            user_message: String::new(),
            conversation_history: vec![],
            page_context: None,
            attachments: vec![],
            session_state: None,
            workflow_step: None,
            form_schema: None,
            schema_catalog: None,
            business_rules: None,
        }
    }
}

/// 消息附件
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    pub mime_type: String,
    pub url: String,
    pub filename: String,
    pub metadata: Option<serde_json::Value>,
}

/// Agent 核心 trait
#[async_trait]
pub trait Agent: Send + Sync {
    /// Agent 配置
    fn config(&self) -> &AgentConfig;

    /// 返回该 Agent 用于意图识别的关键词（用于 L1 规则路由）
    fn intent_keywords(&self) -> Vec<String> {
        vec![]
    }

    /// 返回该 Agent 用于意图识别的正则表达式模式（用于 L1 规则路由）
    fn intent_patterns(&self) -> Vec<Regex> {
        vec![]
    }

    // --- 新增 ---
    /// 返回此 Agent 注册的工具执行器
    fn tool_executor(&self) -> Option<&dyn ToolExecutor> {
        None
    }

    /// 验证输出是否符合 output_schema（如配置了 schema）
    fn validate_output(&self, value: &serde_json::Value) -> Result<(), String> {
        // 若 config.output_schema 存在，用 jsonschema 验证
        // TODO: 集成 jsonschema crate
        let _ = value;
        Ok(())
    }
}

// ToolExecutor 由 tools 模块定义，此处 re-export
pub use crate::tools::ToolExecutor;

/// 从文本中提取 JSON 代码块
pub fn extract_json_block(text: &str) -> Option<serde_json::Value> {
    // 尝试匹配 ```json ... ```
    if let Some(start) = text.find("```json") {
        let rest = &text[start + 7..];
        if let Some(end) = rest.find("```") {
            let json_str = rest[..end].trim();
            return serde_json::from_str(json_str).ok();
        }
    }
    // 尝试匹配 ``` ... ```（无语言标识）
    if let Some(start) = text.find("```") {
        let rest = &text[start + 3..];
        if let Some(end) = rest.find("```") {
            let json_str = rest[..end].trim();
            if json_str.starts_with('{') || json_str.starts_with('[') {
                return serde_json::from_str(json_str).ok();
            }
        }
    }
    // 尝试直接解析整个文本
    serde_json::from_str(text).ok()
}

// 子模块
pub mod data_analysis;
pub mod document_automation;
pub mod flow_design;
pub mod form_filling;
pub mod general;
pub mod simulation;
pub mod tool_orchestrator;

use data_analysis::DataAnalysisAgent;
use document_automation::DocumentAutomationAgent;
use flow_design::FlowDesignAgent;
use form_filling::FormFillingAgent;
use general::GeneralAssistantAgent;
use simulation::SimulationAgent;

/// 构建默认 Agent Registry 并注册所有内置 Agent
pub fn build_default_registry() -> HashMap<String, Box<dyn Agent>> {
    let mut registry: HashMap<String, Box<dyn Agent>> = HashMap::new();

    registry.insert(
        "form_filling".to_string(),
        Box::new(FormFillingAgent::new()),
    );
    registry.insert(
        "data_analysis".to_string(),
        Box::new(DataAnalysisAgent::new()),
    );
    registry.insert("flow_design".to_string(), Box::new(FlowDesignAgent::new()));
    registry.insert("simulation".to_string(), Box::new(SimulationAgent::new()));
    registry.insert(
        "document_automation".to_string(),
        Box::new(DocumentAutomationAgent::new()),
    );
    registry.insert(
        "general".to_string(),
        Box::new(GeneralAssistantAgent::new()),
    );

    registry
}
