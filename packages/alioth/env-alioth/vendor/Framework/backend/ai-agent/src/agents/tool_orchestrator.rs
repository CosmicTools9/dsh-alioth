use async_trait::async_trait;
use futures_util::StreamExt;
use std::collections::HashMap;

use crate::agents::{ConfirmationLevel, TokenUsage, ToolDefinition};
use crate::tools::registry::ToolRegistry;
use crate::tools::{ToolCall, ToolContext, ToolResult};

// ============================================
// Ports (Seams)
// ============================================

#[async_trait]
pub trait LlmGenerationPort: Send + Sync {
    /// 非流式生成 + 工具调用：返回响应与单步 token 用量（provider 缺失为 None）。
    async fn generate_with_tools(
        &self,
        prompt: &str,
        tools: &[llm::ToolDefinition],
    ) -> Result<(llm::LlmResponse, Option<TokenUsage>), String>;

    /// 流式生成 + 工具调用（可选能力；ToolOrchestrator 注册 [`ToolStreamEvent`]
    /// sink 后使用）：文本 delta 经返回的流逐 chunk 下发；流消费完毕（None）后
    /// 读 outcome 槽位得最终 [`llm::StreamToolCallOutcome`]（完整 tool_calls +
    /// usage）。槽位恒 None = 后端未实现工具流式（不支持）——调用方须报错而
    /// 非静默退化。默认实现 = 不支持。
    fn stream_with_tools<'a>(
        &'a self,
        _prompt: &'a str,
        _tools: &'a [llm::ToolDefinition],
    ) -> Result<
        (
            futures_util::stream::BoxStream<'a, Result<String, String>>,
            std::sync::Arc<std::sync::Mutex<Option<llm::StreamToolCallOutcome>>>,
        ),
        String,
    > {
        Err("streaming not supported".to_string())
    }
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

/// 业务动作处理器（fix-chat-ai-feature-gaps D2.1）——execute_action 工具的
/// 真实执行侧。由宿主（Gateway）注入 [`crate::tools::ToolContext::action_handler`]；
/// 无 handler 时 execute_action 维持预览行为（测试/无业务注入场景兼容）。
#[async_trait]
pub trait ActionHandler: Send + Sync {
    /// 执行动作；返回结构化结果（output JSON）。
    async fn execute(
        &self,
        action_type: &str,
        target_ids: &[i64],
        params: &serde_json::Value,
        ctx: &ToolContext,
    ) -> Result<serde_json::Value, String>;
    /// 该动作类型的确认级别：None/Preview/Low 直接执行；
    /// Explicit/High/Critical 必须 `args.confirmed == true` 才执行。
    fn confirmation_level(&self, action_type: &str) -> ConfirmationLevel;
}

/// execute_action 门禁判定：哪些确认级别必须收到 `confirmed=true` 才放行执行。
pub(crate) fn level_requires_confirmation(level: &ConfirmationLevel) -> bool {
    matches!(
        level,
        ConfirmationLevel::Explicit | ConfirmationLevel::High | ConfirmationLevel::Critical
    )
}

// ============================================
// Domain Types
// ============================================

