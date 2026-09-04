use async_trait::async_trait;
use chrono::Utc;
use serde_json::Value;
use sqlx::PgPool;
use std::sync::Arc;

use ai_agent::agents::{extract_json_block, tool_orchestrator};

use crate::api::chat_sessions::ports::{
    AIContactPort, AgentDispatchPort, LlmConfigPort, MessageStorePort, SessionStorePort,
};
use crate::api::chat_sessions::{
    AgentInfoResponse, ChatMessageResponse, ChatSessionResponse, ExecuteActionResponse,
    ModelOptionResponse,
};

/// 上下文段组装（design D1/D8）：system_prompt + 当前页面上下文（4000 字符截断、
/// knowledgeContext 排除出页面段）+ 知识参考（2000）+ NGAC 权限 + 用户记忆
/// （2000，空记忆不注入）+ agent_state 子段（schema_catalog/form_schema/business_rules）。
/// 纯函数——process_turn 与单测共用（page-context 自动获取机制验证）。
fn assemble_context_sections(
    system_prompt: &str,
    page_context: Option<&Value>,
    permissions: Option<&Value>,
    user_memory: Option<&Value>,
    agent_state: Option<&Value>,
) -> String {
    let mut prompt = system_prompt.to_string();

    // 知识注入结果（前端 AIChatContext.knowledgeContext）：独立「知识参考」段，
    // 不入 conversation history、不占 PAGE_CONTEXT_CAP 预算。
    let knowledge_ctx = page_context
        .and_then(|pc| pc.get("knowledgeContext"))
        .and_then(|v| v.as_str())
        .map(String::from);
    prompt.push_str("\n\n## 当前页面上下文\n");
    if let Some(pc) = page_context {
        // Server-side assembly cap (design D8): serialized page_context is
        // truncated at 4000 chars to prevent prompt stuffing.
        // knowledgeContext 已单独提取，渲染时排除（避免嵌套 JSON 语义混杂）。
        let mut pc_clean = pc.clone();
        if let serde_json::Value::Object(map) = &mut pc_clean {
            map.remove("knowledgeContext");
        }
        let serialized = serde_json::to_string(&pc_clean).unwrap_or_default();
        const PAGE_CONTEXT_CAP: usize = 4000;
        if serialized.len() > PAGE_CONTEXT_CAP {
            let mut truncated: String = serialized.chars().take(PAGE_CONTEXT_CAP).collect();
            truncated.push_str("…[truncated]");
            prompt.push_str(&truncated);
        } else {
            prompt.push_str(&serialized);
        }
    } else {
        prompt.push_str("（无特定页面上下文）");
    }
    if let Some(k) = knowledge_ctx {
        prompt.push_str("\n\n## 知识参考\n");
        const KNOWLEDGE_CAP: usize = 2000;
        if k.chars().count() > KNOWLEDGE_CAP {
            let mut truncated: String = k.chars().take(KNOWLEDGE_CAP).collect();
            truncated.push_str("…[truncated]");
            prompt.push_str(&truncated);
        } else {
            prompt.push_str(&k);
        }
    }

    // 注入用户权限范围（NGAC），约束 LLM 数据 API 调用范围
    if let Some(perm) = permissions {
        prompt.push_str("\n\n## 用户权限范围\n");
        prompt.push_str(
            "以下是你当前可访问的数据权限范围，所有数据 API 调用必须严格限定在此范围内：\n",
        );
        prompt.push_str(&serde_json::to_string_pretty(perm).unwrap_or_default());
    }

    // 注入用户级长期记忆（add-agent-pool-user-memory）：
    // 仅当前 user_id 的跨 session 记忆，2000 字符截断；空记忆不注入段。
    if let Some(mem) = user_memory {
        let memory_str = serde_json::to_string(mem).unwrap_or_default();
        if !memory_str.is_empty() && memory_str != "{}" {
            prompt.push_str("\n\n## 用户记忆\n");
            prompt.push_str("以下是你对该用户的长期记忆（跨会话偏好/事实），请据此个性化回答：\n");
            const MEMORY_CAP: usize = 2000;
            if memory_str.chars().count() > MEMORY_CAP {
                let mut truncated: String = memory_str.chars().take(MEMORY_CAP).collect();
                truncated.push_str("…[truncated]");
                prompt.push_str(&truncated);
            } else {
                prompt.push_str(&memory_str);
            }
        }
    }

    if let Some(catalog) = agent_state.and_then(|s| s.get("schema_catalog")) {
        prompt.push_str("\n\n## 可用数据表\n");
        prompt.push_str(&serde_json::to_string(catalog).unwrap_or_default());
    }
    if let Some(schema) = agent_state.and_then(|s| s.get("form_schema")) {
        prompt.push_str("\n\n## 目标表单结构\n");
        prompt.push_str(&serde_json::to_string(schema).unwrap_or_default());
    }
    if let Some(rules) = agent_state.and_then(|s| s.get("business_rules")) {
        prompt.push_str("\n\n## 业务规则\n");
        prompt.push_str(&serde_json::to_string(rules).unwrap_or_default());
    }

    prompt
}

