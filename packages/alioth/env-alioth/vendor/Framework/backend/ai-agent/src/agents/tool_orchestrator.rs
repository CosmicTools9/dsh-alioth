use async_trait::async_trait;
use std::collections::HashMap;

use crate::agents::ToolDefinition;
use crate::tools::registry::ToolRegistry;
use crate::tools::{ToolCall, ToolContext, ToolResult};

// ============================================
// Ports (Seams)
// ============================================

#[async_trait]
pub trait LlmGenerationPort: Send + Sync {
    async fn generate_with_tools(
        &self,
        prompt: &str,
        tools: &[llm::ToolDefinition],
    ) -> Result<llm::LlmResponse, String>;
}

#[async_trait]
pub trait ToolExecutionPort: Send + Sync {
    async fn execute(
        &self,
        call: &ToolCall,
        session_id: i64,
        user_id: Option<i64>,
        allowed_schemas: &[String],
    ) -> Result<ToolResult, String>;
    fn list_tools(&self) -> Vec<ToolDefinition>;
}

// ============================================
// Domain Types
// ============================================

pub struct ToolRunContext {
    pub initial_prompt: String,
    pub session_id: i64,
    pub user_id: Option<i64>,
    pub allowed_schemas: Vec<String>,
}

pub struct ToolRunResult {
    pub final_text: String,
    pub tool_calls: Vec<ExecutedToolCall>,
    pub steps_taken: u32,
    pub truncated: bool,
}

#[derive(Debug, Clone)]
pub struct ExecutedToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
    pub success: bool,
    pub output: String,
}

// ============================================
// ToolOrchestrator (Deep Module)
// ============================================

pub struct ToolOrchestrator<'a> {
    llm_port: Box<dyn LlmGenerationPort + 'a>,
    tool_port: Box<dyn ToolExecutionPort + 'a>,
    max_steps: u32,
    result_template: String,
    continuation_prompt: String,
}

impl<'a> ToolOrchestrator<'a> {
    pub fn new(
        llm_port: Box<dyn LlmGenerationPort + 'a>,
        tool_port: Box<dyn ToolExecutionPort + 'a>,
    ) -> Self {
        Self {
            llm_port,
            tool_port,
            max_steps: 5,
            result_template:
                r#"<tool_result name="{name}" success="{success}">{output}</tool_result>"#
                    .to_string(),
            continuation_prompt:
                "\n\n## 工具执行结果\n{results}\n\n请根据工具执行结果继续回答用户。".to_string(),
        }
    }

    pub fn with_max_steps(mut self, max_steps: u32) -> Self {
        self.max_steps = max_steps;
        self
    }

    pub async fn run(&self, ctx: &ToolRunContext) -> Result<ToolRunResult, String> {
        let tools = self.tool_port.list_tools();
        let llm_tools: Vec<llm::ToolDefinition> = tools
            .iter()
            .map(|t| llm::ToolDefinition {
                name: t.name.clone(),
                description: t.description.clone(),
                parameters: t.parameters.clone(),
            })
            .collect();

        let mut current_prompt = ctx.initial_prompt.clone();
        let mut step = 0u32;
        let mut executed_calls: Vec<ExecutedToolCall> = Vec::new();

        loop {
            if step >= self.max_steps {
                return Ok(ToolRunResult {
                    final_text: current_prompt,
                    tool_calls: executed_calls,
                    steps_taken: step,
                    truncated: true,
                });
            }
            step += 1;

            let response = self
                .llm_port
                .generate_with_tools(&current_prompt, &llm_tools)
                .await?;

            match response {
                llm::LlmResponse::Text(text) => {
                    return Ok(ToolRunResult {
                        final_text: text,
                        tool_calls: executed_calls,
                        steps_taken: step,
                        truncated: false,
                    });
                }
                llm::LlmResponse::ToolCalls(calls) => {
                    let mut results = Vec::new();
                    for call in calls {
                        let result = self
                            .tool_port
                            .execute(
                                &ToolCall {
                                    id: call.id.clone(),
                                    name: call.name.clone(),
                                    arguments: call.arguments.clone(),
                                },
                                ctx.session_id,
                                ctx.user_id,
                                &ctx.allowed_schemas,
                            )
                            .await;

                        match result {
                            Ok(tool_result) => {
                                executed_calls.push(ExecutedToolCall {
                                    id: call.id.clone(),
                                    name: call.name.clone(),
                                    arguments: call.arguments.clone(),
                                    success: tool_result.success,
                                    output: tool_result.output.to_string(),
                                });
                                results.push(
                                    self.result_template
                                        .replace("{name}", &call.name)
                                        .replace("{success}", &tool_result.success.to_string())
                                        .replace("{output}", &tool_result.output.to_string()),
                                );
                            }
                            Err(e) => {
                                executed_calls.push(ExecutedToolCall {
                                    id: call.id.clone(),
                                    name: call.name.clone(),
                                    arguments: call.arguments.clone(),
                                    success: false,
                                    output: e.clone(),
                                });
                                results.push(format!(
                                    r#"<tool_error name="{}">{}</tool_error>"#,
                                    call.name, e
                                ));
                            }
                        }
                    }
                    current_prompt.push_str(
                        &self
                            .continuation_prompt
                            .replace("{results}", &results.join("\n")),
                    );
                }
            }
        }
    }
}