/// 流式执行事件（ToolOrchestrator 注册 event_sink 后逐事件推送）：
/// LLM 文本 delta 与工具生命周期回调。
#[derive(Debug, Clone, PartialEq)]
pub enum ToolStreamEvent {
    /// LLM 文本增量（终答与工具前导均按原样转发）。
    Chunk(String),
    /// 工具开始执行（arguments 为 LLM 给出的 JSON）。
    ToolStart {
        name: String,
        arguments: serde_json::Value,
    },
    /// 工具执行完毕（success=false 时 output 为错误信息）。
    ToolEnd {
        name: String,
        success: bool,
        output: String,
    },
}

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
    /// 全流程各步 token 用量总和（provider 未报用量的步按 0 计）。
    pub usage: TokenUsage,
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
    /// 流式事件推送（None = 走非流式路径，行为与无 sink 时代完全一致）。
    event_sink: Option<Box<dyn Fn(ToolStreamEvent) + Send + Sync>>,
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
            event_sink: None,
        }
    }

    pub fn with_max_steps(mut self, max_steps: u32) -> Self {
        self.max_steps = max_steps;
        self
    }

    /// 注册流式事件 sink：注册后 run() 每步 LLM 调用走流式（文本 delta →
    /// [`ToolStreamEvent::Chunk`]，工具执行前后发 Start/End）。未注册时行为不变。
    pub fn with_event_sink(mut self, sink: Box<dyn Fn(ToolStreamEvent) + Send + Sync>) -> Self {
        self.event_sink = Some(sink);
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
        let mut total_usage = TokenUsage::default();

        loop {
            if step >= self.max_steps {
                return Ok(ToolRunResult {
                    final_text: current_prompt,
                    tool_calls: executed_calls,
                    steps_taken: step,
                    truncated: true,
                    usage: total_usage,
                });
            }
            step += 1;

            // 有 sink 走流式（delta → Chunk 事件）；无 sink 走原非流式路径。
            let (final_text, step_usage) = match &self.event_sink {
                Some(sink) => {
                    let (text, usage) = self
                        .stream_step(
                            sink,
                            &mut current_prompt,
                            &llm_tools,
                            &mut executed_calls,
                            ctx,
                        )
                        .await?;
                    (text, usage)
                }
                None => {
                    let (text, usage) = self
                        .plain_step(&mut current_prompt, &llm_tools, &mut executed_calls, ctx)
                        .await?;
                    (text, usage)
                }
            };
            accumulate_usage(&mut total_usage, step_usage);

            if let Some(text) = final_text {
                return Ok(ToolRunResult {
                    final_text: text,
                    tool_calls: executed_calls,
                    steps_taken: step,
                    truncated: false,
                    usage: total_usage,
                });
            }
        }
    }

    /// 非流式单步：LLM 一次生成。`Text` → 终答；`ToolCalls` → 执行工具并追加
    /// 续问 prompt（返回 None 表示继续循环）。返回 (终答, 本步 usage)。
    async fn plain_step(
        &self,
        current_prompt: &mut String,
        llm_tools: &[llm::ToolDefinition],
        executed_calls: &mut Vec<ExecutedToolCall>,
        ctx: &ToolRunContext,
    ) -> Result<(Option<String>, Option<TokenUsage>), String> {
        let (response, usage) = self
            .llm_port
            .generate_with_tools(current_prompt, llm_tools)
            .await?;
        match response {
            llm::LlmResponse::Text(text) => Ok((Some(text), usage)),
            llm::LlmResponse::ToolCalls(calls) => {
                let mut results = Vec::new();
                for call in calls {
                    let (executed, rendered) = self
                        .execute_one(&call.id, &call.name, &call.arguments, ctx)
                        .await;
                    executed_calls.push(executed);
                    results.push(rendered);
                }
                current_prompt.push_str(
                    &self
                        .continuation_prompt
                        .replace("{results}", &results.join("\n")),
                );
                Ok((None, usage))
            }
        }
    }

    /// 流式单步：文本 delta 逐 chunk 发 `Chunk` 事件；流尽读 outcome 槽位——
    /// 有 tool_calls → 逐个执行（前后发 ToolStart/ToolEnd）并追加续问 prompt
    /// （返回 None）；无 tool_calls → 累积文本即终答。
    async fn stream_step(
        &self,
        sink: &(dyn Fn(ToolStreamEvent) + Send + Sync),
        current_prompt: &mut String,
        llm_tools: &[llm::ToolDefinition],
        executed_calls: &mut Vec<ExecutedToolCall>,
        ctx: &ToolRunContext,
    ) -> Result<(Option<String>, Option<TokenUsage>), String> {
        let (mut stream, outcome_slot) =
            self.llm_port.stream_with_tools(current_prompt, llm_tools)?;
        let mut text = String::new();
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(delta) => {
                    text.push_str(&delta);
                    sink(ToolStreamEvent::Chunk(delta));
                }
                Err(e) => return Err(e),
            }
        }
        // 流已耗尽：显式 drop 释放其对 prompt/self 的借用，才能追加续问 prompt。
        drop(stream);
        let outcome = outcome_slot
            .lock()
            .map_err(|_| "stream outcome slot poisoned".to_string())?
            .take()
            .ok_or_else(|| {
                "llm backend did not provide a streamed tool outcome (unsupported)".to_string()
            })?;
        let usage = outcome.usage.map(map_llm_usage);

        if outcome.tool_calls.is_empty() {
            return Ok((Some(text), usage));
        }
        let mut results = Vec::new();
        for call in &outcome.tool_calls {
            sink(ToolStreamEvent::ToolStart {
                name: call.name.clone(),
                arguments: call.arguments.clone(),
            });
            let (executed, rendered) = self
                .execute_one(&call.id, &call.name, &call.arguments, ctx)
                .await;
            sink(ToolStreamEvent::ToolEnd {
                name: call.name.clone(),
                success: executed.success,
                output: executed.output.clone(),
            });
            executed_calls.push(executed);
            results.push(rendered);
        }
        current_prompt.push_str(
            &self
                .continuation_prompt
                .replace("{results}", &results.join("\n")),
        );
        Ok((None, usage))
    }

    /// 执行单个工具调用：返回 (执行记录, 结果模板串)。执行失败（端口 Err）记
    /// success=false + 错误信息，不进终止路径（与既有行为一致：继续给 LLM）。
    async fn execute_one(
        &self,
        id: &str,
        name: &str,
        arguments: &serde_json::Value,
        ctx: &ToolRunContext,
    ) -> (ExecutedToolCall, String) {
        let result = self
            .tool_port
            .execute(
                &ToolCall {
                    id: id.to_string(),
                    name: name.to_string(),
                    arguments: arguments.clone(),
                },
                ctx.session_id,
                ctx.user_id,
                &ctx.allowed_schemas,
            )
            .await;

        match result {
            Ok(tool_result) => {
                let executed = ExecutedToolCall {
                    id: id.to_string(),
                    name: name.to_string(),
                    arguments: arguments.clone(),
                    success: tool_result.success,
                    output: tool_result.output.to_string(),
                };
                let rendered = self
                    .result_template
                    .replace("{name}", name)
                    .replace("{success}", &tool_result.success.to_string())
                    .replace("{output}", &tool_result.output.to_string());
                (executed, rendered)
            }
            Err(e) => {
                let executed = ExecutedToolCall {
                    id: id.to_string(),
                    name: name.to_string(),
                    arguments: arguments.clone(),
                    success: false,
                    output: e.clone(),
                };
                let rendered = format!(r#"<tool_error name="{}">{}</tool_error>"#, name, e);
                (executed, rendered)
            }
        }
    }
}