/// 模型档位 → LLM model_override 解析（纯函数，单测覆盖）。
/// "flash" → flash 档模型名；"deep"/None → None（主模型默认）；未知档位 → Err。
fn resolve_model_override(tier: Option<&str>, flash_model: &str) -> Result<Option<String>, String> {
    match tier {
        None | Some("deep") => Ok(None),
        Some("flash") => Ok(Some(flash_model.to_string())),
        Some(other) => Err(format!(
            "Unknown model tier '{}', expected \"deep\" or \"flash\"",
            other
        )),
    }
}

pub struct TurnInput {
    pub session_id: i64,
    pub user_id: i64,
    pub locale: String,
    /// 模型档位（chat 模型切换）："deep" | "flash"；None = 主模型默认
    pub model: Option<String>,
}

pub struct TurnResult {
    pub message: ChatMessageResponse,
}

pub struct CreateSessionInput {
    pub title: Option<String>,
    pub context: Option<Value>,
    pub user_id: i64,
    pub locale: String,
}

#[async_trait]
pub trait SessionOrchestrator: Send + Sync {
    /// 处理一轮对话。`on_chunk` 非 None 时，无工具路径以 LLM 真流式
    /// （SSE 逐 chunk）调用回调（P1-6 完成态）；None 时整段生成（兼容调用方）。
    async fn process_turn(
        &self,
        input: TurnInput,
        on_chunk: Option<Box<dyn Fn(String) + Send + Sync>>,
    ) -> Result<TurnResult, String>;
    async fn create_session(
        &self,
        input: CreateSessionInput,
    ) -> Result<ChatSessionResponse, String>;
    async fn add_message(
        &self,
        session_id: i64,
        content: &str,
        context: Option<Value>,
        user_id: i64,
    ) -> Result<ChatMessageResponse, String>;
    async fn switch_agent(
        &self,
        session_id: i64,
        agent_code: &str,
        user_id: i64,
    ) -> Result<(), String>;
    #[allow(dead_code)]
    async fn execute_action(
        &self,
        session_id: i64,
        action_id: &str,
        params: Option<Value>,
        confirmed: bool,
        user_id: i64,
    ) -> Result<ExecuteActionResponse, String>;
    async fn list_agents(&self) -> Result<Vec<AgentInfoResponse>, String>;
    /// 模型档位元数据（chat 模型切换）：deep/flash 两档与实际模型名。
    async fn list_model_options(&self) -> Result<Vec<ModelOptionResponse>, String>;
}

pub struct DefaultSessionOrchestrator {
    pool: PgPool,
    i18n: crate::i18n::I18nManagerRef,
    session_store: Arc<dyn SessionStorePort>,
    message_store: Arc<dyn MessageStorePort>,
    llm_config: Arc<dyn LlmConfigPort>,
    agent_dispatch: Arc<dyn AgentDispatchPort>,
    ai_contact: Arc<dyn AIContactPort>,
}

