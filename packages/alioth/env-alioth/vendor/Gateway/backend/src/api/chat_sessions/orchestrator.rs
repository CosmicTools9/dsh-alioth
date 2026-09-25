use async_trait::async_trait;
use chrono::Utc;
use serde_json::Value;
use sqlx::PgPool;
use std::sync::Arc;

use ai_agent::agents::tool_orchestrator::ActionHandler;
use ai_agent::agents::{extract_json_block, tool_orchestrator};

use crate::api::chat_sessions::ports::{
    AIContactPort, AgentDispatchPort, LlmConfigPort, MessageMetaFields, MessageStorePort,
    SessionStorePort,
};
use crate::api::chat_sessions::{
    AgentInfoResponse, ChatMessageResponse, ChatSessionResponse, ExecuteActionResponse,
    ModelOptionResponse,
};

/// 上下文段组装（design D1/D8）：system_prompt + 当前页面上下文（4000 字符截断、
/// knowledgeContext 排除出页面段）+ 知识参考（2000）+ NGAC 权限 + 双层记忆
/// （主体层/对话方层各 1000，空层不注入）。
/// 纯函数——process_turn 与单测共用（page-context 自动获取机制验证）。
/// summary：早前对话滚动摘要（D2.11），None/空不注入段。
fn assemble_context_sections(
    system_prompt: &str,
    page_context: Option<&Value>,
    permissions: Option<&Value>,
    subject_memory: Option<&Value>,
    counterpart_memory: Option<&Value>,
    agent_state: Option<&Value>,
    summary: Option<&str>,
) -> String {
    let mut prompt = system_prompt.to_string();

    // 早前对话摘要（D2.11 滚动摘要段——长会话窗口剔除内容的压缩记忆）
    if let Some(s) = summary.map(str::trim).filter(|s| !s.is_empty()) {
        prompt.push_str("\n\n## 早前对话摘要\n");
        const SUMMARY_CAP: usize = 2000;
        if s.chars().count() > SUMMARY_CAP {
            let mut truncated: String = s.chars().take(SUMMARY_CAP).collect();
            truncated.push_str("…[truncated]");
            prompt.push_str(&truncated);
        } else {
            prompt.push_str(s);
        }
    }

    // C4（fix-chat-ai-capability-gaps D2.1）会话草稿上下文：draft_context.content
    // 主动精选（偏好/决策/关键实体/进行中事项，每 4 轮 flash 重写）——与早前
    // 摘要并列（summary=窗口溢出被动压缩、draft=会话内主动精选）；3000 字符
    // 截断；空/缺省不注入段。
    if let Some(draft) = agent_state
        .and_then(|s| s.get("draft_context"))
        .and_then(|d| d.get("content"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|c| !c.is_empty())
    {
        prompt.push_str("\n\n## 会话草稿上下文\n");
        prompt.push_str("以下是本会话进行中事项/已确认偏好与关键实体的草稿，持续精炼保持最新：\n");
        const DRAFT_CAP: usize = 3000;
        if draft.chars().count() > DRAFT_CAP {
            let mut truncated: String = draft.chars().take(DRAFT_CAP).collect();
            truncated.push_str("…[truncated]");
            prompt.push_str(&truncated);
        } else {
            prompt.push_str(draft);
        }
    }

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

    // 双层记忆注入（refactor-chat-ai-subject-identity-memory）：
    // L1 主体层跨对话方共享（跨会话累积的经验/偏好），L2 对话方层仅本对话方可见；
    // 各 ≤1000 字符，空层不注入段。键维度是智能体主体，不是人类账号。
    const MEMORY_LAYER_CAP: usize = 1000;
    if let Some(mem) = subject_memory {
        let memory_str = serde_json::to_string(mem).unwrap_or_default();
        if !memory_str.is_empty() && memory_str != "{}" {
            prompt.push_str("\n\n## 主体记忆\n");
            prompt.push_str(
                "以下是你作为该智能体主体跨会话累积的记忆（对所有对话方通用），请据此保持一致：\n",
            );
            if memory_str.chars().count() > MEMORY_LAYER_CAP {
                let mut truncated: String = memory_str.chars().take(MEMORY_LAYER_CAP).collect();
                truncated.push_str("…[truncated]");
                prompt.push_str(&truncated);
            } else {
                prompt.push_str(&memory_str);
            }
        }
    }
    if let Some(mem) = counterpart_memory {
        let memory_str = serde_json::to_string(mem).unwrap_or_default();
        if !memory_str.is_empty() && memory_str != "{}" {
            prompt.push_str("\n\n## 本对话方记忆\n");
            prompt.push_str("以下是与本对话方相关的记忆（仅适用于本对话方，勿外传）：\n");
            if memory_str.chars().count() > MEMORY_LAYER_CAP {
                let mut truncated: String = memory_str.chars().take(MEMORY_LAYER_CAP).collect();
                truncated.push_str("…[truncated]");
                prompt.push_str(&truncated);
            } else {
                prompt.push_str(&memory_str);
            }
        }
    }

    prompt
}

/// 工具白名单派生（E8，upgrade-chat-ai-tool-surface）：agent 声明的工具 →
/// 可经本路径执行的白名单。仅 `ExecutionTarget::Backend` 的声明入选。
/// 调用方 MUST 在派生结果为空时视为**无工具**（fail-closed），
/// MUST NOT 依赖 adapter 的「空集 = 不过滤」语义。
fn derive_allowed_tool_names(tools: &[ai_agent::agents::ToolDefinition]) -> Vec<String> {
    tools
        .iter()
        .filter(|t| {
            matches!(
                t.execution_target,
                ai_agent::agents::ExecutionTarget::Backend
            )
        })
        .map(|t| t.name.clone())
        .collect()
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
    /// 取消信号接收端（D2.9）；None = 不可取消（行为不变，非破坏）。
    pub cancel: Option<tokio::sync::watch::Receiver<bool>>,
}

/// 流式出向事件（D2.10 typed 帧 v2 的服务端模型）：文本增量 + 工具生命周期。
/// WS 层把事件映射为 {"type":"chunk"|"tool",...} 帧；HTTP 轮询路径无事件。
#[derive(Debug, Clone)]
pub enum TurnStreamEvent {
    /// LLM 文本增量（终答与工具前导均按原样转发）
    Chunk(String),
    ToolStart {
        name: String,
        arguments: serde_json::Value,
    },
    ToolEnd {
        name: String,
        success: bool,
        output: String,
    },
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
    /// 处理一轮对话。`on_chunk` 非 None 时启用流式出向（D2.10 typed 事件：
    /// 文本增量 → Chunk；工具执行 → ToolStart/ToolEnd）；None 时整段生成
    /// （兼容 HTTP 轮询调用方）。
    async fn process_turn(
        &self,
        input: TurnInput,
        on_chunk: Option<Box<dyn Fn(TurnStreamEvent) + Send + Sync>>,
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
        attachments: Option<Value>,
        knowledge_refs: Option<Value>,
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
    async fn list_agents(&self, locale: &str) -> Result<Vec<AgentInfoResponse>, String>;
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
    /// 文件存储状态（upgrade-chat-ai-context-coverage E4）：文档附件经
    /// `file_id` 引用取其字节。惰性构造一次——规避 8 处 `build_orchestrator`
    /// 调用点与 handler 签名改动。
    files_state: tokio::sync::OnceCell<crate::api::files::FilesState>,
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
            files_state: tokio::sync::OnceCell::new(),
        }
    }

    /// E4：文件存储状态（惰性构造一次）。文档附件经 `file_id` 引用取其字节，
    /// MUST NOT 走 base64 内联通道（图片维持 `data_base64` 唯一通道）。
    async fn files_state(&self) -> &crate::api::files::FilesState {
        self.files_state
            .get_or_init(|| async {
                crate::api::files::FilesState::from_live_db(self.pool.clone()).await
            })
            .await
    }

    fn derive_role(
        &self,
        sender_addr: Option<i64>,
        agent_contacts: &std::collections::HashSet<i64>,
    ) -> String {
        match sender_addr {
            Some(addr) if agent_contacts.contains(&addr) => "assistant".to_string(),
            _ => "user".to_string(),
        }
    }

    /// 取消检查（D2.9）：接收端 None（不可取消）或值 false → 未取消。
    fn cancelled(rx: Option<&tokio::sync::watch::Receiver<bool>>) -> bool {
        rx.map(|r| *r.borrow()).unwrap_or(false)
    }

    /// D2.6 attachments→vision：逐图 inspect_image 分析（上限 4）。
    /// S2：data_base64 唯一通道（url 拉取兜底已删除——SSRF 面关闭，前端只传
    /// base64）；R1：单图 decode 后 ≤4MiB，超限跳过。返回分析文本列表；
    /// 无图片/全部失败 → None（不注入段）。单图失败仅 warn。
    async fn analyze_turn_images(
        llm: &llm::LlmService,
        attachments: Option<&Value>,
    ) -> Option<Vec<String>> {
        let images: Vec<&Value> = attachments
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter(|a| a.get("type").and_then(|t| t.as_str()) == Some("image"))
                    .take(4)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if images.is_empty() {
            return None;
        }
        let mut analyses: Vec<String> = Vec::new();
        for img in images {
            let Some((mime, data)) = valid_image_bytes(img) else {
                common::telemetry::warn!(
                    "vision: 图片跳过（仅支持 data_base64 且单图 ≤4MiB；缺失/解码失败/超限）"
                );
                continue;
            };
            let content = llm::ImageContent { mime, data };
            match llm
                .inspect_image(
                    &content,
                    "请识别并详细描述这张图片的内容：提取其中的文字、数据、表格、布局与关键信息，供后续对话使用。",
                )
                .await
            {
                Ok(text) => analyses.push(text),
                Err(e) => {
                    common::telemetry::warn!("vision: inspect_image 失败（跳过该图）: {}", e);
                }
            }
        }
        if analyses.is_empty() {
            None
        } else {
            Some(analyses)
        }
    }

    /// C8/D2.4：HTTP execute-action 真实执行/拒绝审计。成功 → Permit；执行被
    /// 拒（CONFIRMATION_REQUIRED/handler 失败）→ Deny + error。
    /// metadata{action_type,target_count,confirmed,error?}；user_email 自会话
    /// 属主 auth_users 解析（回落 user:{id}）。await + 错误仅 telemetry——
    /// 审计写入不阻断执行主链。
    ///
    /// 参数（8，含 `&self`）：会话/动作/属主/动作类型/目标数/确认位/结果——均为审计载荷的
    /// 独立字段，拆结构体只在该调用点加一层噪声，按 documented exemption 处置。
    #[allow(clippy::too_many_arguments)]
    async fn audit_action_execution(
        &self,
        session_id: i64,
        action_id: &str,
        user_id: i64,
        action_type: &str,
        target_count: usize,
        confirmed: bool,
        outcome: &Result<Value, String>,
    ) {
        use common::audit::{record_audit_event_with_metadata, Decision};
        let email = super::adapters::tool_bridge::resolve_user_email(&self.pool, user_id).await;
        let (decision, metadata) = match outcome {
            Ok(_) => (
                Decision::Permit,
                serde_json::json!({
                    "action_type": action_type,
                    "target_count": target_count,
                    "confirmed": confirmed,
                }),
            ),
            Err(e) => (
                Decision::Deny,
                serde_json::json!({
                    "action_type": action_type,
                    "target_count": target_count,
                    "confirmed": confirmed,
                    "error": e,
                }),
            ),
        };
        if let Err(e) = record_audit_event_with_metadata(
            &self.pool,
            user_id,
            &email,
            &format!("chat-sessions/{}/actions/{}", session_id, action_id),
            "chat_ai.action.execute",
            &decision,
            metadata,
        )
        .await
        {
            common::telemetry::warn!("chat_ai.action.execute audit failed: {}", e);
        }
    }
}

/// 单图 decode 上限：4 MiB（S2/R1，HTTP 与 WS 两路共用此校验——两路附件最终
/// 都以 data_base64 落消息 meta，process_turn 统一经 analyze_turn_images 消费）。
const MAX_IMAGE_BYTES: usize = 4 * 1024 * 1024;

/// S2/R1 图片字节校验：仅 data_base64 通道；decode 失败或超 4MiB → None。
/// 返回 (mime, 原始字节)。纯函数——HTTP/WS 附件共用（单测覆盖）。
fn valid_image_bytes(img: &Value) -> Option<(String, Vec<u8>)> {
    use base64::Engine;
    let mime = img
        .get("mime")
        .and_then(|m| m.as_str())
        .unwrap_or("image/png")
        .to_string();
    let b64 = img.get("data_base64").and_then(|d| d.as_str())?;
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64).ok()?;
    if bytes.len() > MAX_IMAGE_BYTES {
        return None;
    }
    Some((mime, bytes))
}

/// 主体 id 解析：无主体行 ⇒ 显式失败（MUST NOT 回落按人类账号写记忆）。
/// 纯函数——失败语义可单测（`refactor-chat-ai-subject-identity-memory` task 2.4）。
fn require_subject_id(
    agent_config: &ai_agent::agents::AgentConfig,
    agent_code: &str,
) -> Result<i64, String> {
    agent_config.subject_id.ok_or_else(|| {
        format!(
            "AGENT_SUBJECT_MISSING: agent '{agent_code}' 无主体行（isahl.zc_id_empl-agent）；\
             请先落 namespace 级主体行与联系方式种子（code=agent-{agent_code}）"
        )
    })
}