/// llm crate usage（input/output tokens）→ agents usage（prompt/completion/total）。
fn map_llm_usage(u: llm::TokenUsage) -> TokenUsage {
    TokenUsage {
        prompt_tokens: u.input_tokens,
        completion_tokens: u.output_tokens,
        total_tokens: u.input_tokens + u.output_tokens,
    }
}

/// 把单步 usage 累加进总量；`None`（provider 未报）按 0 计。
fn accumulate_usage(total: &mut TokenUsage, step: Option<TokenUsage>) {
    if let Some(u) = step {
        total.prompt_tokens += u.prompt_tokens;
        total.completion_tokens += u.completion_tokens;
        total.total_tokens += u.total_tokens;
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
    ) -> Result<(llm::LlmResponse, Option<TokenUsage>), String> {
        // 空 system 被 backend is_empty 守卫跳过：无 override 时与
        // generate_with_tools 请求形态等价（顺带取回 usage）。
        let (resp, usage) = self
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
        Ok((resp, usage.map(map_llm_usage)))
    }

    fn stream_with_tools<'s>(
        &'s self,
        prompt: &'s str,
        tools: &'s [llm::ToolDefinition],
    ) -> Result<
        (
            futures_util::stream::BoxStream<'s, Result<String, String>>,
            std::sync::Arc<std::sync::Mutex<Option<llm::StreamToolCallOutcome>>>,
        ),
        String,
    > {
        // 空 system 语义与非流式路径一致（backend 跳过空 system）。
        let (stream, outcome_slot) = self.service.generate_stream_with_tools(
            None,
            prompt,
            tools,
            None,
            None,
            None,
            None,
            self.model_override.as_deref(),
        );
        let stream = stream.map(|item| item.map_err(|e| e.to_string())).boxed();
        Ok((stream, outcome_slot))
    }
}