// ============================================
// Production Adapters
// ============================================

pub struct LlmServiceAdapter<'a> {
    service: &'a llm::LlmService,
    /// 模型档位 override（chat 模型切换）；None = 主模型默认
    model_override: Option<String>,
}

impl<'a> LlmServiceAdapter<'a> {
    pub fn new(service: &'a llm::LlmService) -> Self {
        Self {
            service,
            model_override: None,
        }
    }

    /// 携带模型档位 override（如 flash 档模型名）。
    pub fn with_model_override(mut self, model_override: Option<String>) -> Self {
        self.model_override = model_override;
        self
    }
}

#[async_trait]
impl<'a> LlmGenerationPort for LlmServiceAdapter<'a> {
    async fn generate_with_tools(
        &self,
        prompt: &str,
        tools: &[llm::ToolDefinition],
    ) -> Result<llm::LlmResponse, String> {
        // 空 system 被 backend is_empty 守卫跳过：无 override 时与
        // generate_with_tools 请求形态等价（多返回 usage，弃用）。
        let (resp, _usage) = self
            .service
            .generate_detailed_with_tools(
                "",
                prompt,
                tools,
                None,
                None,
                None,
                None,
                self.model_override.as_deref(),
            )
            .await
            .map_err(|e| e.to_string())?;
        Ok(resp)
    }
}

pub struct DbToolAdapter {
    pool: sqlx::PgPool,
}

impl DbToolAdapter {
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ToolExecutionPort for DbToolAdapter {
    async fn execute(
        &self,
        call: &ToolCall,
        session_id: i64,
        user_id: Option<i64>,
        allowed_schemas: &[String],
    ) -> Result<ToolResult, String> {
        let registry = ToolRegistry::new();
        let ctx = ToolContext {
            session_id,
            user_id,
            db_pool: self.pool.clone(),
            allowed_schemas: allowed_schemas.to_vec(),
        };
        registry.execute(call, &ctx).await
    }

    fn list_tools(&self) -> Vec<ToolDefinition> {
        let registry = ToolRegistry::new();
        registry.list_tools()
    }
}

// ============================================
// Test Adapters
// ============================================

pub struct FakeLlmAdapter {
    responses: Vec<llm::LlmResponse>,
    pub call_log: std::sync::Mutex<Vec<(String, Vec<llm::ToolDefinition>)>>,
}

impl FakeLlmAdapter {
    pub fn new(responses: Vec<llm::LlmResponse>) -> Self {
        Self {
            responses,
            call_log: std::sync::Mutex::new(vec![]),
        }
    }
}

#[async_trait]
impl LlmGenerationPort for FakeLlmAdapter {
    async fn generate_with_tools(
        &self,
        prompt: &str,
        tools: &[llm::ToolDefinition],
    ) -> Result<llm::LlmResponse, String> {
        let idx = {
            let mut log = self.call_log.lock().unwrap();
            log.push((prompt.to_string(), tools.to_vec()));
            log.len() - 1
        };
        Ok(self
            .responses
            .get(idx)
            .cloned()
            .unwrap_or(llm::LlmResponse::Text("done".to_string())))
    }
}

pub struct FakeToolAdapter {
    definitions: Vec<ToolDefinition>,
    results: HashMap<String, ToolResult>,
    pub call_log: std::sync::Mutex<Vec<ToolCall>>,
}

impl FakeToolAdapter {
    pub fn new(definitions: Vec<ToolDefinition>, results: HashMap<String, ToolResult>) -> Self {
        Self {
            definitions,
            results,
            call_log: std::sync::Mutex::new(vec![]),
        }
    }
}

#[async_trait]
impl ToolExecutionPort for FakeToolAdapter {
    async fn execute(
        &self,
        call: &ToolCall,
        _session_id: i64,
        _user_id: Option<i64>,
        _allowed_schemas: &[String],
    ) -> Result<ToolResult, String> {
        self.call_log.lock().unwrap().push(call.clone());
        self.results
            .get(&call.name)
            .cloned()
            .ok_or_else(|| format!("Tool '{}' not found", call.name))
    }

    fn list_tools(&self) -> Vec<ToolDefinition> {
        self.definitions.clone()
    }
}
