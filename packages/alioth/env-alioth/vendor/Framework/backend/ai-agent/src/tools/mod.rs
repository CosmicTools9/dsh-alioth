//! Agent 工具调用基础设施
//!
//! 工具是 Agent 感知-推理-行动闭环中的「行动」层。
//! 每个工具是一个可执行函数，由 LLM 根据上下文决定是否调用。

pub mod executor;
pub mod registry;
pub mod row_value;

use crate::agents::ToolDefinition;
use serde::{Deserialize, Serialize};

/// 工具调用请求（由 LLM 生成）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

/// 工具执行结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub tool_call_id: String,
    pub name: String,
    pub success: bool,
    pub output: serde_json::Value,
    pub error: Option<String>,
}

/// 工具执行器 trait
#[async_trait::async_trait]
pub trait ToolExecutor: Send + Sync {
    /// 执行单个工具调用
    async fn execute(&self, call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult, String>;

    /// 列出此执行器支持的所有工具
    fn list_tools(&self) -> Vec<ToolDefinition>;
}

/// 工具执行上下文
#[derive(Debug, Clone)]
pub struct ToolContext {
    pub session_id: i64,
    pub user_id: Option<i64>,
    pub db_pool: sqlx::PgPool,
    pub allowed_schemas: Vec<String>,
}