impl DefaultSessionOrchestrator {
    pub fn new(
        pool: PgPool,
        i18n: crate::i18n::I18nManagerRef,
        session_store: Arc<dyn SessionStorePort>,
        message_store: Arc<dyn MessageStorePort>,
        llm_config: Arc<dyn LlmConfigPort>,
        agent_dispatch: Arc<dyn AgentDispatchPort>,
        ai_contact: Arc<dyn AIContactPort>,
    ) -> Self {
        Self {
            pool,
            i18n,
            session_store,
            message_store,
            llm_config,
            agent_dispatch,
            ai_contact,
        }
    }

    fn derive_role(&self, sender_addr: Option<i64>, ai_contact_id: Option<i64>) -> String {
        match (sender_addr, ai_contact_id) {
            (Some(addr), Some(ai_id)) if addr == ai_id => "assistant".to_string(),
            _ => "user".to_string(),
        }
    }
}

#[async_trait]
impl SessionOrchestrator for DefaultSessionOrchestrator {
    async fn process_turn(
        &self,
        input: TurnInput,
        on_chunk: Option<Box<dyn Fn(String) + Send + Sync>>,
    ) -> Result<TurnResult, String> {
        let locale = i18n::Locale::new(&input.locale);
        let locale_str = input.locale;

        // 1. 会话验证
        let session = self
            .session_store
            .get_session(input.session_id, input.user_id)
            .await?;
        if session.is_none() {
            return Err("SESSION_NOT_FOUND".to_string());
        }

        // 加载会话权限
        let permissions = session.as_ref().and_then(|s| s.permissions.clone());

        // 2. 加载 page_context + agent_state
        let (page_context, agent_state) = self
            .session_store
            .get_session_context(input.session_id, input.user_id)
            .await?;

        // 3. 获取最后一条用户消息
        let ai_contact_id = self.ai_contact.resolve_ai_contact_id(&locale).await?;
        let user_content = self
            .message_store
            .get_last_user_message(input.session_id, ai_contact_id)
            .await?
            .ok_or_else(|| "NO_USER_MESSAGE".to_string())?;

        // 4. 加载历史（最多 20 条）
        let history_rows = self
            .message_store
            .get_history(input.session_id, input.user_id, 20)
            .await?;
        let history = history_rows
            .iter()
            .map(|msg| {
                let role = self.derive_role(msg.fk_sender_addr, ai_contact_id);
                (role, msg.content.clone().unwrap_or_default())
            })
            .collect::<Vec<_>>();

        // 5. 加载 LLM 服务
        let llm = self.llm_config.load_service().await?;

        // 5b. 模型档位解析（chat 模型切换）：flash → flash_model，deep/缺省 → 主模型
        let model_override =
            resolve_model_override(input.model.as_deref(), &llm.config().flash_model)?;

        // 6. 路由/解析 Agent
        let agent_code = self
            .agent_dispatch
            .resolve_agent(
                input.session_id,
                &user_content,
                page_context.clone(),
                &history,
                &locale_str,
                &llm,
            )
            .await
            .unwrap_or_else(|e| {
                common::telemetry::warn!("Agent routing failed: {}, falling back to general", e);
                "general".to_string()
            });

        // 7. 获取 Agent 配置
        let agent_config = self.agent_dispatch.get_agent_config(&agent_code).await?;

        // 8. 统一构建 prompt（上下文段组装为纯函数 assemble_context_sections，
        //    页面上下文/知识/权限/记忆/agent_state 段可单测——page-context 机制验证）
        let user_memory = self
            .agent_dispatch
            .load_user_memory(input.user_id)
            .await
            .ok();
        let mut prompt = assemble_context_sections(
            &agent_config.system_prompt,
            page_context.as_ref(),
            permissions.as_ref(),
            user_memory.as_ref(),
            agent_state.as_ref(),
        );

        prompt.push_str("\n\n## 对话历史\n");
        for (role, content) in &history {
            prompt.push_str(&format!("{}: {}\n", role, content));
        }

        prompt.push_str(&format!("\n\n## 用户最新消息\n{}\n", user_content));

        // 9. 统一执行：ToolOrchestrator（有工具）或直调 LLM（无工具）
        let content = if !agent_config.available_tools.is_empty() {
            let llm_port = Box::new(
                tool_orchestrator::LlmServiceAdapter::new(&llm)
                    .with_model_override(model_override.clone()),
            );
            let tool_port = Box::new(tool_orchestrator::DbToolAdapter::new(self.pool.clone()));
            let orchestrator = tool_orchestrator::ToolOrchestrator::new(llm_port, tool_port)
                .with_max_steps(agent_config.max_execution_steps);
            let tool_ctx = tool_orchestrator::ToolRunContext {
                initial_prompt: prompt,
                session_id: input.session_id,
                user_id: Some(input.user_id),
                allowed_schemas: agent_config.allowed_schemas.clone(),
            };
            let result = orchestrator.run(&tool_ctx).await?;
            result.final_text
        } else {
            // 无工具：LLM 真流式（P1-6）——on_chunk 存在时逐 chunk 回调并累积，
            // 否则整段生成（兼容）。
            match on_chunk {
                Some(cb) => {
                    let mut full = String::new();
                    let mut stream = llm.generate_stream_detailed(
                        None,
                        &prompt,
                        None,
                        None,
                        None,
                        None,
                        model_override.as_deref(),
                    );
                    use futures::StreamExt;
                    while let Some(chunk) = stream.next().await {
                        match chunk {
                            Ok(text) => {
                                full.push_str(&text);
                                cb(text);
                            }
                            Err(e) => return Err(e.to_string()),
                        }
                    }
                    full
                }
                None => {
                    llm.generate_detailed(
                        "",
                        &prompt,
                        None,
                        None,
                        None,
                        None,
                        model_override.as_deref(),
                    )
                    .await
                    .map_err(|e| e.to_string())?
                    .0
                }
            }
        };

        // 空回复守卫（fix-chat-ai-empty-reply）：thinking 模型思考耗尽预算或
        // 模型未输出时 content 为空，静默落库会沿 WS/HTTP 传播成前端空气泡。
        // 显式失败——错误经 error 帧 / failed 链透传到用户界面。
        if content.trim().is_empty() {
            return Err(
                "LLM_RETURNED_EMPTY: 模型未生成任何正文（常见原因：思考超出 max_tokens 预算被截断），请重试或换一个更具体的问题"
                    .to_string(),
            );
        }

        // 10. 统一后处理
        let structured = extract_json_block(&content);
        let requires_input = agent_config.requires_input_default
            || structured
                .as_ref()
                .map(|v| {
                    v.get("pending_confirmations")
                        .and_then(|p| p.as_array())
                        .map(|arr| !arr.is_empty())
                        .unwrap_or(false)
                })
                .unwrap_or(false)
            || structured
                .as_ref()
                .map(|v| {
                    v.get("requires_confirmation")
                        .and_then(|r| r.as_bool())
                        .unwrap_or(false)
                })
                .unwrap_or(false);

        let agent_result = ai_agent::agents::AgentResult::new(
            &agent_config.code,
            content,
            structured,
            requires_input,
            agent_config.suggested_actions.clone(),
        );

        // 11. 保存 agent_state
        let _ = self
            .session_store
            .update_session_state(
                input.session_id,
                input.user_id,
                serde_json::json!({
                    "last_result": {
                        "structured": agent_result.structured,
                        "requires_input": agent_result.requires_input,
                        "suggested_actions": agent_result.suggested_actions,
                    }
                }),
            )
            .await;

        // 12. 保存 assistant 消息
        let row = self
            .message_store
            .add_message(input.session_id, &agent_result.content, ai_contact_id)
            .await?;

        // 13. 更新会话时间戳
        let _ = self
            .session_store
            .update_session_timestamp(input.session_id, input.user_id)
            .await;
        let resp = ChatMessageResponse {
            id: row.id,
            role: self.derive_role(row.fk_sender_addr, ai_contact_id),
            content: row.content.unwrap_or_default(),
            created_at: row.created_at,
            agent_code: agent_code.clone(),
            structured: agent_result.structured,
            requires_input: agent_result.requires_input,
            suggested_actions: agent_result.suggested_actions,
        };

        Ok(TurnResult { message: resp })
    }