pub struct DbToolAdapter {
    pool: sqlx::PgPool,
    /// 允许该 agent 调用的工具白名单；空集 = 不过滤（registry 全量）。
    allowed_tools: std::collections::HashSet<String>,
}

impl DbToolAdapter {
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self {
            pool,
            allowed_tools: std::collections::HashSet::new(),
        }
    }

    /// 工具过滤（fix-chat-ai-feature-gaps D2.3）：`names` 与 registry 取交集后
    /// 作为可调用白名单；空 `names` = 不过滤（保持全量）。
    pub fn with_allowed_tools(mut self, names: Vec<String>) -> Self {
        self.allowed_tools = names.into_iter().collect();
        self
    }

    fn tool_allowed(&self, name: &str) -> bool {
        self.allowed_tools.is_empty() || self.allowed_tools.contains(name)
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
        if !self.tool_allowed(&call.name) {
            return Ok(ToolResult {
                tool_call_id: call.id.clone(),
                name: call.name.clone(),
                success: false,
                output: serde_json::Value::Null,
                error: Some("tool not allowed for agent".to_string()),
            });
        }
        let registry = ToolRegistry::new();
        let ctx = ToolContext {
            session_id,
            user_id,
            db_pool: self.pool.clone(),
            allowed_schemas: allowed_schemas.to_vec(),
            // DbToolAdapter 仅承载 DB 工具；业务动作 handler 由宿主
            // （Gateway ActionHandler）经自定义执行端口注入。
            action_handler: None,
        };
        registry.execute(call, &ctx).await
    }

    fn list_tools(&self) -> Vec<ToolDefinition> {
        let registry = ToolRegistry::new();
        registry
            .list_tools()
            .into_iter()
            .filter(|t| self.tool_allowed(&t.name))
            .collect()
    }
}

// ============================================
// Test Adapters
// ============================================

/// 流式场景预设（FakeLlmAdapter）：一步 LLM 生成的文本分片 + 最终 outcome。
#[derive(Debug, Clone)]
pub struct FakeStreamTurn {
    /// 文本 delta 序列（逐项 = 一个 Chunk 事件）。
    pub chunks: Vec<Result<String, String>>,
    /// 流尽后的 outcome（tool_calls + usage）；None 可模拟不支持/异常。
    pub outcome: Option<llm::StreamToolCallOutcome>,
}

pub struct FakeLlmAdapter {
    responses: Vec<llm::LlmResponse>,
    /// 非流式 usage 预设：与 responses 按调用序对齐；缺口 = None（按 0 计）。
    usages: Vec<Option<TokenUsage>>,
    /// 流式场景预设：按调用序消费；空 = 流式不支持。
    stream_turns: Vec<FakeStreamTurn>,
    pub call_log: std::sync::Mutex<Vec<(String, Vec<llm::ToolDefinition>)>>,
}

impl FakeLlmAdapter {
    pub fn new(responses: Vec<llm::LlmResponse>) -> Self {
        Self {
            responses,
            usages: vec![],
            stream_turns: vec![],
            call_log: std::sync::Mutex::new(vec![]),
        }
    }

    /// usage 预设（与 responses 按调用序对齐；缺失槽位 = None）。
    pub fn with_usages(mut self, usages: Vec<Option<TokenUsage>>) -> Self {
        self.usages = usages;
        self
    }

    /// 流式场景预设：注册后 `stream_with_tools` 按调用序回放分片与 outcome。
    pub fn with_stream_turns(mut self, stream_turns: Vec<FakeStreamTurn>) -> Self {
        self.stream_turns = stream_turns;
        self
    }
}

