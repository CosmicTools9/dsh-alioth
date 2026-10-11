//! 工具注册表
//!
//! **单一工具名清单**（change `retire-tool-passthrough`）：工具名 / LLM 定义 /
//! 派发执行三者同源于下方 `REGISTRY` 表——本文件 MUST NOT 出现第二份等价的
//! 工具名清单（旧「N 次 `insert` + 同名 `match`」透传层已删）。

use super::{ToolCall, ToolContext, ToolResult};
use crate::agents::ToolDefinition;
use crate::tools::executor::{
    DmnDraftTool, DmnReviseTool, ExecuteActionTool, NgacCheckAccessTool, NgacDryRunTool,
    NgacExplainPathsTool, NgacPolicyDraftTool, NgacStrategyDraftTool, QuerySchemaTool,
    QuerySqlTool, SystemConfigDraftTool, ValidateFormTool,
};
use std::future::Future;
use std::pin::Pin;

/// 工具执行返回的装箱 future（表内函数指针的统一签名）。
type ToolFuture<'a> = Pin<Box<dyn Future<Output = Result<ToolResult, String>> + Send + 'a>>;

/// 单个工具条目：名称 / LLM 定义 / 派发执行——三者同源，无独立清单。
struct ToolEntry {
    name: &'static str,
    definition: fn() -> ToolDefinition,
    execute: for<'a> fn(&'a ToolCall, &'a ToolContext) -> ToolFuture<'a>,
}

/// 条目构造：`$name` 与 `$tool` 在本宏内**配对一次** ⇒ 工具名清单只出现一次。
macro_rules! tool_entry {
    ($name:literal, $tool:ty) => {{
        fn execute_tool<'a>(call: &'a ToolCall, ctx: &'a ToolContext) -> ToolFuture<'a> {
            Box::pin(<$tool>::execute(call, ctx))
        }
        ToolEntry {
            name: $name,
            definition: <$tool>::definition,
            execute: execute_tool,
        }
    }};
}

/// 工具注册表（唯一真相源）。
const REGISTRY: &[ToolEntry] = &[
    tool_entry!("query_sql", QuerySqlTool),
    tool_entry!("query_schema", QuerySchemaTool),
    tool_entry!("validate_form", ValidateFormTool),
    tool_entry!("execute_action", ExecuteActionTool),
    tool_entry!("dmn_draft", DmnDraftTool),
    tool_entry!("dmn_revise", DmnReviseTool),
    tool_entry!("ngac_policy_draft", NgacPolicyDraftTool),
    tool_entry!("ngac_strategy_draft", NgacStrategyDraftTool),
    tool_entry!("ngac_dry_run", NgacDryRunTool),
    tool_entry!("ngac_explain_paths", NgacExplainPathsTool),
    tool_entry!("ngac_check_access", NgacCheckAccessTool),
    tool_entry!("system_config_draft", SystemConfigDraftTool),
];

/// 工具注册表。
pub struct ToolRegistry;

impl ToolRegistry {
    pub fn new() -> Self {
        Self
    }

    /// 按名称取注册条目。
    fn entry(name: &str) -> Option<&'static ToolEntry> {
        REGISTRY.iter().find(|e| e.name == name)
    }

    /// 获取工具定义列表（用于传递给 LLM）。
    pub fn list_tools(&self) -> Vec<ToolDefinition> {
        REGISTRY.iter().map(|e| (e.definition)()).collect()
    }

    /// 执行指定工具。
    pub async fn execute(&self, call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult, String> {
        match Self::entry(&call.name) {
            Some(e) => (e.execute)(call, ctx).await,
            None => Ok(ToolResult {
                tool_call_id: call.id.clone(),
                name: call.name.clone(),
                success: false,
                output: serde_json::Value::Null,
                error: Some(format!("Unknown tool: {}", call.name)),
            }),
        }
    }

    /// 将 ai-agent 的 ToolDefinition 转换为 llm 的 ToolDefinition。
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 注册表名称清单唯一且与派发面一致：每个名称都能解析到定义。
    #[test]
    fn registry_is_single_source_of_tool_names() {
        assert_eq!(
            REGISTRY.len(),
            12,
            "工具数 = 11 + 1（ngac_check_access 只读检查工具，wire-ngac-zero-consumer-faces）"
        );
        let mut names: Vec<&str> = REGISTRY.iter().map(|e| e.name).collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(before, names.len(), "工具名 MUST 唯一");
        // list_tools 与 REGISTRY 同源（条数一致）
        assert_eq!(ToolRegistry::new().list_tools().len(), REGISTRY.len());
    }
}