    async fn create_session(
        &self,
        input: CreateSessionInput,
    ) -> Result<ChatSessionResponse, String> {
        let locale = i18n::Locale::new(&input.locale);
        let i18n = self.i18n.read().await;
        let default_title = i18n
            .get(&locale, "chat.session.defaultTitle")
            .unwrap_or("New Chat")
            .to_string();
        drop(i18n);

        let title = input.title.unwrap_or(default_title.clone());

        let session = self
            .session_store
            .create_session(&title, input.context, input.user_id)
            .await?;

        Ok(ChatSessionResponse {
            id: session.id,
            title: session.title,
            status: "active".to_string(),
            agent_code: None,
            created_at: session.created_at,
            updated_at: session.updated_at,
        })
    }

    async fn add_message(
        &self,
        session_id: i64,
        content: &str,
        context: Option<Value>,
        user_id: i64,
    ) -> Result<ChatMessageResponse, String> {
        let session = self.session_store.get_session(session_id, user_id).await?;
        if session.is_none() {
            return Err("SESSION_NOT_FOUND".to_string());
        }

        // Message-level context: refresh the page_context snapshot in the same
        // turn as the message write (full-replacement semantics, design D2).
        // Precondition: create_session must precede add_message (frontend
        // ensureSession path); a missing session errors before any context write.
        if let Some(ctx) = context {
            self.session_store
                .update_session_context(session_id, user_id, ctx)
                .await?;
        }

        let sender_addr = self.ai_contact.resolve_user_contact_id(user_id).await?;

        let row = self
            .message_store
            .add_message(session_id, content, sender_addr)
            .await?;

        let _ = self
            .session_store
            .update_session_timestamp(session_id, user_id)
            .await;

        Ok(ChatMessageResponse {
            id: row.id,
            role: self.derive_role(row.fk_sender_addr, None),
            content: row.content.unwrap_or_default(),
            created_at: row.created_at,
            // leader-agent 绑定持久化已随 zc_id_threads_rr_entity 删除，普通消息无绑定 agent
            agent_code: String::new(),
            structured: None,
            requires_input: false,
            suggested_actions: vec![],
        })
    }