#[async_trait]
impl LlmGenerationPort for FakeLlmAdapter {
    async fn generate_with_tools(
        &self,
        prompt: &str,
        tools: &[llm::ToolDefinition],
    ) -> Result<(llm::LlmResponse, Option<TokenUsage>), String> {
        let idx = {
            let mut log = self.call_log.lock().unwrap();
            log.push((prompt.to_string(), tools.to_vec()));
            log.len() - 1
        };
        let usage = self.usages.get(idx).cloned().flatten();
        Ok((
            self.responses
                .get(idx)
                .cloned()
                .unwrap_or(llm::LlmResponse::Text("done".to_string())),
            usage,
        ))
    }

    fn stream_with_tools<'s>(
        &'s self,
        prompt: &'s str,
        tools: &'s [llm::ToolDefinition],
    ) -> Result<
        (
            futures_util::stream::BoxStream<'s, Result<String, String>>,
            std::sync::Arc<std::sync::Mutex<Option<llm::StreamToolCallOutcome>>>,
        ),
        String,
    > {
        let idx = {
            let mut log = self.call_log.lock().unwrap();
            log.push((prompt.to_string(), tools.to_vec()));
            log.len() - 1
        };
        let turn = self.stream_turns.get(idx).cloned().ok_or_else(|| {
            format!(
                "streaming not supported (no scenario preset for step {})",
                idx + 1
            )
        })?;
        let stream = futures_util::stream::iter(turn.chunks).boxed();
        Ok((
            stream,
            std::sync::Arc::new(std::sync::Mutex::new(turn.outcome)),
        ))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn lazy_pool() -> sqlx::PgPool {
        sqlx::PgPool::connect_lazy("postgres://u:p@localhost:5432/nodb").unwrap()
    }

    fn tool_names(defs: Vec<ToolDefinition>) -> Vec<String> {
        let mut names: Vec<String> = defs.into_iter().map(|d| d.name).collect();
        names.sort();
        names
    }

    #[tokio::test]
    async fn db_adapter_allowed_tools_intersects_registry() {
        let adapter = DbToolAdapter::new(lazy_pool())
            .with_allowed_tools(vec!["query_sql".into(), "no_such_tool".into()]);
        // 白名单 ∩ registry：no_such_tool 不存在于 registry，被自然剔除
        assert_eq!(tool_names(adapter.list_tools()), vec!["query_sql"]);
    }

    #[tokio::test]
    async fn db_adapter_empty_allowed_means_no_filter() {
        let full = DbToolAdapter::new(lazy_pool());
        let filtered = DbToolAdapter::new(lazy_pool()).with_allowed_tools(vec![]);
        assert_eq!(filtered.list_tools().len(), full.list_tools().len());
        assert!(filtered.list_tools().len() >= 5, "registry 全量工具");
    }

    #[tokio::test]
    async fn db_adapter_execute_rejects_non_allowed_tool() {
        let adapter = DbToolAdapter::new(lazy_pool()).with_allowed_tools(vec!["query_sql".into()]);
        let call = ToolCall {
            id: "c1".into(),
            name: "validate_form".into(),
            arguments: serde_json::json!({}),
        };
        let result = adapter.execute(&call, 1, None, &[]).await.unwrap();
        assert!(!result.success);
        assert_eq!(result.error.as_deref(), Some("tool not allowed for agent"));
    }

    #[tokio::test]
    async fn db_adapter_unfiltered_execute_passes_through() {
        // 未过滤（空白名单）→ 白名单门禁不拦截，走向 registry（未知工具由 registry 报错）
        let adapter = DbToolAdapter::new(lazy_pool());
        let call = ToolCall {
            id: "c1".into(),
            name: "definitely_not_a_tool".into(),
            arguments: serde_json::json!({}),
        };
        let result = adapter.execute(&call, 1, None, &[]).await.unwrap();
        assert!(!result.success);
        assert_eq!(
            result.error.as_deref(),
            Some("Unknown tool: definitely_not_a_tool")
        );
    }
}
