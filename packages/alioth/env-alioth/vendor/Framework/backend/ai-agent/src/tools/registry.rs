//! 工具注册表

use super::{ToolCall, ToolContext, ToolResult};
use crate::agents::ToolDefinition;
use crate::tools::executor::{
    ExecuteActionTool, QueryDocumentTool, QuerySchemaTool, QuerySqlTool, ValidateFormTool,
};
use std::collections::HashMap;

/// 工具注册表
pub struct ToolRegistry {
    definitions: HashMap<String, ToolDefinition>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        let mut definitions = HashMap::new();
        definitions.insert("query_sql".to_string(), QuerySqlTool::definition());
        definitions.insert("query_schema".to_string(), QuerySchemaTool::definition());
        definitions.insert("validate_form".to_string(), ValidateFormTool::definition());
        definitions.insert(
            "query_document".to_string(),
            QueryDocumentTool::definition(),
        );
        definitions.insert(
            "execute_action".to_string(),
            ExecuteActionTool::definition(),
        );

        Self { definitions }
    }

    /// 获取工具定义列表（用于传递给 LLM）
    pub fn list_tools(&self) -> Vec<ToolDefinition> {
        self.definitions.values().cloned().collect()
    }

    /// 根据名称查找工具定义
    pub fn get_definition(&self, name: &str) -> Option<&ToolDefinition> {
        self.definitions.get(name)
    }

    /// 执行指定工具
    pub async fn execute(&self, call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult, String> {
        match call.name.as_str() {
            "query_sql" => QuerySqlTool::execute(call, ctx).await,
            "query_schema" => QuerySchemaTool::execute(call, ctx).await,
            "validate_form" => ValidateFormTool::execute(call, ctx).await,
            "query_document" => QueryDocumentTool::execute(call, ctx).await,
            "execute_action" => ExecuteActionTool::execute(call, ctx).await,
            _ => Ok(ToolResult {
                tool_call_id: call.id.clone(),
                name: call.name.clone(),
                success: false,
                output: serde_json::Value::Null,
                error: Some(format!("Unknown tool: {}", call.name)),
            }),
        }
    }

    /// 将 ai-agent 的 ToolDefinition 转换为 llm 的 ToolDefinition
    pub fn to_llm_tools(&self) -> Vec<llm::ToolDefinition> {
        self.list_tools()
            .into_iter()
            .map(|t| llm::ToolDefinition {
                name: t.name,
                description: t.description,
                parameters: t.parameters,
            })
            .collect()
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}