/// 单轮文档附件上限（与图片同口径：4 份）。
const MAX_DOCUMENT_ATTACHMENTS: usize = 4;

/// 本轮是否含文档附件（决定是否触碰文件存储；无文档的轮次 MUST NOT 依赖文件服务）。
fn has_document_attachments(attachments: Option<&Value>) -> bool {
    attachments
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .any(|a| a.get("type").and_then(|t| t.as_str()) == Some("document"))
        })
        .unwrap_or(false)
}

/// 单份文档附件的解析结果（E4）：`text` 为 None ⇒ 未解析（`reason` 说明原因）。
/// 未解析 MUST NOT 阻断该轮，仅在 prompt 内显式提示。
struct DocumentParse {
    name: String,
    text: Option<String>,
    reason: Option<String>,
}

/// E4：本轮文档附件（file 引用通道）逐份解析为文本。
///
/// 通道约束：文档走 `file_id` → 文件存储取字节（`api/files`），MUST NOT 走
/// base64 内联；行级授权沿用文件服务（`get_metadata(file_id, Some(user_id))`——
/// 非本人创建且不在授权列的 file 不可读）。每份解析失败只记原因、不中断该轮。
async fn analyze_turn_documents(
    files: &crate::api::files::FilesState,
    user_id: i64,
    attachments: Option<&Value>,
) -> Vec<DocumentParse> {
    let documents: Vec<&Value> = attachments
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter(|a| a.get("type").and_then(|t| t.as_str()) == Some("document"))
                .take(MAX_DOCUMENT_ATTACHMENTS)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let mut parsed = Vec::new();
    for doc in documents {
        let name = doc
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or("未命名文档")
            .to_string();
        let failed = |reason: String| DocumentParse {
            name: name.clone(),
            text: None,
            reason: Some(reason),
        };
        // ID_JSON_PRECISION：前端按字符串传 zuid，后端兼容数字形态
        let Some(file_id) = doc.get("file_id").and_then(|v| {
            v.as_i64()
                .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
        }) else {
            parsed.push(failed("缺少文件引用 file_id".to_string()));
            continue;
        };
        let record = match files.service.get_metadata(file_id, Some(user_id)).await {
            Ok(Some(record)) => record,
            Ok(None) => {
                parsed.push(failed("文件不存在或无权访问".to_string()));
                continue;
            }
            Err(e) => {
                parsed.push(failed(format!("读取文件元数据失败: {e}")));
                continue;
            }
        };
        let download = match files.service.download(file_id).await {
            Ok(download) => download,
            Err(e) => {
                parsed.push(failed(format!("读取文件失败: {e}")));
                continue;
            }
        };
        // 文件名以存储记录为准（前端传入的 name 仅作展示回退）
        let filename = record
            .filename
            .filter(|f| !f.trim().is_empty())
            .unwrap_or_else(|| download.filename.clone());
        match super::attachment_parse::extract_text_async(filename, download.data).await {
            Ok(text) => parsed.push(DocumentParse {
                name,
                text: Some(text),
                reason: None,
            }),
            Err(e) => parsed.push(failed(e.to_string())),
        }
    }
    parsed
}

/// R5（D2.7）后台 flash 任务（摘要/记忆沉淀）失败重试节流窗：距上次尝试 ≥60s
/// 才允许再次发起；期间跳过——防持久失败下每轮打 LLM。
const BG_RETRY_THROTTLE_SECS: i64 = 60;

/// R5：后台任务重试门禁。status ∈ {failed, pending} 且距 last_attempt_at <60s →
/// 节流跳过（pending = 在飞/崩溃遗留，超窗自动放行）；ok / 无记录 → 放行。
fn bg_retry_allowed(state: Option<&Value>, now_secs: i64) -> bool {
    let Some(st) = state else {
        return true;
    };
    let Some(status) = st.get("status").and_then(|v| v.as_str()) else {
        return true;
    };
    match status {
        "failed" | "pending" => {
            let last = st
                .get("last_attempt_at")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            now_secs.saturating_sub(last) >= BG_RETRY_THROTTLE_SECS
        }
        _ => true,
    }
}

/// R5：agent_state 任务状态补丁 {<key>: {status, last_attempt_at}}（动态键——
/// serde_json json! 不支持表达式键，手动 Map 组装）。
fn bg_task_state_patch(key: &str, status: &str, attempt_at: i64) -> Value {
    let mut obj = serde_json::Map::new();
    obj.insert(
        key.to_string(),
        serde_json::json!({ "status": status, "last_attempt_at": attempt_at }),
    );
    Value::Object(obj)
}

/// R5：记忆沉淀触发判定——每 5 轮周期 或 上次失败强制重试（非周期轮），均受
/// 60s 节流约束（pending 在飞时周期轮同样跳过防重复 spawn）。
fn memory_consolidation_due(state: Option<&Value>, turn_count: i64, now_secs: i64) -> bool {
    let periodic = turn_count % 5 == 0;
    let forced = state.and_then(|s| s.get("status")).and_then(|v| v.as_str()) == Some("failed");
    (periodic || forced) && bg_retry_allowed(state, now_secs)
}

/// C4（D2.1）draft-context 精炼触发判定：每 4 轮周期 或 上次失败强制重试，
/// 均受 60s 节流约束（复用 R5 bg_retry_allowed；key=draft_context_state）。
fn draft_context_due(state: Option<&Value>, turn_count: i64, now_secs: i64) -> bool {
    let periodic = turn_count % 4 == 0;
    let forced = state.and_then(|s| s.get("status")).and_then(|v| v.as_str()) == Some("failed");
    (periodic || forced) && bg_retry_allowed(state, now_secs)
}

/// C4：成功 patch——重写式更新 draft_context{content,updated_at,source_turn_count}
/// + 任务状态 ok（一次落库；改写非 append，防无界增长）。纯函数（单测覆盖
///   「重写式更新」形态）。
fn draft_context_success_patch(content: &str, source_turn_count: i64, attempt_at: i64) -> Value {
    let mut patch = bg_task_state_patch("draft_context_state", "ok", attempt_at);
    patch["draft_context"] = serde_json::json!({
        "content": content,
        "updated_at": attempt_at,
        "source_turn_count": source_turn_count,
    });
    patch
}

/// C4：flash 精炼 prompt 组装（D2.1 精炼指令原文）——输入 = 旧 draft.content
/// （无则占位「（无）」）+ 最近对话。纯函数（单测覆盖「prompt 组装」）。
fn draft_refine_prompt(old_draft: &str, recent_text: &str) -> String {
    let draft_slot = if old_draft.trim().is_empty() {
        "（无）".to_string()
    } else {
        old_draft.to_string()
    };
    format!(
        "你是会话草稿上下文管理员。当前草稿：{}\n\n最近对话：\n{}\n\n\
         任务：提取并更新会话草稿上下文：保留仍然相关的偏好/决策/关键实体/进行中事项，\
         移除已过时项，合并新信息。输出完整新草稿（≤1500 字，markdown 要点式），\
         无新增则原样输出。",
        draft_slot, recent_text
    )
}

/// 「L1 禁入类」规则集（refactor-chat-ai-subject-identity-memory D-1：spec 固化 +
/// 代码常量实现）——命中即强制落 L2 对话方层，MUST NOT 进主体层（L1 跨对话方共享）。
/// 匹配 key 与序列化值（小写子串；CJK 不受大小写影响）。常量即规约的可审计锚点。
const L1_DENY_PATTERNS: &[&str] = &[
    // 英文键
    "amount",
    "price",
    "cost",
    "salary",
    "phone",
    "mobile",
    "email",
    "contact",
    "address",
    "id_card",
    "idno",
    "bank",
    "account",
    "password",
    "token",
    "customer",
    "client",
    "personal",
    "private",
    // 中文键/值
    "金额",
    "价格",
    "单价",
    "费用",
    "工资",
    "薪酬",
    "手机",
    "电话",
    "邮箱",
    "地址",
    "身份证",
    "银行",
    "账号",
    "账户",
    "密码",
    "口令",
    "客户",
    "联系人",
    "个人信息",
    "隐私",
];

/// 单条记忆条目是否命中禁入类（key 或序列化值子串命中）。
fn hits_l1_deny(key: &str, value: &Value) -> bool {
    let key_l = key.to_lowercase();
    if L1_DENY_PATTERNS.iter().any(|p| key_l.contains(p)) {
        return true;
    }
    let value_l = value.to_string().to_lowercase();
    L1_DENY_PATTERNS.iter().any(|p| value_l.contains(p))
}

/// 模型输出 → 双层归属。约定输出 `{subject, counterpart}`；缺键时容忍为
/// 「整对象即主体层」（旧形态兼容）并保留旧对话方层，避免误清空。
fn split_memory_layers(raw: Value, old_subject: &Value, old_counterpart: &Value) -> (Value, Value) {
    let has_layers = raw.get("subject").is_some() || raw.get("counterpart").is_some();
    if !has_layers {
        return (raw, old_counterpart.clone());
    }
    let subject = raw
        .get("subject")
        .filter(|v| v.is_object())
        .cloned()
        .unwrap_or_else(|| old_subject.clone());
    let counterpart = raw
        .get("counterpart")
        .filter(|v| v.is_object())
        .cloned()
        .unwrap_or_else(|| old_counterpart.clone());
    (subject, counterpart)
}

/// 归属兜底：把主体层中命中禁入类的条目移入对话方层（模型误判在此被改写）。
/// 返回被改写的条目数（供单测断言）。对话方层已有同名键时保留其现值。
fn enforce_l1_deny_rules(subject: &mut Value, counterpart: &mut Value) -> usize {
    let Some(subj_obj) = subject.as_object_mut() else {
        return 0;
    };
    let keys: Vec<String> = subj_obj.keys().cloned().collect();
    let mut moved: Vec<(String, Value)> = Vec::new();
    for k in keys {
        let hit = subj_obj
            .get(&k)
            .map(|v| hits_l1_deny(&k, v))
            .unwrap_or(false);
        if hit {
            if let Some(v) = subj_obj.remove(&k) {
                moved.push((k, v));
            }
        }
    }
    if moved.is_empty() {
        return 0;
    }
    let n = moved.len();
    if !counterpart.is_object() {
        *counterpart = serde_json::json!({});
    }
    if let Some(cp) = counterpart.as_object_mut() {
        for (k, v) in moved {
            cp.entry(k).or_insert(v);
        }
    }
    n
}

/// R6（D2.9）UI locale 短码 → 自然语言名：zh 前缀 → 中文，其余 → English（纯函数）。
fn locale_native_name(locale: &str) -> &'static str {
    if locale.to_lowercase().starts_with("zh") {
        "中文"
    } else {
        "English"
    }
}

// ============================================================
// 运行时上下文（批 ④ add-chat-ai-runtime-context）：运行身份（system 前言）
// 与当前运行时刻（prompt 尾部）
// ============================================================

/// 运行身份段安全冗余上限（字段本身有界）。
const IDENTITY_CAP: usize = 500;

/// 运行身份（design D2）：静态事实 ⇒ 进 system 前言（不破坏前缀缓存）。
pub struct RuntimeIdentity<'a> {
    pub namespace: &'a str,
    pub app_code: &'a str,
    pub agent_code: &'a str,
    pub subject_id: i64,
}

/// 本地时刻字符串（`chrono::Local`，含 UTC 偏移）。
fn local_now_label() -> String {
    chrono::Local::now()
        .format("%Y-%m-%d %H:%M:%S (UTC%:z)")
        .to_string()
}

/// Gateway 实例 namespace（未设置/空串 → "default"）。
fn runtime_namespace() -> String {
    std::env::var("NAMESPACE")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "default".to_string())
}

/// Gateway 实例 app code —— 与 `main.rs` AppContext 同语义同优先级：
/// `GATEWAY_APP_CODE`（含空串）> `NAMESPACE` > `"default"`；空串不回落到
/// `NAMESPACE`（`main.rs:423-428` 的 `.ok().or_else(..).filter(..)` 形态）。
fn runtime_app_code() -> String {
    std::env::var("GATEWAY_APP_CODE")
        .ok()
        .or_else(|| std::env::var("NAMESPACE").ok())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "default".to_string())
}

/// 主体人格截断上限（design D3）。
const PERSONA_CAP: usize = 1500;

/// 主体人格段（add-agent-persona-channel D3，纯函数）：非空则产出，置于 system
/// 前言（人格 → 运行身份）；人格是主体级**静态**事实 ⇒ 不破坏前缀缓存。
/// 空/空白 → 空串（不产段，沿用「空层不注入段」）。
fn subject_persona_section(persona: Option<&str>) -> String {
    let Some(persona) = persona.map(str::trim).filter(|p| !p.is_empty()) else {
        return String::new();
    };
    format!(
        "\n\n## 主体人格\n以下是你作为该智能体主体的稳定人格设定，回答与判断须与其保持一致：\n{}",
        truncate_chars_with(persona, PERSONA_CAP, "…[truncated]")
    )
}