    async fn switch_agent(
        &self,
        session_id: i64,
        agent_code: &str,
        user_id: i64,
    ) -> Result<(), String> {
        if !self.agent_dispatch.agent_exists(agent_code).await {
            return Err(format!("Agent '{}' not found", agent_code));
        }

        let session = self.session_store.get_session(session_id, user_id).await?;
        if session.is_none() {
            return Err("SESSION_NOT_FOUND".to_string());
        }

        let _ = self
            .session_store
            .update_session_state(
                session_id,
                user_id,
                serde_json::json!({"switched_at": Utc::now().to_rfc3339()}),
            )
            .await;

        Ok(())
    }

    async fn execute_action(
        &self,
        session_id: i64,
        _action_id: &str,
        _params: Option<Value>,
        _confirmed: bool,
        _user_id: i64,
    ) -> Result<ExecuteActionResponse, String> {
        let session = self.session_store.get_session(session_id, _user_id).await?;
        if session.is_none() {
            return Err("SESSION_NOT_FOUND".to_string());
        }

        Err("ACTION_NOT_IMPLEMENTED: Direct action execution is not yet implemented. Use the message + generate-response flow instead.".to_string())
    }

    async fn list_agents(&self) -> Result<Vec<AgentInfoResponse>, String> {
        let configs = self.agent_dispatch.list_agent_configs().await?;
        let agents: Vec<AgentInfoResponse> = configs
            .iter()
            .map(|c| AgentInfoResponse {
                code: c.code.clone(),
                name: c.name.clone(),
                description: c.description.clone(),
                capabilities: c
                    .capabilities
                    .iter()
                    .map(|cap| format!("{:?}", cap))
                    .collect(),
                user_selectable: c.user_selectable,
                sort_order: c.sort_order,
                icon: c.icon.clone(),
                color: c.color.clone(),
                category: c.category.clone(),
            })
            .collect();
        Ok(agents)
    }
    async fn list_model_options(&self) -> Result<Vec<ModelOptionResponse>, String> {
        let svc = self.llm_config.load_service().await?;
        let cfg = svc.config();
        Ok(vec![
            ModelOptionResponse {
                id: "deep".to_string(),
                model: cfg.model.clone(),
            },
            ModelOptionResponse {
                id: "flash".to_string(),
                model: cfg.flash_model.clone(),
            },
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn pc(v: Value) -> Option<Value> {
        Some(v)
    }

    // ── page-context 自动获取机制验证（design D1/D8）────────────────

    #[test]
    fn test_page_context_injected() {
        let prompt = assemble_context_sections(
            "system",
            pc(json!({
                "page": "流程设计器",
                "module": "wz",
                "currentData": {"flow": "FLOW-FREIGHT", "nodes": 4}
            }))
            .as_ref(),
            None,
            None,
            None,
        );
        assert!(prompt.contains("## 当前页面上下文"));
        assert!(prompt.contains("流程设计器"));
        assert!(prompt.contains("FLOW-FREIGHT"));
        assert!(prompt.contains("system"));
    }

    #[test]
    fn test_page_context_truncated_at_cap() {
        let big = json!({"page": "x".repeat(5000)});
        let prompt = assemble_context_sections("s", pc(big).as_ref(), None, None, None);
        assert!(prompt.contains("…[truncated]"));
        // 4000 截断 + 标记
        let section = prompt.split("## 当前页面上下文").nth(1).unwrap_or_default();
        assert!(section.len() < 4100, "page context section must be capped");
    }

    #[test]
    fn test_knowledge_context_excluded_and_separate() {
        let prompt = assemble_context_sections(
            "s",
            pc(json!({
                "page": "托单跟踪",
                "knowledgeContext": "LAB-44 赔偿标准：破损按运费的 3 倍赔付"
            }))
            .as_ref(),
            None,
            None,
            None,
        );
        // 页面上下文段不得嵌套 knowledgeContext 原文
        let page_section = prompt
            .split("## 当前页面上下文")
            .nth(1)
            .and_then(|s| s.split("## 知识参考").next())
            .unwrap_or_default();
        assert!(
            !page_section.contains("LAB-44"),
            "knowledge must not nest in page section"
        );
        // 独立知识参考段
        assert!(prompt.contains("## 知识参考"));
        assert!(prompt.contains("LAB-44"));
    }

    #[test]
    fn test_no_page_context_fallback() {
        let prompt = assemble_context_sections("s", None, None, None, None);
        assert!(prompt.contains("（无特定页面上下文）"));
    }

    #[test]
    fn test_permissions_section() {
        let prompt = assemble_context_sections(
            "s",
            None,
            pc(json!({"policies": ["approval.read"]})).as_ref(),
            None,
            None,
        );
        assert!(prompt.contains("## 用户权限范围"));
        assert!(prompt.contains("approval.read"));
    }

    #[test]
    fn test_user_memory_empty_not_injected() {
        let prompt = assemble_context_sections("s", None, None, pc(json!({})).as_ref(), None);
        assert!(!prompt.contains("## 用户记忆"));
    }

    #[test]
    fn test_agent_state_sections() {
        let prompt = assemble_context_sections(
            "s",
            None,
            None,
            None,
            pc(json!({
                "schema_catalog": {"t": 1},
                "form_schema": {"f": 2},
                "business_rules": ["r1"]
            }))
            .as_ref(),
        );
        assert!(prompt.contains("## 可用数据表"));
        assert!(prompt.contains("## 目标表单结构"));
        assert!(prompt.contains("## 业务规则"));
    }
    // ── 模型档位解析（chat 模型切换）────────────────────────────────

    #[test]
    fn test_resolve_model_override_tiers() {
        assert_eq!(resolve_model_override(None, "flash-m").unwrap(), None);
        assert_eq!(
            resolve_model_override(Some("deep"), "flash-m").unwrap(),
            None
        );
        assert_eq!(
            resolve_model_override(Some("flash"), "flash-m").unwrap(),
            Some("flash-m".to_string())
        );
    }

    #[test]
    fn test_resolve_model_override_unknown_rejected() {
        let err = resolve_model_override(Some("gpt-9"), "flash-m").unwrap_err();
        assert!(err.contains("gpt-9"));
    }
}