/// 运行身份段（纯函数）：拼在 system 前言（system_prompt + locale 指令）之后。
/// 身份字段为静态值（namespace/app/agent 均由部署或注册表决定，用户输入不可达）；
/// 超长时按**字符**截断（MUST NOT 字节切片——多字节边界会 panic）。
fn runtime_identity_block(id: &RuntimeIdentity<'_>) -> String {
    let identity = format!(
        "- namespace: {}\n- app: {}\n- agent: {}\n- 主体 id: {}\n",
        id.namespace, id.app_code, id.agent_code, id.subject_id
    );
    let identity = truncate_chars_with(&identity, IDENTITY_CAP, "…[truncated]");
    let mut out = String::new();
    out.push_str("\n\n## 运行身份\n");
    out.push_str("你正在以下运行时身份下作答（数据访问与权限以此为准）：\n");
    out.push_str(&identity);
    out
}

/// 当前运行时刻段（纯函数）：秒级动态事实 ⇒ 置于 **prompt 尾部**（MUST NOT 进
/// system 前言——否则每轮 system 变化、服务商前缀缓存归零；纪律同
/// `docs/specs/META_AI_SPEC.md`「动态环境事实置 user prompt 尾部」）。
fn runtime_time_section(now: &str, locale: &str) -> String {
    format!(
        "\n\n## 当前运行时刻\n{}（locale: {}）\n涉及「现在/今天/本周」等时间判断时以此为准。\n",
        now, locale
    )
}

/// 工具执行注记（批 ④ T4，design D4）：把 E7 落库的 `tool_calls` 渲染为紧凑
/// 单行注记——单调用 output ≤300 字符、arguments ≤80 字符。注记**物理总长**
/// （含包装与截断标记）≤600 字符：超出即从尾部逐片丢弃并标截断。
/// 空/非数组 → None（不追加）。
const TOOL_NOTE_OUTPUT_CAP: usize = 300;
const TOOL_NOTE_ARGS_CAP: usize = 80;
const TOOL_NOTE_TOTAL_CAP: usize = 600;

fn format_tool_trace(tool_calls: &Value) -> Option<String> {
    let calls = tool_calls.as_array()?;
    if calls.is_empty() {
        return None;
    }
    let mut parts: Vec<String> = Vec::new();
    for call in calls {
        let name = call
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let args = call
            .get("arguments")
            .map(|v| v.to_string())
            .unwrap_or_default();
        let success = call
            .get("success")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let output = call
            .get("output")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        parts.push(format!(
            "{}({}) → {}: {}",
            name,
            take_chars(args.as_str(), TOOL_NOTE_ARGS_CAP),
            if success { "ok" } else { "fail" },
            take_chars(output, TOOL_NOTE_OUTPUT_CAP)
        ));
    }
    if parts.is_empty() {
        return None;
    }
    // 物理长度封顶：整段（含包装/截断标记）MUST ≤ TOOL_NOTE_TOTAL_CAP 字符。
    let mut dropped = false;
    loop {
        let note = compose_tool_note(&parts, dropped);
        if note.chars().count() <= TOOL_NOTE_TOTAL_CAP || parts.len() == 1 {
            return Some(note);
        }
        parts.pop();
        dropped = true;
    }
}

/// 组装注记文本（`dropped` = 是否已丢弃后续调用）。
fn compose_tool_note(parts: &[String], dropped: bool) -> String {
    let mut note = format!("（本轮工具执行：{}", parts.join("；"));
    if dropped {
        note.push_str("；…[工具记录截断]");
    }
    note.push('）');
    note
}

/// 历史行渲染（批 ④ T4，design D4）：assistant 行携带工具注记，其余行原样。
/// 注记只进 prompt，MUST NOT 回写消息 `content`。
fn history_line(role: &str, content: &str, tool_calls: Option<&Value>) -> String {
    let mut line = content.to_string();
    if role == "assistant" {
        if let Some(note) = tool_calls.and_then(format_tool_trace) {
            line.push_str(&note);
        }
    }
    line
}

/// 字符数截断（超限追加 `marker`）——唯一实现，避免各处自写字节切片踩多字节边界。
fn truncate_chars_with(text: &str, cap: usize, marker: &str) -> String {
    if text.chars().count() > cap {
        let mut s: String = text.chars().take(cap).collect();
        s.push_str(marker);
        s
    } else {
        text.to_string()
    }
}

/// 按字符数截断（超限追加 `…[截断]`）——委托 `truncate_chars_with`（单一实现）。
fn take_chars(text: &str, cap: usize) -> String {
    truncate_chars_with(text, cap, "…[截断]")
}

/// R6：语言指令段——注入 system_prompt 尾（design D2.9 措辞：用户界面语言 +
/// 始终使用该语言回复），约束 LLM 以与 UI 一致的语言作答；\n 包裹为独立段。
fn locale_instruction(locale: &str) -> String {
    format!(
        "\n## 语言指令\n用户界面语言: {}。请始终使用该语言回复。\n",
        locale_native_name(locale)
    )
}

// ============================================================
// C7 structured 服务端校验（fix-chat-ai-capability-gaps D2.3）
// ============================================================

/// filled_fields 字段值形态：object{value,confidence?,source?}（须含 value 且
/// 键 ⊆ 白名单）或 JSON 原始标量（bool/number/string/null）——array/缺 value
/// 的 object 属幻觉形态，不合法。
fn filled_field_value_ok(v: &Value) -> bool {
    match v {
        Value::Object(o) => {
            o.contains_key("value")
                && o.keys()
                    .all(|k| matches!(k.as_str(), "value" | "confidence" | "source"))
        }
        Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Null => true,
        _ => false,
    }
}

/// actions 元素：object 且含非空 id/label + kind ∈ {message,execute,page_action}
/// （page_action 的 op/args 等扩展键保留）。
fn action_element_ok(v: &Value) -> bool {
    let Some(o) = v.as_object() else {
        return false;
    };
    let id_ok = o
        .get("id")
        .and_then(|x| x.as_str())
        .is_some_and(|s| !s.is_empty());
    let label_ok = o
        .get("label")
        .and_then(|x| x.as_str())
        .is_some_and(|s| !s.is_empty());
    let kind_ok = matches!(
        o.get("kind").and_then(|x| x.as_str()),
        Some("message" | "execute" | "page_action")
    );
    id_ok && label_ok && kind_ok
}

/// C7（D2.3）纯函数：extract_json_block 结果的服务端形态校验。
/// - filled_fields 存在须 object：字段值须 object{value,confidence?,source?} 或
///   标量——不符剔除该字段（warn）；剔除后空 object → 连坐剔除键。
/// - actions 存在须 array：元素须含 id+label+kind ∈ {message,execute,page_action}
///   ——不符剔除元素（warn）；空 array → 连坐剔除键。
/// - 其余键保留。全部剔除 → None（structured 降级纯文本，不存 meta）。
///
/// process_turn 在 extract_json_block 后调用（structured 落 ChatMessageResponse
/// 与 meta 之前）。
fn validate_structured(v: Value) -> Option<Value> {
    let Value::Object(mut obj) = v else {
        return None; // 根非 object：整体不合法
    };
    if let Some(ff) = obj.get("filled_fields").cloned() {
        match ff {
            Value::Object(fields) => {
                let mut cleaned = serde_json::Map::new();
                for (k, val) in fields {
                    if filled_field_value_ok(&val) {
                        cleaned.insert(k.clone(), val);
                    } else {
                        common::telemetry::warn!(
                            "structured 校验：filled_fields.{} 形态不合法，剔除该字段",
                            k
                        );
                    }
                }
                if cleaned.is_empty() {
                    common::telemetry::warn!("structured 校验：filled_fields 全剔，剔除键");
                    obj.remove("filled_fields");
                } else {
                    obj.insert("filled_fields".to_string(), Value::Object(cleaned));
                }
            }
            other => {
                common::telemetry::warn!(
                    "structured 校验：filled_fields 非 object（{}），剔除键",
                    other
                );
                obj.remove("filled_fields");
            }
        }
    }
    if let Some(acts) = obj.get("actions").cloned() {
        match acts {
            Value::Array(elems) => {
                let kept: Vec<Value> = elems
                    .iter()
                    .filter(|e| {
                        let ok = action_element_ok(e);
                        if !ok {
                            common::telemetry::warn!(
                                "structured 校验：actions 元素不合法（缺 id/label 或 kind 越界），剔除"
                            );
                        }
                        ok
                    })
                    .cloned()
                    .collect();
                if kept.is_empty() {
                    common::telemetry::warn!("structured 校验：actions 全剔，剔除键");
                    obj.remove("actions");
                } else {
                    obj.insert("actions".to_string(), Value::Array(kept));
                }
            }
            other => {
                common::telemetry::warn!("structured 校验：actions 非 array（{}），剔除键", other);
                obj.remove("actions");
            }
        }
    }
    if obj.is_empty() {
        None
    } else {
        Some(Value::Object(obj))
    }
}

#[async_trait]
impl SessionOrchestrator for DefaultSessionOrchestrator {
    async fn process_turn(
        &self,
        input: TurnInput,
        on_chunk: Option<Box<dyn Fn(TurnStreamEvent) + Send + Sync>>,
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

        // 权限每轮刷新（refactor-chat-ai-subject-identity-memory D10）：以当前认证主体
        // 重新解析 NGAC 权限范围，MUST NOT 沿用会话创建时的快照（主体/岗位变化即时生效）。
        // 解析失败 ⇒ **fail-closed**：不注入权限段（`None`），MUST NOT 回退陈旧快照
        // （陈旧范围可能已含被回收的授权）。数据访问仍由 PEP 每次调用独立裁决。
        let permissions =
            match crate::ngac::resolve_user_permissions(&self.pool, input.user_id).await {
                Ok(perm) => Some(perm),
                Err(e) => {
                    common::telemetry::warn!(
                        "permissions refresh failed: {} (fail-closed: no permission section)",
                        e
                    );
                    None
                }
            };

        // D2.8 标题自动生成前置判定：会话 notice 仍为默认值时本轮成功后异步生成。
        let needs_auto_title = {
            let session_title = session
                .as_ref()
                .map(|s| s.title.clone())
                .unwrap_or_default();
            let default_i18n = self
                .i18n
                .read()
                .await
                .get(&locale, "chat.session.defaultTitle")
                .unwrap_or("New Chat")
                .to_string();
            session_title == default_i18n
                || session_title == "New Chat"
                || session_title == "EmpAgent"
        };

        // 2. 加载 page_context + agent_state
        let (page_context, agent_state) = self
            .session_store
            .get_session_context(input.session_id, input.user_id)
            .await?;

        // 3. 获取最后一条用户消息（完整行：content + meta——本轮附件/知识引用）
        //    角色判定按「发送方是否为智能体侧联系方式」（code 前缀 agent-），
        //    取代单一共享联系人（refactor-chat-ai-subject-identity-memory D-3）。
        let agent_contacts = super::memory_scope::agent_contact_id_set(&self.pool)
            .await
            .unwrap_or_default();
        let last_user = self
            .message_store
            .get_last_user_message_row(input.session_id)
            .await?
            .ok_or_else(|| "NO_USER_MESSAGE".to_string())?;
        let user_content = last_user.content.clone().unwrap_or_default();
        // 本轮用户消息 meta（assistant 回复 knowledge_refs 回显 + 附件视觉处理共用）
        let turn_knowledge_refs = last_user.knowledge_refs.clone();
        let turn_attachments = last_user.attachments.clone();

        // 4. D2.11 历史窗口：settings.max_history_messages（默认 50）；超窗时
        //    最老溢出条 → 后台滚动摘要（本次 turn 仍用旧 summary——异步语义）
        let history_window = self.llm_config.max_history_messages().await.max(2);
        let mut history_rows = self
            .message_store
            .get_history(input.session_id, input.user_id, history_window + 1)
            .await?;
        // rows ASC（最近窗口+1）——超窗时最老一条 = 溢出，移出窗口
        let overflow = if history_rows.len() as i64 > history_window {
            Some(history_rows.remove(0))
        } else {
            None
        };
        let history = history_rows
            .iter()
            .map(|msg| {
                let role = self.derive_role(msg.fk_sender_addr, &agent_contacts);
                // 批 ④ T4（design D4）：带工具记录的 assistant 行追加紧凑注记，
                // 使后续轮次可引用上轮工具原始结果；只进 prompt，不回写 content。
                let content = history_line(
                    &role,
                    &msg.content.clone().unwrap_or_default(),
                    msg.tool_calls.as_ref(),
                );
                (role, content)
            })
            .collect::<Vec<_>>();
        // 旧滚动摘要（agent_state.conversation_summary；本次 prompt 注入旧值）
        let old_summary: Option<String> = agent_state
            .as_ref()
            .and_then(|s| s.get("conversation_summary"))
            .and_then(|v| v.as_str())
            .map(String::from)
            .filter(|s| !s.is_empty());

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

        // 7b. 主体身份与记忆作用域（refactor-chat-ai-subject-identity-memory）：
        // L1 键 = 智能体主体行 id；L2 键第二维 = 对话方联系人（参与方集合 ∖ 智能体侧
        // → 聚合到 zc_id_contacts）。主体未 materialize ⇒ 显式失败，MUST NOT 回落为
        // 按人类账号（user_id）写记忆。
        let subject_id = require_subject_id(&agent_config, &agent_code)?;
        let subject_contact_id =
            super::memory_scope::resolve_subject_contact_id(&self.pool, &agent_code)
                .await?
                .ok_or_else(|| {
                    format!(
                        "AGENT_CONTACT_MISSING: agent '{agent_code}' 无独立联系方式\
                         （isahl.zc_id_contact_infos code=agent-{agent_code}）；\
                         MUST 由 namespace 级种子 materialize（无共享联系人兜底）"
                    )
                })?;
        let agent_side_info_ids: Vec<i64> = vec![subject_contact_id];
        let counterpart_id = super::memory_scope::CounterpartResolver::new(self.pool.clone())
            .resolve(input.session_id, input.user_id, &agent_side_info_ids)
            .await
            .unwrap_or_else(|e| {
                common::telemetry::warn!("counterpart resolve failed: {}", e);
                None
            });
        // D-2：AI 回复收件人 = 对话方首选联系方式（使「最后一条消息」可直接算出对话方）
        let counterpart_info = match counterpart_id {
            Some(cid) => {
                framework_contacts::ContactsService::resolve_preferred_info(&self.pool, cid)
                    .await
                    .unwrap_or(None)
            }
            None => None,
        };
        let assistant_recipients: Vec<i64> = counterpart_info.into_iter().collect();
        let memory_layers = super::memory_store::ChatMemoryStore::new(self.pool.clone())
            .load(subject_id, counterpart_id)
            .await
            .unwrap_or_else(|e| {
                common::telemetry::warn!("memory load failed: {}", e);
                super::memory_store::MemoryLayers::default()
            });

        // 批 ④/⑤（add-agent-persona-channel D4）：主体人格每轮直读主体行（`soul`，
        // 由 agent 管理面写入）⇒ 管理页保存后下一轮即生效（不经 registry TTL）；
        // 空/NULL 不产段；读取失败 warn 降级不阻断该轮。
        let persona = super::memory_scope::load_subject_persona(&self.pool, subject_id)
            .await
            .unwrap_or_else(|e| {
                common::telemetry::warn!("subject persona load failed: {}", e);
                None
            });

        // 8. 统一构建 prompt（上下文段组装为纯函数 assemble_context_sections，
        //    页面上下文/知识/权限/双层记忆/agent_state 段可单测——page-context 机制验证）
        //    R6（D2.9）语言感知：先向 system_prompt 尾追加 UI locale 语言指令段
        //    （locale 短码→中文/English 自然语言名），约束回复语言与界面一致；
        //    仅本 turn 生效——不动 agent 定义文件。
        //    批 ④/⑤：system 前言 = system_prompt + 语言指令 + 主体人格（静态，
        //    主体级）+ 运行身份（静态）；时刻段见 prompt 尾部。
        let runtime_ns = runtime_namespace();
        let runtime_app = runtime_app_code();
        let runtime_identity = RuntimeIdentity {
            namespace: &runtime_ns,
            app_code: &runtime_app,
            agent_code: &agent_code,
            subject_id,
        };
        let system_prompt = format!(
            "{}{}{}{}",
            agent_config.system_prompt,
            locale_instruction(&locale_str),
            subject_persona_section(persona.as_deref()),
            runtime_identity_block(&runtime_identity)
        );
        let mut prompt = assemble_context_sections(
            &system_prompt,
            page_context.as_ref(),
            permissions.as_ref(),
            Some(&memory_layers.subject),
            memory_layers.counterpart.as_ref(),
            agent_state.as_ref(),
            old_summary.as_deref(),
        );

        // D2.6 attachments→vision：本轮图片（≤4）逐图 llm.inspect_image 分析，
        // 文本拼「## 附件图片内容」段；S2 data_base64 唯一通道（url 兜底已删）；
        // 单图失败 warn 跳过不阻断（vision 后端未配置时 provider 报错同降级）。
        if let Some(image_analyses) =
            Self::analyze_turn_images(&llm, turn_attachments.as_ref()).await
        {
            prompt.push_str("\n\n## 附件图片内容\n");
            for (i, text) in image_analyses.iter().enumerate() {
                prompt.push_str(&format!("图片{}：{}\n", i + 1, text));
            }
        }

        // E4：文档附件（txt/md/csv/json/pdf/docx/xlsx）经 `file_id` → 文件存储取字节
        // → 抽取文本拼「## 附件文档内容」段；单份失败标「附件未解析」+ 原因，不阻断该轮。
        // 无文档附件的轮次 MUST NOT 触碰文件存储（不构造 FilesState、不加 DB 往返）。
        if has_document_attachments(turn_attachments.as_ref()) {
            let document_parses = analyze_turn_documents(
                self.files_state().await,
                input.user_id,
                turn_attachments.as_ref(),
            )
            .await;
            prompt.push_str("\n\n## 附件文档内容\n");
            for parse in &document_parses {
                match &parse.text {
                    Some(text) => prompt.push_str(&format!("【{}】\n{}\n", parse.name, text)),
                    None => prompt.push_str(&format!(
                        "【{}】附件未解析（{}）\n",
                        parse.name,
                        parse.reason.as_deref().unwrap_or("未知原因")
                    )),
                }
            }
        }

        prompt.push_str("\n\n## 对话历史\n");
        for (role, content) in &history {
            prompt.push_str(&format!("{}: {}\n", role, content));
        }

        prompt.push_str(&format!("\n\n## 用户最新消息\n{}\n", user_content));

        // 批 ④ T1（design D2/W4）：当前运行时刻段置 **prompt 尾部**——秒级动态事实
        // 不进 system 前言（否则每轮 system 变化、前缀缓存归零；纪律同
        // `docs/specs/META_AI_SPEC.md`「动态环境事实置 user prompt 尾部」）。
        prompt.push_str(&runtime_time_section(&local_now_label(), &locale_str));

        // 9. 统一执行：ToolOrchestrator（有工具）或直调 LLM（无工具）
        //    turn_usage：本轮 token 用量（assistant meta/响应回显；流式路径无 usage）
        let mut turn_usage: Option<Value> = None;
        // D2.9 取消检查点：整个生成段之前
        if Self::cancelled(input.cancel.as_ref()) {
            return Err("CANCELLED".to_string());
        }
        // 工具白名单派生（E8，upgrade-chat-ai-tool-surface）：仅 `ExecutionTarget::Backend`
        // 的声明可经本路径执行。`available_tools` 非空但派生白名单为空（如全为 Frontend
        // 目标）⇒ 视为**无工具**，MUST NOT 落入 adapter 的「空集 = 不过滤」语义（fail-closed）。
        let allowed_names: Vec<String> = derive_allowed_tool_names(&agent_config.available_tools);
        if !agent_config.available_tools.is_empty() && allowed_names.is_empty() {
            common::telemetry::warn!(
                "chat-ai: agent '{}' available_tools 非空但无 Backend 执行目标 ⇒ 视为无工具（fail-closed）",
                agent_code
            );
        }
        // E7：本轮工具调用记录（落 meta，供追溯；读取侧由历史重建渲染有界注记
        // ——批 ④ tool-trace-cross-turn，见 chat-ai#chat-ai-tool-result-persistence）
        let mut turn_tool_calls: Option<Value> = None;
        let content = if !allowed_names.is_empty() {
            let llm_port = Box::new(
                tool_orchestrator::LlmServiceAdapter::new(&llm)
                    .with_model_override(model_override.clone()),
            );
            let db_tools = tool_orchestrator::DbToolAdapter::new(self.pool.clone())
                .with_allowed_tools(allowed_names.clone());
            // C3/D2.2：一律经 GatewayToolPort 包装（execute_action 仅当白名单含
            // 该名才注入 handler 执行，其余委托 db_tools）——所有出向 ToolResult
            // 统一做 >4000 字符截断（含纯 DB 工具 agent，截断不依赖动作注入）。
            let tool_port: Box<dyn tool_orchestrator::ToolExecutionPort> =
                Box::new(super::adapters::tool_bridge::GatewayToolPort::new(
                    db_tools,
                    std::sync::Arc::new(super::adapters::tool_bridge::GatewayActionHandler::new(
                        self.pool.clone(),
                    )),
                    self.pool.clone(),
                    &allowed_names,
                ));
            let mut orchestrator = tool_orchestrator::ToolOrchestrator::new(llm_port, tool_port)
                .with_max_steps(agent_config.max_execution_steps);
            // D2.3/D2.10：on_chunk（WS typed 帧）存在时注册事件 sink——LLM 每步
            // 走流式，文本 delta → Chunk；工具生命周期 → ToolStart/ToolEnd
            if let Some(cb) = on_chunk {
                orchestrator = orchestrator.with_event_sink(Box::new(move |ev| {
                    if let Some(ev) = super::adapters::tool_bridge::map_tool_stream_event(ev) {
                        cb(ev);
                    }
                }));
            }
            let tool_ctx = tool_orchestrator::ToolRunContext {
                initial_prompt: prompt,
                session_id: input.session_id,
                user_id: Some(input.user_id),
                allowed_schemas: agent_config.allowed_schemas.clone(),
            };
            let result = orchestrator.run(&tool_ctx).await?;
            // D2.9：工具步执行期间可能已取消（run 内部步间由 M1 sink 事件承载，
            // Gateway 侧检查点在 run 返回后）——取消则不落库、直接放弃本轮
            if Self::cancelled(input.cancel.as_ref()) {
                return Err("CANCELLED".to_string());
            }
            // D2.4：ToolRunResult.usage → canonical JSON（无工具路径同构形态）
            turn_usage = super::adapters::tool_bridge::tool_result_usage(&result);
            // E7：工具调用记录（含截断后的原始输出）落 meta
            turn_tool_calls = super::adapters::tool_bridge::tool_calls_json(&result);
            result.final_text
        } else {
            // 无工具：LLM 真流式（P1-6）——on_chunk 存在时逐 chunk 回调并累积，
            // 否则整段生成（兼容）。
            match on_chunk {
                Some(cb) => {
                    // 流式（WS）文本路径：provider 仅回推文本 delta，无 usage——
                    // M1 的 StreamToolCallOutcome（含 usage）只存在于工具流式
                    // （generate_stream_with_tools）。故此路径 turn_usage 保持
                    // None → meta.usage 落 NULL（回显省略）；非流式整段与工具
                    // 路径分别经 generate_detailed / ToolRunResult.usage 持久化
                    // 真实用量（D2.4）。
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
                                cb(TurnStreamEvent::Chunk(text));
                            }
                            Err(e) => return Err(e.to_string()),
                        }
                        // D2.9：流式 chunk 间取消检查
                        if Self::cancelled(input.cancel.as_ref()) {
                            return Err("CANCELLED".to_string());
                        }
                    }
                    full
                }
                None => {
                    let (text, usage) = llm
                        .generate_detailed(
                            "",
                            &prompt,
                            None,
                            None,
                            None,
                            None,
                            model_override.as_deref(),
                        )
                        .await
                        .map_err(|e| e.to_string())?;
                    // 归一为 ai-agent TokenUsage 形态（prompt/completion/total），
                    // 与 M1 工具路径 ToolRunResult.usage 同构（D2.4）
                    turn_usage = usage.map(|u| {
                        serde_json::json!({
                            "prompt_tokens": u.input_tokens,
                            "completion_tokens": u.output_tokens,
                            "total_tokens": u.input_tokens + u.output_tokens,
                        })
                    });
                    text
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

        // 10. 统一后处理：C7/D2.3 服务端形态校验——extract_json_block 结果经
        // validate_structured 剔除幻觉 filled_fields/actions；全剔 → None（降级
        // 纯文本，structured 不落 meta/响应）
        let structured = extract_json_block(&content).and_then(validate_structured);
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

        // D2.9：落库前终检——取消则不写 assistant 消息（半途内容无价值，保持原子）
        if Self::cancelled(input.cancel.as_ref()) {
            return Err("CANCELLED".to_string());
        }

        // 12. 保存 assistant 消息 + 消息级 meta（agent_code/structured/usage/
        //     knowledge_refs 落 isahl_auth.chat_message_meta——isahl 消息表冻结，
        //     衍生存储 019 表；knowledge_refs 回显本轮用户消息注入的命中块 D2.15）
        let row = self
            .message_store
            .add_message(
                input.session_id,
                &agent_result.content,
                Some(subject_contact_id),
                &assistant_recipients,
            )
            .await?;
        if let Err(e) = self
            .message_store
            .save_message_meta(
                row.id,
                input.session_id,
                MessageMetaFields {
                    agent_code: &agent_code,
                    structured: agent_result.structured.as_ref(),
                    usage: turn_usage.as_ref(),
                    knowledge_refs: turn_knowledge_refs.as_ref(),
                    tool_calls: turn_tool_calls.as_ref(),
                    ..Default::default()
                },
            )
            .await
        {
            // meta 写失败不阻断主链（消息正文已落库；仅衍生数据缺失）
            common::telemetry::warn!(
                "save assistant message meta failed (session {}): {}",
                input.session_id,
                e
            );
        }

        // 13. 更新会话时间戳
        let _ = self
            .session_store
            .update_session_timestamp(input.session_id, input.user_id)
            .await;
        let resp = ChatMessageResponse {
            id: row.id,
            role: self.derive_role(row.fk_sender_addr, &agent_contacts),
            content: row.content.unwrap_or_default(),
            created_at: row.created_at,
            agent_code: agent_code.clone(),
            structured: agent_result.structured,
            requires_input: agent_result.requires_input,
            suggested_actions: agent_result.suggested_actions,
            usage: turn_usage,
            knowledge_refs: turn_knowledge_refs,
        };

        // 13.5 记忆沉淀（refactor-chat-ai-subject-identity-memory）：turn_count++；
        //     每 5 轮 spawn flash 提取「归属 + 内容」，按归属分流写入 L1 主体层与
        //     L2 对话方层（均全量替换）。禁入类事实由代码常量兜底强制落 L2。
        //     失败标记 memory_state 节流重试（D2.7）——失败仅 warn 不影响主链。
        let turn_count = agent_state
            .as_ref()
            .and_then(|s| s.get("turn_count"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0)
            + 1;
        let _ = self
            .session_store
            .update_session_state(
                input.session_id,
                input.user_id,
                serde_json::json!({ "turn_count": turn_count }),
            )
            .await;
        // R5：周期轮触发；上次失败 → 非周期轮也强制重试；均受 60s 节流约束
        let memory_due = memory_consolidation_due(
            agent_state.as_ref().and_then(|s| s.get("memory_state")),
            turn_count,
            chrono::Utc::now().timestamp(),
        );
        if memory_due {
            let user_id = input.user_id;
            let session_id = input.session_id;
            let attempt_at = chrono::Utc::now().timestamp();
            let llm_config = self.llm_config.clone();
            let session_store = self.session_store.clone();
            let memory_store = super::memory_store::ChatMemoryStore::new(self.pool.clone());
            let old_subject = memory_layers.subject.clone();
            let old_counterpart = memory_layers
                .counterpart
                .clone()
                .unwrap_or_else(|| serde_json::json!({}));
            // 最近对话（历史尾 4 轮 + 本轮问答）
            let mut recent_lines: Vec<String> = history
                .iter()
                .rev()
                .take(4)
                .map(|(role, c)| format!("{}: {}", role, c))
                .collect();
            recent_lines.reverse();
            recent_lines.push(format!("user: {}", user_content));
            recent_lines.push(format!("assistant: {}", resp.content));
            tokio::spawn(async move {
                // R5：attempt 起点标记 pending（节流锚点 = 尝试时刻）
                let _ = session_store
                    .update_session_state(
                        session_id,
                        user_id,
                        bg_task_state_patch("memory_state", "pending", attempt_at),
                    )
                    .await;
                let svc = match llm_config.load_service().await {
                    Ok(s) => s,
                    Err(e) => {
                        common::telemetry::warn!("memory: llm load failed: {}", e);
                        let _ = session_store
                            .update_session_state(
                                session_id,
                                user_id,
                                bg_task_state_patch("memory_state", "failed", attempt_at),
                            )
                            .await;
                        return;
                    }
                };
                let flash_model = svc.config().flash_model.clone();
                let prompt = format!(
                    "你是双层记忆管理员。主体记忆（跨对话方共享）：{}\n\n\
                     对话方记忆（仅本对话方）：{}\n\n最近对话：\n{}\n\n\
                     任务：提取/更新稳定的偏好与事实（增量，不臆造），输出 JSON：\
                     {{\"subject\": {{...合并后的完整主体记忆...}}, \
                     \"counterpart\": {{...合并后的完整对话方记忆...}}}}。\
                     约束：个人事实（金额/联系方式/身份标识/客户个人信息）MUST 只进 counterpart，\
                     MUST NOT 写进 subject；无更新的层保持原值；不要输出其它文字。",
                    serde_json::to_string(&old_subject).unwrap_or_default(),
                    serde_json::to_string(&old_counterpart).unwrap_or_default(),
                    recent_lines.join("\n")
                );
                let text = match svc
                    .generate_detailed("", &prompt, None, None, None, None, Some(&flash_model))
                    .await
                {
                    Ok((t, _)) => t,
                    Err(e) => {
                        common::telemetry::warn!("memory: generate failed: {}", e);
                        let _ = session_store
                            .update_session_state(
                                session_id,
                                user_id,
                                bg_task_state_patch("memory_state", "failed", attempt_at),
                            )
                            .await;
                        return;
                    }
                };
                let Some(raw) = ai_agent::agents::extract_json_block(&text) else {
                    common::telemetry::warn!("memory: 输出非 JSON，放弃本轮沉淀");
                    let _ = session_store
                        .update_session_state(
                            session_id,
                            user_id,
                            bg_task_state_patch("memory_state", "failed", attempt_at),
                        )
                        .await;
                    return;
                };
                if !raw.is_object() {
                    common::telemetry::warn!("memory: 输出非对象，放弃本轮沉淀");
                    let _ = session_store
                        .update_session_state(
                            session_id,
                            user_id,
                            bg_task_state_patch("memory_state", "failed", attempt_at),
                        )
                        .await;
                    return;
                }
                // 归属解析：双层对象；缺键时容忍为「整对象即主体层」（旧形态兼容）
                let (mut subject_mem, mut counterpart_mem) =
                    split_memory_layers(raw.clone(), &old_subject, &old_counterpart);
                // 规则兜底：禁入类事实强制落 L2（模型误判在此被改写）
                enforce_l1_deny_rules(&mut subject_mem, &mut counterpart_mem);
                if let Err(e) = memory_store.save_subject(subject_id, subject_mem).await {
                    common::telemetry::warn!("memory: subject save failed: {}", e);
                    let _ = session_store
                        .update_session_state(
                            session_id,
                            user_id,
                            bg_task_state_patch("memory_state", "failed", attempt_at),
                        )
                        .await;
                    return;
                }
                if let Some(cid) = counterpart_id {
                    if let Err(e) = memory_store
                        .save_counterpart(subject_id, cid, counterpart_mem)
                        .await
                    {
                        common::telemetry::warn!("memory: counterpart save failed: {}", e);
                    }
                }
                // R5：成功标记 ok（沉淀已落库）
                let _ = session_store
                    .update_session_state(
                        session_id,
                        user_id,
                        bg_task_state_patch("memory_state", "ok", attempt_at),
                    )
                    .await;
            });
        }

        // 13.6 D2.11/R5 滚动摘要：超窗溢出条（最老 1 条）+ 旧 summary → 异步 flash
        //     增量压缩存回 agent_state.conversation_summary；失败标记 summary_state
        //     （pending/ok/failed + last_attempt_at，60s 节流重试——不得永久静默陈旧）
        let summary_due = bg_retry_allowed(
            agent_state.as_ref().and_then(|s| s.get("summary_state")),
            chrono::Utc::now().timestamp(),
        );
        if summary_due {
            if let Some(spill_row) = overflow {
                if let Some(spill_content) = spill_row.content {
                    let session_id = input.session_id;
                    let user_id = input.user_id;
                    let attempt_at = chrono::Utc::now().timestamp();
                    let llm_config = self.llm_config.clone();
                    let session_store = self.session_store.clone();
                    let prev_summary = old_summary.clone().unwrap_or_default();
                    let spill_text = spill_content;
                    tokio::spawn(async move {
                        // R5：attempt 起点标记 pending（节流锚点 = 尝试时刻）
                        let _ = session_store
                            .update_session_state(
                                session_id,
                                user_id,
                                bg_task_state_patch("summary_state", "pending", attempt_at),
                            )
                            .await;
                        let svc = match llm_config.load_service().await {
                            Ok(s) => s,
                            Err(e) => {
                                common::telemetry::warn!("summary: llm load failed: {}", e);
                                let _ = session_store
                                    .update_session_state(
                                        session_id,
                                        user_id,
                                        bg_task_state_patch("summary_state", "failed", attempt_at),
                                    )
                                    .await;
                                return;
                            }
                        };
                        let flash_model = svc.config().flash_model.clone();
                        let prompt = format!(
                            "你是对话摘要器。已有摘要：{}\n\n新增待压缩的旧对话：\n{}\n\n\
                             任务：把新增内容合并进摘要（保留关键事实/决策/未竟事项，丢弃寒暄），\
                             输出更新后的摘要正文（≤800 字），只输出摘要本身。",
                            prev_summary, spill_text
                        );
                        let text = match svc
                            .generate_detailed(
                                "",
                                &prompt,
                                None,
                                None,
                                None,
                                None,
                                Some(&flash_model),
                            )
                            .await
                        {
                            Ok((t, _)) => t,
                            Err(e) => {
                                common::telemetry::warn!("summary: generate failed: {}", e);
                                let _ = session_store
                                    .update_session_state(
                                        session_id,
                                        user_id,
                                        bg_task_state_patch("summary_state", "failed", attempt_at),
                                    )
                                    .await;
                                return;
                            }
                        };
                        let new_summary: String = text.trim().chars().take(800).collect();
                        if new_summary.is_empty() {
                            // 空摘要 = 失败：留标记供下轮节流重试
                            let _ = session_store
                                .update_session_state(
                                    session_id,
                                    user_id,
                                    bg_task_state_patch("summary_state", "failed", attempt_at),
                                )
                                .await;
                            return;
                        }
                        // 成功：summary_state ok 与 conversation_summary 同 patch 落库
                        let mut ok_patch = bg_task_state_patch("summary_state", "ok", attempt_at);
                        ok_patch["conversation_summary"] = Value::String(new_summary);
                        if let Err(e) = session_store
                            .update_session_state(session_id, user_id, ok_patch)
                            .await
                        {
                            common::telemetry::warn!(
                                "summary: save failed (session {}): {}",
                                session_id,
                                e
                            );
                        }
                    });
                }
            }
        }

        // 13.7 C4/D2.1 draft-context 持续精选：每 4 轮（或上次失败强制重试，
        //     60s 节流）spawn flash 精炼——旧 draft.content + 最近对话 →
        //     重写式更新 agent_state.draft_context（{content,updated_at,
        //     source_turn_count}；改写非 append）；失败标记 draft_context_state
        //     供下轮节流重试。draft 仅会话级上下文——跨会话偏好仍归 memory
        //     通道沉淀（同轮双跑幂等：抽取范围独立）。
        let draft_due = draft_context_due(
            agent_state
                .as_ref()
                .and_then(|s| s.get("draft_context_state")),
            turn_count,
            chrono::Utc::now().timestamp(),
        );
        if draft_due {
            let session_id = input.session_id;
            let user_id = input.user_id;
            let attempt_at = chrono::Utc::now().timestamp();
            let source_turn_count = turn_count;
            let llm_config = self.llm_config.clone();
            let session_store = self.session_store.clone();
            let old_draft = agent_state
                .as_ref()
                .and_then(|s| s.get("draft_context"))
                .and_then(|d| d.get("content"))
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            // 最近对话：历史尾 4 行（含本轮用户消息）+ 本轮回复
            let mut recent_lines: Vec<String> = history
                .iter()
                .rev()
                .take(4)
                .map(|(role, c)| format!("{}: {}", role, c))
                .collect();
            recent_lines.reverse();
            recent_lines.push(format!("assistant: {}", resp.content));
            let recent_text = recent_lines.join("\n");
            tokio::spawn(async move {
                let _ = session_store
                    .update_session_state(
                        session_id,
                        user_id,
                        bg_task_state_patch("draft_context_state", "pending", attempt_at),
                    )
                    .await;
                let svc = match llm_config.load_service().await {
                    Ok(s) => s,
                    Err(e) => {
                        common::telemetry::warn!("draft: llm load failed: {}", e);
                        let _ = session_store
                            .update_session_state(
                                session_id,
                                user_id,
                                bg_task_state_patch("draft_context_state", "failed", attempt_at),
                            )
                            .await;
                        return;
                    }
                };
                let flash_model = svc.config().flash_model.clone();
                let prompt = draft_refine_prompt(&old_draft, &recent_text);
                let text = match svc
                    .generate_detailed("", &prompt, None, None, None, None, Some(&flash_model))
                    .await
                {
                    Ok((t, _)) => t,
                    Err(e) => {
                        common::telemetry::warn!("draft: generate failed: {}", e);
                        let _ = session_store
                            .update_session_state(
                                session_id,
                                user_id,
                                bg_task_state_patch("draft_context_state", "failed", attempt_at),
                            )
                            .await;
                        return;
                    }
                };
                let new_draft: String = text.trim().chars().take(1500).collect();
                if new_draft.is_empty() {
                    // 空草稿 = 失败：留标记供下轮节流重试
                    let _ = session_store
                        .update_session_state(
                            session_id,
                            user_id,
                            bg_task_state_patch("draft_context_state", "failed", attempt_at),
                        )
                        .await;
                    return;
                }
                // 成功：重写式更新（draft_context 整体替换，非 append）
                let ok_patch =
                    draft_context_success_patch(&new_draft, source_turn_count, attempt_at);
                if let Err(e) = session_store
                    .update_session_state(session_id, user_id, ok_patch)
                    .await
                {
                    common::telemetry::warn!("draft: save failed (session {}): {}", session_id, e);
                }
            });
        }

        // 14. D2.8 标题自动生成：默认标题会话在本轮成功后异步 flash 生成
        //     （≤16 字，取首条用户消息 + 首回复为输入）；失败仅 telemetry warn。
        if needs_auto_title {
            let session_id = input.session_id;
            let user_id = input.user_id;
            let llm_config = self.llm_config.clone();
            let session_store = self.session_store.clone();
            let user_text = user_content;
            let reply_text = resp.content.clone();
            tokio::spawn(async move {
                let svc = match llm_config.load_service().await {
                    Ok(s) => s,
                    Err(e) => {
                        common::telemetry::warn!("auto-title: llm load failed: {}", e);
                        return;
                    }
                };
                let flash_model = svc.config().flash_model.clone();
                let prompt = format!(
                    "根据以下对话概括主题，不超过16个字，仅输出标题本身，不要引号。\n用户：{}\n助手：{}",
                    user_text, reply_text
                );
                match svc
                    .generate_detailed("", &prompt, None, None, None, None, Some(&flash_model))
                    .await
                {
                    Ok((text, _)) => {
                        let title: String = text
                            .trim()
                            .trim_matches(['"', '\'', '「', '」', '《', '》'])
                            .chars()
                            .take(16)
                            .collect();
                        if title.is_empty() {
                            return;
                        }
                        if let Err(e) = session_store
                            .update_session_title(session_id, user_id, &title)
                            .await
                        {
                            common::telemetry::warn!(
                                "auto-title: update session {} title failed: {}",
                                session_id,
                                e
                            );
                        }
                    }
                    Err(e) => {
                        common::telemetry::warn!("auto-title: generate failed: {}", e);
                    }
                }
            });
        }

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
        attachments: Option<Value>,
        knowledge_refs: Option<Value>,
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
        // D-2：用户消息收件人 = 会话当前智能体联系方式（无则空集，不阻断写入）
        let recipients: Vec<i64> =
            super::memory_scope::session_agent_contact_id(&self.pool, session_id)
                .await
                .unwrap_or(None)
                .into_iter()
                .collect();

        let row = self
            .message_store
            .add_message(session_id, content, sender_addr, &recipients)
            .await?;

        // 附件/知识引用随用户消息落 meta（D2.6/D2.15；正文不污染）。
        // 失败仅 warn——消息已落库，不应让附件元数据问题阻断消息写入。
        if attachments.is_some() || knowledge_refs.is_some() {
            if let Err(e) = self
                .message_store
                .save_message_meta(
                    row.id,
                    session_id,
                    MessageMetaFields {
                        attachments: attachments.as_ref(),
                        knowledge_refs: knowledge_refs.as_ref(),
                        ..Default::default()
                    },
                )
                .await
            {
                common::telemetry::warn!(
                    "save user message meta failed (session {}): {}",
                    session_id,
                    e
                );
            }
        }

        let _ = self
            .session_store
            .update_session_timestamp(session_id, user_id)
            .await;

        Ok(ChatMessageResponse {
            id: row.id,
            role: self.derive_role(row.fk_sender_addr, &std::collections::HashSet::new()),
            content: row.content.unwrap_or_default(),
            created_at: row.created_at,
            // leader-agent 绑定持久化已随 zc_id_threads_rr_entity 删除，普通消息无绑定 agent
            agent_code: String::new(),
            structured: None,
            requires_input: false,
            suggested_actions: vec![],
            usage: None,
            knowledge_refs: None,
        })
    }

    async fn switch_agent(
        &self,
        session_id: i64,
        agent_code: &str,
        user_id: i64,
    ) -> Result<(), String> {
        let session = self.session_store.get_session(session_id, user_id).await?;
        if session.is_none() {
            return Err("SESSION_NOT_FOUND".to_string());
        }

        // D2.7 pinned_agent：agent_code="auto" → 清 pin（回自动路由，null 合并
        // 语义——resolve_agent 以 ->>'pinned_agent' as_str 判 None 等价无 pin）；
        // 否则校验 registry 后写 pin。resolve 入口优先 pin，跳过 LLM 路由。
        let state = if agent_code == "auto" {
            serde_json::json!({
                "pinned_agent": null,
                "switched_at": Utc::now().to_rfc3339(),
            })
        } else {
            if !self.agent_dispatch.agent_exists(agent_code).await {
                return Err(format!("Agent '{}' not found", agent_code));
            }
            serde_json::json!({
                "pinned_agent": agent_code,
                "switched_at": Utc::now().to_rfc3339(),
            })
        };
        let _ = self
            .session_store
            .update_session_state(session_id, user_id, state)
            .await;

        Ok(())
    }

    async fn execute_action(
        &self,
        session_id: i64,
        action_id: &str,
        params: Option<Value>,
        confirmed: bool,
        user_id: i64,
    ) -> Result<ExecuteActionResponse, String> {
        let session = self.session_store.get_session(session_id, user_id).await?;
        if session.is_none() {
            return Err("SESSION_NOT_FOUND".to_string());
        }
        let params = params.unwrap_or(Value::Null);
        if !params.is_object() {
            return Err("INVALID_PARAMS: params must be an object".to_string());
        }

        // 动作上下文：action_type + target_ids 解析
        //  - "agent_action:<type>"：type 直接取，target_ids 必须来自 params.target_ids
        //  - 其余：查 agent_state.last_result.structured.actions[]（上一轮 agent
        //    建议的动作，id 匹配，kind="execute"）；target_ids 取 params.target_ids
        //    > 动作记录内 params.target_ids
        let (_, agent_state) = self
            .session_store
            .get_session_context(session_id, user_id)
            .await?;
        let (action_type, target_ids): (String, Vec<i64>) = if let Some(stripped) =
            action_id.strip_prefix("agent_action:")
        {
            let ids = params
                .get("target_ids")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| {
                            v.as_i64()
                                .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
                        })
                        .collect::<Vec<i64>>()
                })
                .unwrap_or_default();
            if ids.is_empty() {
                return Err("MISSING_TARGET_IDS: params.target_ids 必须提供动作目标".to_string());
            }
            (stripped.to_string(), ids)
        } else {
            let action = agent_state
                .as_ref()
                .and_then(|s| s.get("last_result"))
                .and_then(|lr| lr.get("structured"))
                .and_then(|st| st.get("actions"))
                .and_then(|a| a.as_array())
                .and_then(|actions| {
                    actions.iter().find(|a| {
                        a.get("id").and_then(|v| v.as_str()) == Some(action_id)
                            && a.get("kind").and_then(|v| v.as_str()) == Some("execute")
                    })
                })
                .ok_or_else(|| format!("ACTION_NOT_FOUND: {}", action_id))?;
            let action_type = action
                .get("action_type")
                .and_then(|v| v.as_str())
                .or_else(|| params.get("action_type").and_then(|v| v.as_str()))
                .ok_or("MISSING_ACTION_TYPE")?
                .to_string();
            let ids = params
                .get("target_ids")
                .and_then(|v| v.as_array())
                .or_else(|| {
                    action
                        .get("params")
                        .and_then(|p| p.get("target_ids"))
                        .and_then(|v| v.as_array())
                })
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| {
                            v.as_i64()
                                .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
                        })
                        .collect::<Vec<i64>>()
                })
                .unwrap_or_default();
            (action_type, ids)
        };

        // agent 上下文（allowed_schemas 域门禁用）：pin 优先；无 pin 时取最后
        // 一条 assistant 消息 meta 记录的 agent_code；兜底 general。
        let agent_code: String = {
            let pinned = agent_state
                .as_ref()
                .and_then(|s| s.get("pinned_agent"))
                .and_then(|v| v.as_str())
                .map(String::from);
            match pinned {
                Some(code) if !code.is_empty() => code,
                _ => {
                    // 最近 assistant 消息的 meta.agent_code
                    let from_meta = sqlx::query_scalar::<_, String>(
                        r#"SELECT cm.agent_code
                           FROM isahl."zc_id_msgs-chat_ai" m
                           JOIN isahl_auth.chat_message_meta cm ON cm.msg_id = m.id
                           WHERE m.fk_thread = $1 AND cm.agent_code <> ''
                             AND m.deleted_at IS NULL
                           ORDER BY m.created_at DESC LIMIT 1"#,
                    )
                    .bind(session_id)
                    .fetch_optional(&self.pool)
                    .await
                    .ok()
                    .flatten();
                    from_meta.unwrap_or_else(|| "general".to_string())
                }
            }
        };
        let agent_config = self.agent_dispatch.get_agent_config(&agent_code).await?;

        let handler = super::adapters::tool_bridge::GatewayActionHandler::new(self.pool.clone());
        // Explicit/High/Critical 级动作必须 confirmed=true（D2.1；与工具路径
        // needs_confirmation 门禁同口径）——拒绝即审计 Deny（动作已解析）
        let level = handler.confirmation_level(&action_type);
        if super::adapters::tool_bridge::requires_confirmation(&level) && !confirmed {
            let err = format!("CONFIRMATION_REQUIRED: 动作 {} 需要显式确认", action_type);
            self.audit_action_execution(
                session_id,
                action_id,
                user_id,
                &action_type,
                target_ids.len(),
                confirmed,
                &Err(err.clone()),
            )
            .await;
            return Err(err);
        }

        let ctx = ai_agent::tools::ToolContext {
            session_id,
            user_id: Some(user_id),
            db_pool: self.pool.clone(),
            allowed_schemas: agent_config.allowed_schemas.clone(),
            action_handler: None,
        };
        let outcome = handler
            .run_action(&action_type, &target_ids, &params, &ctx)
            .await;
        // C8/D2.4：真实执行审计——成功 Permit / 失败 Deny（await + 错误仅
        // telemetry，不阻断主链）
        self.audit_action_execution(
            session_id,
            action_id,
            user_id,
            &action_type,
            target_ids.len(),
            confirmed,
            &outcome,
        )
        .await;
        let result = outcome?;

        Ok(ExecuteActionResponse {
            success: true,
            action_id: action_id.to_string(),
            result: Some(result),
            message: format!("Action '{}' executed", action_type),
        })
    }

    async fn list_agents(&self, locale: &str) -> Result<Vec<AgentInfoResponse>, String> {
        let configs = self.agent_dispatch.list_agent_configs().await?;
        let mut agents: Vec<AgentInfoResponse> = configs
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
        // D2.7 自动路由入口（sort_order -1 恒居首）：code="auto" 是 switch-agent
        // 清除 pin 的哨兵；i18n 键缺失时按 locale 回退文案。
        let auto_name = self
            .i18n
            .read()
            .await
            .get(&i18n::Locale::new(locale), "chat.agent.autoRoute")
            .map(String::from)
            .unwrap_or_else(|| {
                if locale.to_lowercase().starts_with("zh") {
                    "智能路由".to_string()
                } else {
                    "Smart Routing".to_string()
                }
            });
        agents.insert(
            0,
            AgentInfoResponse {
                code: "auto".to_string(),
                name: auto_name,
                description: String::new(),
                capabilities: vec![],
                user_selectable: true,
                sort_order: -1,
                icon: "Sparkles".to_string(),
                color: "#6366f1".to_string(),
                category: "auto".to_string(),
            },
        );
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

    // ── 运行时上下文（批 ④ add-chat-ai-runtime-context）────────────────

    fn sample_identity<'a>(
        namespace: &'a str,
        app_code: &'a str,
        agent_code: &'a str,
    ) -> RuntimeIdentity<'a> {
        RuntimeIdentity {
            namespace,
            app_code,
            agent_code,
            subject_id: 341_543_906_011_526,
        }
    }

    #[test]
    fn test_subject_persona_section_injected_before_identity() {
        let persona = subject_persona_section(Some("你是严谨的运输调度主体：先核对单据再作答"));
        assert!(persona.contains("## 主体人格"));
        assert!(persona.contains("严谨的运输调度主体"));
        // 段序契约：语言指令 → 主体人格 → 运行身份
        let sys = format!(
            "{}{}{}",
            locale_instruction("zh-CN"),
            persona,
            runtime_identity_block(&sample_identity("WZ", "WZ", "general"))
        );
        let locale_at = sys.find("## 语言指令").expect("locale");
        let persona_at = sys.find("## 主体人格").expect("persona");
        let identity_at = sys.find("## 运行身份").expect("identity");
        assert!(
            locale_at < persona_at && persona_at < identity_at,
            "段序错误"
        );
    }

    #[test]
    fn test_subject_persona_absent_when_empty() {
        for persona in [None, Some(""), Some("   ")] {
            assert!(
                subject_persona_section(persona).is_empty(),
                "空/空白人格 MUST NOT 产段（persona={persona:?}）"
            );
        }
    }

    #[test]
    fn test_subject_persona_truncated_at_cap() {
        let long = "人".repeat(1600);
        let section = subject_persona_section(Some(&long));
        assert!(section.contains("…[truncated]"));
        let body = section.split("保持一致：\n").nth(1).unwrap_or_default();
        assert!(
            body.chars().count() <= PERSONA_CAP + 12,
            "人格段须按 PERSONA_CAP 截断（实际 {}）",
            body.chars().count()
        );
    }

    #[test]
    fn test_runtime_identity_block_carries_runtime_facts() {
        let block = runtime_identity_block(&sample_identity("WZ", "WZ", "general"));
        assert!(block.contains("## 运行身份"));
        assert!(block.contains("namespace: WZ"));
        assert!(block.contains("app: WZ"));
        assert!(block.contains("agent: general"));
        assert!(block.contains("341543906011526"));
        // 时刻不在 system 前言（秒级动态事实置 prompt 尾部）
        assert!(!block.contains("当前运行时刻"));
    }

    #[test]
    fn test_runtime_identity_truncation_is_char_safe() {
        // W2：多字节 namespace（>500 字节、跨越第 500 字节边界）MUST NOT panic，
        // 且按字符截断（byte 切片会在非字符边界 panic）。
        let wide = "宽".repeat(400); // 400 字符 / 1200 字节
        let block = runtime_identity_block(&sample_identity(&wide, &wide, "general"));
        assert!(block.contains("…[truncated]"));
        let identity = block
            .split("以此为准）：")
            .nth(1)
            .unwrap_or_default()
            .trim();
        assert!(
            identity.chars().count() <= IDENTITY_CAP + 12,
            "身份段须按字符截断（实际 {} 字符）",
            identity.chars().count()
        );
    }

    #[test]
    fn test_runtime_time_section_renders_fixed_label() {
        let section = runtime_time_section("2026-09-13 18:20:00 (UTC+08:00)", "zh-CN");
        assert!(section.contains("## 当前运行时刻"));
        assert!(section.contains("2026-09-13 18:20:00 (UTC+08:00)"));
        assert!(section.contains("zh-CN"));
    }

    #[test]
    fn test_prompt_layout_identity_in_system_head_time_at_tail() {
        // 段位契约：system 前言 = system_prompt + locale + 运行身份；时刻段置尾部。
        let sys = format!(
            "{}{}{}",
            "sys",
            locale_instruction("zh-CN"),
            runtime_identity_block(&sample_identity("Alioth", "Alioth", "general"))
        );
        assert!(sys.starts_with("sys"));
        let locale_at = sys.find("## 语言指令").expect("locale section");
        let identity_at = sys.find("## 运行身份").expect("identity section");
        assert!(locale_at < identity_at, "locale 段须在运行身份段之前");
        assert!(
            !sys.contains("## 当前运行时刻"),
            "时刻段 MUST NOT 进 system 前言"
        );

        let tail = format!(
            "\n\n## 用户最新消息\n{}\n{}",
            "今天几号",
            runtime_time_section("2026-09-13 18:20:00 (UTC+08:00)", "zh-CN")
        );
        let user_at = tail.find("## 用户最新消息").expect("user section");
        let time_at = tail.find("## 当前运行时刻").expect("time section");
        assert!(user_at < time_at, "时刻段须在 prompt 尾部（用户消息之后）");
    }

    #[test]
    fn test_format_tool_trace_renders_calls() {
        let calls = json!([{
            "name": "query_sql",
            "arguments": {"sql": "select count(*) from isahl.zc_id_contract"},
            "success": true,
            "output": "count=42"
        }]);
        let note = format_tool_trace(&calls).expect("注记");
        assert!(note.contains("query_sql"));
        assert!(note.contains("ok"));
        assert!(note.contains("count=42"));
        assert!(note.contains("本轮工具执行"));
    }

    #[test]
    fn test_format_tool_trace_truncates_output_at_300() {
        let calls = json!([{
            "name": "query_sql",
            "arguments": {},
            "success": false,
            "output": "x".repeat(400)
        }]);
        let note = format_tool_trace(&calls).expect("注记");
        assert!(note.contains("fail"));
        assert!(note.contains("…[截断]"));
        let output_part = note.split("fail: ").nth(1).unwrap_or_default();
        assert!(output_part.chars().count() <= TOOL_NOTE_OUTPUT_CAP + 10);
    }

    #[test]
    fn test_format_tool_trace_total_cap_marks_truncation() {
        let calls = json!([
            {"name": "a", "arguments": {}, "success": true, "output": "y".repeat(250)},
            {"name": "b", "arguments": {}, "success": true, "output": "y".repeat(250)},
            {"name": "c", "arguments": {}, "success": true, "output": "y".repeat(250)}
        ]);
        let note = format_tool_trace(&calls).expect("注记");
        assert!(note.contains("…[工具记录截断]"));
        assert!(
            note.chars().count() <= TOOL_NOTE_TOTAL_CAP,
            "注记物理总长（含包装/标记）MUST ≤ {TOOL_NOTE_TOTAL_CAP}，实际 {}",
            note.chars().count()
        );
    }

    #[test]
    fn test_format_tool_trace_empty_is_none() {
        assert!(format_tool_trace(&json!([])).is_none());
        assert!(format_tool_trace(&json!("not-an-array")).is_none());
        assert!(format_tool_trace(&json!(null)).is_none());
    }

    #[test]
    fn test_history_line_tool_note_only_for_assistant() {
        let calls = json!([{
            "name": "query_sql",
            "arguments": {},
            "success": true,
            "output": "raw-rows-42"
        }]);
        let assistant_line = history_line("assistant", "见上表", Some(&calls));
        assert!(assistant_line.starts_with("见上表"));
        assert!(assistant_line.contains("raw-rows-42"));

        assert_eq!(
            history_line("user", "那批数据呢", Some(&calls)),
            "那批数据呢"
        );
        assert_eq!(history_line("assistant", "无工具答复", None), "无工具答复");
    }

    // ── 运行时上下文测试结束 ────────────────────────────────

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
        let prompt = assemble_context_sections("s", pc(big).as_ref(), None, None, None, None, None);
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
        let prompt = assemble_context_sections("s", None, None, None, None, None, None);
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
            None,
            None,
        );
        assert!(prompt.contains("## 用户权限范围"));
        assert!(prompt.contains("approval.read"));
    }

    #[test]
    fn test_memory_two_sections_and_empty_not_injected() {
        // 两层皆空 → 不注入记忆段
        let prompt = assemble_context_sections(
            "s",
            None,
            None,
            pc(json!({})).as_ref(),
            pc(json!({})).as_ref(),
            None,
            None,
        );
        assert!(!prompt.contains("## 主体记忆"));
        assert!(!prompt.contains("## 本对话方记忆"));

        // 双层并存；主体层 1000 截断
        let big_subject = json!({"style": "x".repeat(1500)});
        let prompt = assemble_context_sections(
            "s",
            None,
            None,
            pc(big_subject).as_ref(),
            pc(json!({"pref": "本对话方偏好"})).as_ref(),
            None,
            None,
        );
        assert!(prompt.contains("## 主体记忆"));
        assert!(prompt.contains("## 本对话方记忆"));
        assert!(prompt.contains("本对话方偏好"));
        let subject_body = prompt
            .split("## 主体记忆")
            .nth(1)
            .unwrap_or_default()
            .split("## 本对话方记忆")
            .next()
            .unwrap_or_default();
        assert!(subject_body.contains("…[truncated]"), "主体层须 1000 截断");
        assert!(subject_body.chars().count() < 1100);
    }

    #[test]
    fn test_l1_deny_rules_force_counterpart() {
        let mut subject = json!({
            "风控审批习惯": "大额走二级审批",
            "客户联系人电话": "13800000000",
            "结算金额": 12000
        });
        let mut counterpart = json!({});
        let moved = enforce_l1_deny_rules(&mut subject, &mut counterpart);
        assert_eq!(moved, 2, "禁入类两条应被移出主体层");
        assert!(subject.get("风控审批习惯").is_some());
        assert!(subject.get("客户联系人电话").is_none());
        assert!(subject.get("结算金额").is_none());
        assert_eq!(counterpart.get("客户联系人电话").unwrap(), "13800000000");
        assert_eq!(counterpart.get("结算金额").unwrap(), 12000);
    }

    #[test]
    fn test_l1_deny_rules_keep_existing_counterpart_value() {
        let mut subject = json!({"客户": "张三"});
        let mut counterpart = json!({"客户": "李四"});
        let moved = enforce_l1_deny_rules(&mut subject, &mut counterpart);
        assert_eq!(moved, 1);
        assert_eq!(counterpart.get("客户").unwrap(), "李四");
    }

    #[test]
    fn test_derive_allowed_tool_names_fail_closed() {
        use ai_agent::agents::{ExecutionTarget, ToolDefinition};
        let backend = ToolDefinition {
            name: "query_sql".to_string(),
            description: String::new(),
            parameters: json!({}),
            execution_target: ExecutionTarget::Backend,
        };
        let frontend = ToolDefinition {
            name: "client_only".to_string(),
            description: String::new(),
            parameters: json!({}),
            execution_target: ExecutionTarget::Frontend,
        };
        // 声明面 = 派生面（仅 Backend 入选）
        assert_eq!(
            derive_allowed_tool_names(std::slice::from_ref(&backend)),
            vec!["query_sql"]
        );
        // 无声明 ⇒ 空（调用方走无工具路径）
        assert!(derive_allowed_tool_names(&[]).is_empty());
        // 全为非 Backend 目标 ⇒ 空 ⇒ 调用方 MUST 视为无工具（fail-closed，不落入「空集=不过滤」）
        assert!(derive_allowed_tool_names(&[frontend]).is_empty());
    }

    #[test]
    fn test_require_subject_id_fails_explicitly_without_subject_row() {
        // 无主体行 ⇒ 显式失败（MUST NOT 回落按人类账号写记忆）
        let mut config = ai_agent::AgentRegistry::new()
            .merged_config("general")
            .expect("内置 general");
        config.subject_id = None;
        let error = require_subject_id(&config, "general").expect_err("无主体行必须显式失败");
        assert!(
            error.contains("AGENT_SUBJECT_MISSING") && error.contains("general"),
            "失败信息须可定位 agent: {error}"
        );

        // 有主体行 ⇒ 原值透传（主体 id 为 L1 记忆键）
        config.subject_id = Some(341543906011526);
        assert_eq!(
            require_subject_id(&config, "general").expect("有主体行"),
            341543906011526
        );
    }

    #[test]
    fn test_permission_section_fail_closed_when_resolution_missing() {
        // 权限解析失败/缺失 ⇒ None 传入 ⇒ 权限段 MUST 不出现（fail-closed：
        // 不回落陈旧快照；数据访问仍由 PEP 每请求裁决）
        let without = assemble_context_sections("sys", None, None, None, None, None, None);
        assert!(!without.contains("## 用户权限范围"), "缺权限段原文");

        let permissions = json!({"userId": 1, "userAttributes": ["view:cust"]});
        let with =
            assemble_context_sections("sys", None, Some(&permissions), None, None, None, None);
        assert!(with.contains("## 用户权限范围"), "有权限时 MUST 注入权限段");
        assert!(with.contains("view:cust"));
    }

    #[test]
    fn test_split_memory_layers_tolerates_legacy_shape() {
        // 旧形态（单层对象）→ 整对象视为主体层，保留旧对话方层
        let old_cp = json!({"pref": "旧对话方偏好"});
        let (subject, counterpart) =
            split_memory_layers(json!({"style": "简洁"}), &json!({}), &old_cp);
        assert_eq!(subject.get("style").unwrap(), "简洁");
        assert_eq!(counterpart, old_cp);

        // 双层形态缺 subject 键 → 保留旧主体层，避免误清空
        let old_subj = json!({"style": "旧"});
        let (subject, counterpart) = split_memory_layers(
            json!({"counterpart": {"pref": "新"}}),
            &old_subj,
            &json!({}),
        );
        assert_eq!(subject, old_subj);
        assert_eq!(counterpart.get("pref").unwrap(), "新");
    }

    #[test]
    fn test_summary_section_injected() {
        let prompt = assemble_context_sections(
            "s",
            None,
            None,
            None,
            None,
            None,
            Some("早前：用户确认了运费分摊规则"),
        );
        assert!(prompt.contains("## 早前对话摘要"));
        assert!(prompt.contains("运费分摊规则"));
    }

    #[test]
    fn test_summary_empty_not_injected() {
        let prompt = assemble_context_sections("s", None, None, None, None, None, None);
        assert!(!prompt.contains("## 早前对话摘要"));
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

    // ── S2/R1 附件收紧：仅 data_base64、单图 ≤4MiB（纯函数校验）────────────

    fn b64(bytes: &[u8]) -> String {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    #[test]
    fn test_attachment_url_only_rejected_no_fetch() {
        // S2：url-only 附件（无 data_base64）→ 无字节可解 → None；无出站拉取分支。
        let img = json!({ "type": "image", "url": "http://169.254.169.254/latest/meta-data/" });
        assert!(valid_image_bytes(&img).is_none());
    }

    #[test]
    fn test_attachment_missing_fields_rejected() {
        // 非图片 / 空对象 / type 缺失 → 不进入校验（analyze 侧过滤），空附件列表语义
        let plain = json!({ "type": "image" });
        assert!(valid_image_bytes(&plain).is_none());
    }

    #[test]
    fn test_attachment_bad_base64_rejected() {
        let img = json!({ "type": "image", "data_base64": "not-valid-b64!!!" });
        assert!(valid_image_bytes(&img).is_none());
    }

    #[test]
    fn test_attachment_at_cap_accepted_exactly_4mib() {
        // 边界：恰 4MiB 允许
        let bytes = vec![0u8; 4 * 1024 * 1024];
        let img = json!({ "type": "image", "data_base64": b64(&bytes) });
        let got = valid_image_bytes(&img).expect("4MiB 恰在限额内");
        assert_eq!(got.0, "image/png", "mime 缺省 png");
        assert_eq!(got.1.len(), 4 * 1024 * 1024);
    }

    #[test]
    fn test_attachment_over_cap_rejected() {
        // R1：4MiB+1 → None（跳过 + warn）
        let bytes = vec![0u8; 4 * 1024 * 1024 + 1];
        let img = json!({ "type": "image", "data_base64": b64(&bytes) });
        assert!(valid_image_bytes(&img).is_none());
    }

    #[test]
    fn test_attachment_data_base64_accepted_with_mime() {
        let img = json!({
            "type": "image",
            "mime": "image/jpeg",
            "data_base64": b64(b"\xff\xd8\xff\xe0tiny-jpeg")
        });
        let got = valid_image_bytes(&img).expect("合法 data_base64 接受");
        assert_eq!(got.0, "image/jpeg");
        assert_eq!(got.1, b"\xff\xd8\xff\xe0tiny-jpeg");
    }

    // ── R5 后台任务失败标记 + 60s 节流（纯函数门禁）────────────────

    #[test]
    fn test_bg_retry_allowed_no_state_or_ok() {
        assert!(bg_retry_allowed(None, 1000), "无记录 → 首次尝试放行");
        let ok = json!({ "status": "ok", "last_attempt_at": 900 });
        assert!(bg_retry_allowed(Some(&ok), 1000), "ok → 新触发放行");
    }

    #[test]
    fn test_bg_retry_allowed_throttle_window() {
        let failed = json!({ "status": "failed", "last_attempt_at": 1000 });
        assert!(!bg_retry_allowed(Some(&failed), 1059), "59s 内节流跳过");
        assert!(bg_retry_allowed(Some(&failed), 1060), "恰 60s → 放行重试");
        assert!(bg_retry_allowed(Some(&failed), 2000), "远超 60s → 放行");
    }

    #[test]
    fn test_bg_retry_allowed_stale_pending_released() {
        let pending = json!({ "status": "pending", "last_attempt_at": 1000 });
        assert!(
            !bg_retry_allowed(Some(&pending), 1001),
            "在飞 pending 防重复"
        );
        assert!(
            bg_retry_allowed(Some(&pending), 1060),
            "崩溃遗留 pending 超窗放行"
        );
    }

    #[test]
    fn test_bg_task_state_patch_shape() {
        let patch = bg_task_state_patch("memory_state", "failed", 42);
        assert_eq!(patch["memory_state"]["status"], "failed");
        assert_eq!(patch["memory_state"]["last_attempt_at"], 42);
        let patch2 = bg_task_state_patch("summary_state", "ok", 7);
        assert_eq!(patch2["summary_state"]["status"], "ok");
    }

    #[test]
    fn test_memory_consolidation_due_periodic_and_forced() {
        let ok = json!({ "status": "ok", "last_attempt_at": 0 });
        let failed_recent = json!({ "status": "failed", "last_attempt_at": 1000 });
        let failed_stale = json!({ "status": "failed", "last_attempt_at": 500 });
        assert!(
            !memory_consolidation_due(Some(&ok), 3, 1100),
            "非周期无失败 → 不做"
        );
        assert!(memory_consolidation_due(Some(&ok), 5, 1100), "周期轮 → 做");
        assert!(
            memory_consolidation_due(Some(&failed_stale), 6, 1100),
            "上次失败 ≥60s → 非周期轮强制重试"
        );
        assert!(
            !memory_consolidation_due(Some(&failed_recent), 10, 1030),
            "周期轮但 <60s → 节流跳过"
        );
    }

    // ── R6 locale 语言指令（纯函数）──────────────────────────

    #[test]
    fn test_locale_native_name_mapping() {
        assert_eq!(locale_native_name("zh-CN"), "中文");
        assert_eq!(locale_native_name("zh"), "中文");
        assert_eq!(locale_native_name("en"), "English");
        assert_eq!(locale_native_name("en-US"), "English");
        assert_eq!(
            locale_native_name("fr-FR"),
            "English",
            "未知 locale → 默认 English"
        );
    }

    #[test]
    fn test_locale_instruction_section() {
        let zh = locale_instruction("zh-CN");
        assert!(zh.contains("## 语言指令"), "指令独立成段");
        assert!(zh.contains("用户界面语言: 中文"));
        assert!(zh.contains("请始终使用该语言回复"));
        let en = locale_instruction("en-US");
        assert!(en.contains("## 语言指令"));
        assert!(en.contains("用户界面语言: English"));
        assert!(en.contains("请始终使用该语言回复"));
    }

    // ── C4 draft-context（fix-chat-ai-capability-gaps D2.1）──────────

    #[test]
    fn test_draft_context_due_periodic_and_forced() {
        let ok = json!({ "status": "ok", "last_attempt_at": 0 });
        let failed_recent = json!({ "status": "failed", "last_attempt_at": 1000 });
        let failed_stale = json!({ "status": "failed", "last_attempt_at": 500 });
        assert!(
            !draft_context_due(Some(&ok), 3, 1100),
            "非周期无失败 → 不做"
        );
        assert!(draft_context_due(Some(&ok), 4, 1100), "每 4 轮周期 → 做");
        assert!(draft_context_due(Some(&ok), 8, 1100), "8 轮同样周期 → 做");
        assert!(
            draft_context_due(Some(&failed_stale), 5, 1100),
            "上次失败 ≥60s → 非周期轮强制重试"
        );
        assert!(
            !draft_context_due(Some(&failed_recent), 8, 1030),
            "周期轮但 <60s → 节流跳过"
        );
    }

    #[test]
    fn test_draft_context_success_patch_rewrite_shape() {
        let patch = draft_context_success_patch("- 用户偏好：中文报表", 4, 1700);
        // 任务状态 ok 标记
        assert_eq!(patch["draft_context_state"]["status"], "ok");
        assert_eq!(patch["draft_context_state"]["last_attempt_at"], 1700);
        // 重写式更新形态：draft_context 整体对象替换（content/updated_at/source_turn_count）
        assert_eq!(patch["draft_context"]["content"], "- 用户偏好：中文报表");
        assert_eq!(patch["draft_context"]["updated_at"], 1700);
        assert_eq!(patch["draft_context"]["source_turn_count"], 4);
    }

    #[test]
    fn test_draft_refine_prompt_assembly() {
        // D2.1 精炼指令：保留仍相关/移除过时/合并新信息/≤1500 字要点式/无新增原样输出
        let p = draft_refine_prompt("- 旧偏好：A", "user: 你好\nassistant: 你好！");
        assert!(p.starts_with("你是会话草稿上下文管理员。当前草稿："));
        assert!(p.contains("- 旧偏好：A"), "旧草稿必须嵌入");
        assert!(
            p.contains("user: 你好\nassistant: 你好！"),
            "最近对话必须嵌入"
        );
        assert!(p.contains("保留仍然相关的偏好/决策/关键实体/进行中事项"));
        assert!(p.contains("移除已过时项，合并新信息"));
        assert!(p.contains("≤1500 字"));
        assert!(p.contains("markdown 要点式"));
        assert!(p.contains("无新增则原样输出"));
        // 空草稿 → （无）占位
        let p2 = draft_refine_prompt("", "user: hi");
        assert!(p2.contains("当前草稿：（无）"), "空草稿占位: {}", p2);
        let p3 = draft_refine_prompt("   ", "user: hi");
        assert!(p3.contains("当前草稿：（无）"), "纯空白草稿占位: {}", p3);
    }

    #[test]
    fn test_draft_section_injected_with_cap() {
        let state = json!({
            "draft_context": {
                "content": "- 进行中：运费分摊规则确认\n- 偏好：中文回复",
                "updated_at": 1700,
                "source_turn_count": 4
            }
        });
        let prompt =
            assemble_context_sections("s", None, None, None, None, pc(state).as_ref(), None);
        assert!(prompt.contains("## 会话草稿上下文"), "草稿段必须注入");
        assert!(prompt.contains("运费分摊规则确认"));
        // 超长草稿 → 3000 字符截断
        let big = json!({
            "draft_context": { "content": "z".repeat(5000) }
        });
        let p2 = assemble_context_sections("s", None, None, None, None, pc(big).as_ref(), None);
        assert!(p2.contains("…[truncated]"), "草稿 3000 截断");
        let section = p2.split("## 会话草稿上下文").nth(1).unwrap_or_default();
        assert!(section.chars().count() < 3100, "草稿段 ≤3000+标记");
    }

    #[test]
    fn test_draft_empty_not_injected() {
        // 空 content / 缺 draft_context / content 纯空白 → 不注入段
        for state in [
            json!({}),
            json!({ "draft_context": null }),
            json!({ "draft_context": { "content": "" } }),
            json!({ "draft_context": { "content": "   " } }),
        ] {
            let prompt = assemble_context_sections(
                "s",
                None,
                None,
                None,
                None,
                pc(state.clone()).as_ref(),
                None,
            );
            assert!(
                !prompt.contains("## 会话草稿上下文"),
                "空草稿不得注入: {}",
                state
            );
        }
    }

    // ── C7 structured 服务端校验（fix-chat-ai-capability-gaps D2.3）──

    #[test]
    fn test_validate_structured_keeps_valid_shape() {
        let v = json!({
            "kind": "form_fill",
            "filled_fields": {
                "customer": { "value": "中远物流", "confidence": 0.95, "source": "对话" },
                "amount": 12.5,
                "note": "直接标量"
            },
            "actions": [
                { "id": "a1", "label": "发送", "kind": "message" },
                { "id": "a2", "label": "批量审批", "kind": "execute", "action_type": "batch_approve" },
                { "id": "a3", "label": "去审批页", "kind": "page_action", "op": "navigate", "args": { "path": "/approvals" } }
            ]
        });
        let got = validate_structured(v).expect("合法形态必须保留");
        assert_eq!(got["kind"], "form_fill");
        assert_eq!(got["filled_fields"]["customer"]["confidence"], 0.95);
        assert_eq!(got["filled_fields"]["amount"], 12.5);
        assert_eq!(got["actions"].as_array().map(|a| a.len()), Some(3));
        // page_action 扩展键 op/args 保留
        assert_eq!(got["actions"][2]["op"], "navigate");
        assert_eq!(got["actions"][2]["args"]["path"], "/approvals");
    }

    #[test]
    fn test_validate_structured_drops_hallucinated_entries() {
        let v = json!({
            "filled_fields": {
                "good": { "value": "x" },
                "bad_array": [1, 2],
                "bad_object": { "foo": "缺 value 键" },
                "bad_extra_key": { "value": 1, "hacked": true }
            },
            "actions": [
                { "id": "ok", "label": "合法", "kind": "execute" },
                { "foo": "bar" },
                { "id": "no-kind" },
                { "id": "x", "label": "y", "kind": "pageAction" },
                { "id": "", "label": "z", "kind": "message" },
                "not-an-object"
            ]
        });
        let got = validate_structured(v).expect("尚有合法键 → Some");
        // 幻觉字段剔除
        assert!(got["filled_fields"].get("bad_array").is_none());
        assert!(got["filled_fields"].get("bad_object").is_none());
        assert!(got["filled_fields"].get("bad_extra_key").is_none());
        assert!(got["filled_fields"].get("good").is_some());
        // 幻觉元素剔除（保留唯一合法元素）
        assert_eq!(got["actions"].as_array().map(|a| a.len()), Some(1));
        assert_eq!(got["actions"][0]["id"], "ok");
    }

    #[test]
    fn test_validate_structured_full_drop_degrades_to_none() {
        // 全剔 → None（降级纯文本，不存 meta）
        assert!(validate_structured(json!({})).is_none());
        assert!(validate_structured(json!({ "filled_fields": {} })).is_none());
        assert!(validate_structured(json!({ "actions": [] })).is_none());
        assert!(validate_structured(json!({ "filled_fields": "x", "actions": 42 })).is_none());
        assert!(
            validate_structured(json!({ "filled_fields": { "f": { "no_value": 1 } } })).is_none()
        );
        // 非 object 根（array）→ None
        assert!(validate_structured(json!([1, 2, 3])).is_none());
        // 其他键保留 → 即使两个特殊键全剔也不降级
        let kept = validate_structured(json!({
            "kind": "chat",
            "filled_fields": { "bad": { "no_value": 1 } },
            "actions": [{ "foo": "bar" }]
        }))
        .expect("保留键 → Some");
        assert_eq!(kept["kind"], "chat");
        assert!(kept.get("filled_fields").is_none());
        assert!(kept.get("actions").is_none());
    }
}
