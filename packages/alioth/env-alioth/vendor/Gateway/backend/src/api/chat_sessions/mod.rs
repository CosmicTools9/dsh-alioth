//! EmpAgent Chat Session API Routes (Gateway) — Multi-Agent Architecture
//!
//! Uses standard Alioth domain tables:
//!   isahl.zc_id_thre-ai_session      — chat sessions (inherits zc_id_threads → zc_id_lifecycle)
//!   isahl.zc_id_msgs-chat_ai         — chat messages (inherits zc_id_message → zc_id_lifecycle)
//!   isahl.zc_id_prot-llm_config      — LLM provider config (inherits zc_id_protocol → zc_id_lifecycle)
//!
//! Role is derived from sender chain: fk_sender-addr → zc_id_contact_infos → zc_id_subjects → 系统用户
//! Session status managed via lifecycle relationship tables (not stored inline).
//!
//! Routes: /api/chat-sessions/*

pub mod adapters;
#[cfg(feature = "sso")]
pub mod admin_agents;
pub mod memory_store;
mod orchestrator;
pub mod ports;
pub mod ws_handler;

use actix_web::{web, HttpMessage, HttpRequest, HttpResponse, Result};
use chrono::{DateTime, Utc};
use i18n::Locale;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::PgPool;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use tokio::sync::RwLock;

use crate::i18n::I18nManagerRef;

use self::adapters::{
    agent_dispatch::AgentRouterAdapter, db_ai_contact::DbAIContactAdapter,
    db_llm_config::DbLlmConfigAdapter, db_message::SqlxMessageAdapter, db_message_meta,
    db_session::SqlxSessionAdapter,
};
use self::orchestrator::{
    CreateSessionInput, DefaultSessionOrchestrator, SessionOrchestrator, TurnInput,
};
use self::ports::{AIContactPort, MessageStorePort, SessionStorePort};
// ============================================
// 后台响应生成追踪器
//
// generate_response 改为后台任务，避免 LLM 调用阻塞前端。
// 前端通过 polling 接口获取生成结果。
// ============================================

/// 后台生成任务的状态
#[derive(Debug, Clone)]
enum GenerationStatus {
    Processing,
    Completed(ChatMessageResponse),
    Failed(String),
}

/// 失败/完成条目保留时长（防前端丢帧后重读 None → 202 假象；写时 GC 过期项）
const GENERATION_ENTRY_TTL: std::time::Duration = std::time::Duration::from_secs(300);

/// 缓存条目：状态 + 写入时间戳（TTL 判定）+ 取消信号（D2.9：watch<bool>，
/// 零新依赖；cancel 端点 / WS cancel 帧置位，process_turn 检查点消费）
#[derive(Debug, Clone)]
struct GenerationEntry {
    status: GenerationStatus,
    created_at: std::time::Instant,
    cancel_tx: tokio::sync::watch::Sender<bool>,
}

impl GenerationEntry {
    fn new(status: GenerationStatus) -> Self {
        let (cancel_tx, _) = tokio::sync::watch::channel(false);
        Self {
            status,
            created_at: std::time::Instant::now(),
            cancel_tx,
        }
    }
    fn expired(&self) -> bool {
        self.created_at.elapsed() > GENERATION_ENTRY_TTL
    }
}

/// 全局生成状态缓存：(session_id, generation_id) → Entry（D2.13 多轮并存；
/// generation_id = 本轮用户消息 id）。generation_id 精度：202 body/轮询参数
/// 字符串化（serde_zuid），内部 i64。
static GENERATION_CACHE: std::sync::LazyLock<Arc<RwLock<HashMap<(i64, i64), GenerationEntry>>>> =
    std::sync::LazyLock::new(|| Arc::new(RwLock::new(HashMap::new())));

fn generation_cache() -> &'static Arc<RwLock<HashMap<(i64, i64), GenerationEntry>>> {
    &GENERATION_CACHE
}

/// 写入新条目并顺带 GC 过期项（防内存膨胀；写路径单点调用）。
/// 返回该条目的取消接收端（供 generate_response 注入 TurnInput）。
async fn cache_insert(
    session_id: i64,
    generation_id: i64,
    status: GenerationStatus,
) -> tokio::sync::watch::Receiver<bool> {
    let cache = generation_cache().clone();
    let mut guard = cache.write().await;
    guard.retain(|_, entry| !entry.expired());
    let entry = GenerationEntry::new(status);
    let rx = entry.cancel_tx.subscribe();
    guard.insert((session_id, generation_id), entry);
    rx
}

/// 该 session 最新条目 key（无参轮询/取消用；created_at 最新）。
fn latest_generation_key(
    guard: &tokio::sync::RwLockReadGuard<'_, HashMap<(i64, i64), GenerationEntry>>,
    session_id: i64,
) -> Option<(i64, i64)> {
    guard
        .iter()
        .filter(|((sid, _), _)| *sid == session_id)
        .max_by_key(|(_, entry)| entry.created_at)
        .map(|(key, _)| *key)
}

// ============================================
// Request / Response Types
// ============================================

#[derive(Debug, Deserialize)]
pub struct CreateSessionRequest {
    pub title: Option<String>,
    pub context: Option<Value>,
}

#[derive(Debug, Deserialize)]
pub struct CreateMessageRequest {
    pub content: String,
    pub attachments: Option<Value>,
    /// 命中的知识引用 [{key,title}]（runKnowledgeInjectors 结构化清单，D2.15）
    pub knowledge_refs: Option<Value>,
    /// Message-level page/entity context (design D1): when present, the
    /// session's page_context snapshot is replaced in the same turn.
    pub context: Option<Value>,
}

#[derive(Debug, Deserialize)]
pub struct GenerateResponseRequest {
    /// 模型档位（chat 模型切换）："deep" | "flash"；缺省 = deep（主模型）
    pub model: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ModelOptionResponse {
    /// 档位 id："deep"（主模型）| "flash"（快速档）
    pub id: String,
    /// 实际模型名（来自系统 LLM 配置）
    pub model: String,
}

#[derive(Debug, Deserialize)]
pub struct SwitchAgentRequest {
    pub agent_code: String,
}

#[derive(Debug, Deserialize)]
pub struct ExecuteActionRequest {
    pub action_id: String,
    pub params: Option<Value>,
    pub confirmed: bool,
}

#[derive(Debug, Serialize)]
pub struct ExecuteActionResponse {
    pub success: bool,
    pub action_id: String,
    pub result: Option<Value>,
    pub message: String,
}

#[derive(Debug, Deserialize)]
pub struct MessagesQuery {
    #[serde(default, with = "common::serde_zuid::opt")]
    pub offset: Option<i64>,
    #[serde(default, with = "common::serde_zuid::opt")]
    pub limit: Option<i64>,
}

/// GET /{id}/response 轮询参数（D2.13）：generation_id 字符串接收（ID 精度）
#[derive(Debug, Deserialize)]
pub struct GenerationQuery {
    #[serde(default, with = "common::serde_zuid::opt")]
    pub generation_id: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct ChatSessionResponse {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub title: String,
    pub status: String,
    pub agent_code: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChatMessageResponse {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub role: String,
    pub content: String,
    pub created_at: DateTime<Utc>,
    pub agent_code: String,
    pub structured: Option<Value>,
    pub requires_input: bool,
    pub suggested_actions: Vec<String>,
    /// token 用量（assistant 消息；后端生成时统计，旧消息无 meta → 缺省省略）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Value>,
    /// 命中的知识引用 [{key,title}]（随用户消息注入并回显，见 fix-chat-ai-feature-gaps D2.15）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub knowledge_refs: Option<Value>,
}

#[derive(Debug, Serialize)]
pub struct AgentInfoResponse {
    pub code: String,
    pub name: String,
    pub description: String,
    pub capabilities: Vec<String>,
    pub user_selectable: bool,
    pub sort_order: i32,
    pub icon: String,
    pub color: String,
    pub category: String,
}

#[derive(Debug, Serialize)]
pub struct MessagesListResponse {
    pub messages: Vec<ChatMessageResponse>,
    #[serde(with = "common::serde_zuid")]
    pub offset: i64,
    #[serde(with = "common::serde_zuid")]
    pub limit: i64,
}

#[derive(Debug, Serialize)]
struct GenerationStatusResponse {
    pub status: String,
    pub message: Option<ChatMessageResponse>,
    /// status="failed" 时的后端根因文本（前端轮询直接展示/抛出）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

// ============================================
// API Response Wrappers
// ============================================

#[derive(Serialize)]
struct ApiSuccess<T: Serialize> {
    success: bool,
    data: T,
}

#[derive(Serialize)]
struct ApiError {
    success: bool,
    error: ErrorDetail,
}

#[derive(Serialize)]
struct ErrorDetail {
    code: String,
    message: String,
}

fn success<T: Serialize>(data: T) -> HttpResponse {
    HttpResponse::Ok().json(ApiSuccess {
        success: true,
        data,
    })
}

fn accepted<T: Serialize>(data: T) -> HttpResponse {
    HttpResponse::Accepted().json(ApiSuccess {
        success: true,
        data,
    })
}

fn error(code: &str, message: &str, status: actix_web::http::StatusCode) -> HttpResponse {
    HttpResponse::build(status).json(ApiError {
        success: false,
        error: ErrorDetail {
            code: code.to_string(),
            message: message.to_string(),
        },
    })
}

fn bad_request(code: &str, message: &str) -> HttpResponse {
    error(code, message, actix_web::http::StatusCode::BAD_REQUEST)
}

fn not_found(code: &str, message: &str) -> HttpResponse {
    error(code, message, actix_web::http::StatusCode::NOT_FOUND)
}

fn internal_error(code: &str, message: &str) -> HttpResponse {
    error(
        code,
        message,
        actix_web::http::StatusCode::INTERNAL_SERVER_ERROR,
    )
}

/// R2（D2.5）内部错误泛化文案：DB/SQL 原文只进 telemetry，不透传响应
/// （chat_sessions 全部 internal_error 调用点统一使用；code 保留供前端分支）。
const GENERIC_INTERNAL_MSG: &str = "操作失败，请稍后重试";

// ============================================
// Helpers
// ============================================

fn extract_user_id(req: &HttpRequest) -> Result<i64, actix_web::Error> {
    req.extensions()
        .get::<common::context::RequestContext>()
        .map(|ctx| ctx.user_id)
        .ok_or_else(|| actix_web::error::ErrorUnauthorized("Authentication required"))
}

static SHARED_ORCHESTRATOR: OnceLock<Arc<DefaultSessionOrchestrator>> = OnceLock::new();

fn build_orchestrator(pool: &PgPool, i18n: I18nManagerRef) -> Arc<DefaultSessionOrchestrator> {
    let pool = pool.clone();
    SHARED_ORCHESTRATOR
        .get_or_init(move || {
            let session_store = Arc::new(SqlxSessionAdapter::new(pool.clone()));
            let message_store = Arc::new(SqlxMessageAdapter::new(pool.clone()));
            let llm_config = Arc::new(DbLlmConfigAdapter::new(pool.clone()));
            let agent_dispatch = Arc::new(AgentRouterAdapter::new(pool.clone()));
            let ai_contact = Arc::new(DbAIContactAdapter::new(pool.clone(), i18n.clone()));
            Arc::new(DefaultSessionOrchestrator::new(
                pool,
                i18n,
                session_store,
                message_store,
                llm_config,
                agent_dispatch,
                ai_contact,
            ))
        })
        .clone()
}

// ============================================
// Handlers
// ============================================

/// POST /api/chat-sessions
pub async fn create_session(
    pool: web::Data<PgPool>,
    i18n_manager: web::Data<I18nManagerRef>,
    req: HttpRequest,
    body: web::Json<CreateSessionRequest>,
) -> Result<HttpResponse> {
    let locale = req
        .extensions()
        .get::<Locale>()
        .cloned()
        .unwrap_or(Locale::new("zh-CN"));

    let orchestrator = build_orchestrator(pool.get_ref(), i18n_manager.get_ref().clone());

    let input = CreateSessionInput {
        title: body.title.clone(),
        context: body.context.clone(),
        user_id: extract_user_id(&req)?,
        locale: locale.to_string(),
    };

    match orchestrator.create_session(input).await {
        Ok(response) => Ok(success(response)),
        Err(e) if e.starts_with("TRIGGER_BLOCKED") => {
            Ok(HttpResponse::BadRequest().json(serde_json::json!({
                "success": false,
                "error": "TRIGGER_BLOCKED",
                "message": e
            })))
        }
        Err(e) => {
            common::telemetry::error!("Failed to create chat session: {}", e);
            Ok(internal_error("DB_ERROR", GENERIC_INTERNAL_MSG))
        }
    }
}

/// GET /api/chat-sessions — List user's sessions
pub async fn list_sessions(
    pool: web::Data<PgPool>,
    i18n_manager: web::Data<I18nManagerRef>,
    req: HttpRequest,
) -> Result<HttpResponse> {
    let user_id = extract_user_id(&req)?;
    let _orchestrator = build_orchestrator(pool.get_ref(), i18n_manager.get_ref().clone());

    let session_store = SqlxSessionAdapter::new(pool.get_ref().clone());
    match session_store.list_sessions(user_id).await {
        Ok(sessions) => {
            let responses: Vec<ChatSessionResponse> = sessions
                .into_iter()
                .map(|s| ChatSessionResponse {
                    id: s.id,
                    title: s.title,
                    status: "active".to_string(),
                    agent_code: None,
                    created_at: s.created_at,
                    updated_at: s.updated_at,
                })
                .collect();
            Ok(success(responses))
        }
        Err(e) => {
            common::telemetry::error!("Failed to list chat sessions: {}", e);
            Ok(internal_error("DB_ERROR", GENERIC_INTERNAL_MSG))
        }
    }
}

/// PATCH /api/chat-sessions/{id} — 手动重命名（D2.8：title trim 1-64 字符）
#[derive(Debug, Deserialize)]
pub struct RenameSessionRequest {
    pub title: String,
}

pub async fn rename_session(
    pool: web::Data<PgPool>,
    _i18n_manager: web::Data<I18nManagerRef>,
    req: HttpRequest,
    path: web::Path<i64>,
    body: web::Json<RenameSessionRequest>,
) -> Result<HttpResponse> {
    let session_id = path.into_inner();
    let user_id = extract_user_id(&req)?;

    let title = body.title.trim().to_string();
    let len = title.chars().count();
    if len == 0 || len > 64 {
        return Ok(bad_request(
            "INVALID_TITLE",
            "title must be 1-64 characters after trim",
        ));
    }

    let session_store = SqlxSessionAdapter::new(pool.get_ref().clone());
    match session_store
        .update_session_title(session_id, user_id, &title)
        .await
    {
        Ok(true) => match session_store.get_session(session_id, user_id).await {
            Ok(Some(s)) => Ok(success(ChatSessionResponse {
                id: s.id,
                title: s.title,
                status: "active".to_string(),
                agent_code: None,
                created_at: s.created_at,
                updated_at: s.updated_at,
            })),
            _ => Ok(internal_error(
                "DB_ERROR",
                "Session updated but reload failed",
            )),
        },
        Ok(false) => Ok(not_found(
            "SESSION_NOT_FOUND",
            &format!("Chat session {} not found", session_id),
        )),
        Err(e) => {
            common::telemetry::error!("Failed to rename session: {}", e);
            Ok(internal_error("DB_ERROR", GENERIC_INTERNAL_MSG))
        }
    }
}

/// DELETE /api/chat-sessions/{id} — Soft-delete a session
pub async fn delete_session(
    pool: web::Data<PgPool>,
    _i18n_manager: web::Data<I18nManagerRef>,
    req: HttpRequest,
    path: web::Path<i64>,
) -> Result<HttpResponse> {
    let session_id = path.into_inner();
    let user_id = extract_user_id(&req)?;

    let session_store = SqlxSessionAdapter::new(pool.get_ref().clone());
    match session_store.delete_session(session_id, user_id).await {
        Ok(()) => Ok(HttpResponse::NoContent().finish()),
        Err(e) if e == "SESSION_NOT_FOUND" => Ok(not_found(
            "SESSION_NOT_FOUND",
            &format!("Chat session {} not found", session_id),
        )),
        Err(e) => {
            common::telemetry::error!("Failed to delete session: {}", e);
            Ok(internal_error("DB_ERROR", GENERIC_INTERNAL_MSG))
        }
    }
}

/// POST /api/chat-sessions/messages/{msg_id}/feedback — 消息反馈（D2.16）
#[derive(Debug, Deserialize)]
pub struct FeedbackRequest {
    pub rating: String,
    #[serde(default)]
    pub comment: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct FeedbackResponse {
    /// 当前生效 rating；None = toggle 命中已撤销（前端据此熄灭高亮）
    pub rating: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

pub async fn message_feedback(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    path: web::Path<i64>,
    body: web::Json<FeedbackRequest>,
) -> Result<HttpResponse> {
    let msg_id = path.into_inner();
    let user_id = extract_user_id(&req)?;

    let rating = body.rating.trim().to_string();
    if rating != "up" && rating != "down" {
        return Ok(bad_request(
            "INVALID_RATING",
            "rating must be \"up\" or \"down\"",
        ));
    }
    let comment = body
        .comment
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .map(String::from);

    let message_store = SqlxMessageAdapter::new(pool.get_ref().clone());

    // owner 校验：消息存在且所属 session 的 created_by_id == user（404 不区分
    // 不存在/非本人，防消息 id 枚举）
    match message_store.message_belongs_to_user(msg_id, user_id).await {
        Ok(true) => {}
        Ok(false) => {
            return Ok(not_found(
                "MESSAGE_NOT_FOUND",
                &format!("Message {} not found", msg_id),
            ))
        }
        Err(e) => {
            common::telemetry::error!("Failed to check message owner: {}", e);
            return Ok(internal_error("DB_ERROR", GENERIC_INTERNAL_MSG));
        }
    }

    match message_store
        .set_message_feedback(msg_id, user_id, &rating, comment.as_deref())
        .await
    {
        Ok(current) => Ok(success(FeedbackResponse {
            rating: current,
            comment,
        })),
        Err(e) => {
            common::telemetry::error!("Failed to save message feedback: {}", e);
            Ok(internal_error("DB_ERROR", GENERIC_INTERNAL_MSG))
        }
    }
}

/// POST /api/chat-sessions/{id}/messages
pub async fn add_message(
    pool: web::Data<PgPool>,
    i18n_manager: web::Data<I18nManagerRef>,
    message_req: HttpRequest,
    path: web::Path<i64>,
    req: web::Json<CreateMessageRequest>,
) -> Result<HttpResponse> {
    let session_id = path.into_inner();
    let user_id = extract_user_id(&message_req)?;

    let orchestrator = build_orchestrator(pool.get_ref(), i18n_manager.get_ref().clone());

    match orchestrator
        .add_message(
            session_id,
            &req.content,
            req.context.clone(),
            req.attachments.clone(),
            req.knowledge_refs.clone(),
            user_id,
        )
        .await
    {
        Ok(response) => Ok(success(response)),
        Err(e) if e == "SESSION_NOT_FOUND" => Ok(not_found(
            "SESSION_NOT_FOUND",
            &format!("Chat session {} not found", session_id),
        )),
        Err(e) => {
            common::telemetry::error!("Failed to add chat message: {}", e);
            Ok(internal_error("DB_ERROR", GENERIC_INTERNAL_MSG))
        }
    }
}

/// GET /api/chat-sessions/{id}/messages — Load paginated message history
pub async fn get_messages(
    pool: web::Data<PgPool>,
    i18n_manager: web::Data<I18nManagerRef>,
    req: HttpRequest,
    path: web::Path<i64>,
    query: web::Query<MessagesQuery>,
) -> Result<HttpResponse> {
    let session_id = path.into_inner();
    let user_id = extract_user_id(&req)?;
    let offset = query.offset.unwrap_or(0);
    let limit = query.limit.unwrap_or(50).min(100);

    // 角色推导与 orchestrator.derive_role 同语义：fk_sender-addr == AI contact
    // → assistant，其余（含 NULL sender）→ user。历史里用户消息此前被硬编码
    // 成 "unknown"，前端一律按 AI 气泡渲染。
    let locale = req
        .extensions()
        .get::<Locale>()
        .cloned()
        .unwrap_or(Locale::new("zh-CN"));
    let ai_contact =
        DbAIContactAdapter::new(pool.get_ref().clone(), i18n_manager.get_ref().clone());
    // 解析失败（如 AI contact 引导失败）降级为 None：全部按 user 渲染，
    // 不让历史接口因联系人引导问题 500。
    let ai_contact_id = ai_contact
        .resolve_ai_contact_id(&locale)
        .await
        .unwrap_or(None);

    let message_store = SqlxMessageAdapter::new(pool.get_ref().clone());
    match message_store
        .get_messages(session_id, user_id, offset, limit)
        .await
    {
        Ok(rows) => {
            let messages: Vec<ChatMessageResponse> = rows
                .into_iter()
                .map(|r| {
                    let is_assistant = r
                        .fk_sender_addr
                        .zip(ai_contact_id)
                        .map(|(sender, ai)| sender == ai)
                        .unwrap_or(false);
                    ChatMessageResponse {
                        id: r.id,
                        role: if is_assistant { "assistant" } else { "user" }.to_string(),
                        content: r.content.unwrap_or_default(),
                        created_at: r.created_at,
                        // meta JOIN 恢复真实 agent_code/structured（旧消息无 meta →
                        // assistant 回退 general 与既有硬编码同值；user 恒空串）
                        agent_code: if is_assistant {
                            r.agent_code
                                .clone()
                                .unwrap_or_else(|| "general".to_string())
                        } else {
                            String::new()
                        },
                        structured: if is_assistant {
                            r.structured.clone()
                        } else {
                            None
                        },
                        requires_input: false,
                        suggested_actions: vec![],
                        usage: r.usage.clone(),
                        knowledge_refs: r.knowledge_refs.clone(),
                    }
                })
                .collect();
            Ok(success(MessagesListResponse {
                messages,
                offset,
                limit,
            }))
        }
        Err(e) => {
            common::telemetry::error!("Failed to get messages: {}", e);
            Ok(internal_error("DB_ERROR", GENERIC_INTERNAL_MSG))
        }
    }
}

/// 触发一轮后台生成（generate-response / regenerate 共用）：解析本轮用户消息
/// id 为 generation_id → cache Processing → spawn process_turn → 202。
/// Err("NO_USER_MESSAGE") / DB 类错误由调用方映射 HTTP。
async fn launch_generation(
    pool: PgPool,
    i18n: I18nManagerRef,
    locale: Locale,
    session_id: i64,
    user_id: i64,
    model: Option<String>,
) -> Result<HttpResponse, String> {
    let locale_str = locale.to_string();
    // D2.13：generation_id = 本轮用户消息 id
    let ai_contact = DbAIContactAdapter::new(pool.clone(), i18n.clone());
    let ai_contact_id = ai_contact
        .resolve_ai_contact_id(&locale)
        .await
        .unwrap_or(None);
    let message_store = SqlxMessageAdapter::new(pool.clone());
    let generation_id = match message_store
        .get_last_user_message_row(session_id, ai_contact_id)
        .await
    {
        Ok(Some(row)) => row.id,
        Ok(None) => return Err("NO_USER_MESSAGE".to_string()),
        // DB 错误（如缺表/连接失败）不得吞为 NO_USER_MESSAGE——透传根因
        // 便于定位（实测 wz 库缺 019 表时被误导报「请先发送消息」）。
        Err(e) => return Err(format!("DB_ERROR: {}", e)),
    };

    let cancel_rx = cache_insert(session_id, generation_id, GenerationStatus::Processing).await;

    // Spawn background task — do NOT block the HTTP worker
    tokio::spawn(async move {
        let orchestrator = build_orchestrator(&pool, i18n);

        let input = TurnInput {
            session_id,
            user_id,
            locale: locale_str,
            model,
            cancel: Some(cancel_rx),
        };

        let result = orchestrator.process_turn(input, None).await;

        match result {
            Ok(turn_result) => {
                cache_insert(
                    session_id,
                    generation_id,
                    GenerationStatus::Completed(turn_result.message),
                )
                .await;
            }
            Err(e) => {
                common::telemetry::error!(
                    "Background generation failed for session {}: {}",
                    session_id,
                    e
                );
                cache_insert(session_id, generation_id, GenerationStatus::Failed(e)).await;
            }
        }
    });

    // Return 202 Accepted immediately（generation_id 字符串化——ID 精度）
    Ok(accepted(serde_json::json!({
        "session_id": session_id.to_string(),
        "generation_id": generation_id.to_string(),
        "status": "processing"
    })))
}

/// POST /api/chat-sessions/{id}/generate-response — Async (background task)
pub async fn generate_response(
    pool: web::Data<PgPool>,
    i18n_manager: web::Data<I18nManagerRef>,
    req: HttpRequest,
    path: web::Path<i64>,
    // 可选 JSON 体（chat 模型切换）：无体/解析失败 → None = deep 档默认
    body: Option<web::Json<GenerateResponseRequest>>,
) -> Result<HttpResponse> {
    let locale = req
        .extensions()
        .get::<Locale>()
        .cloned()
        .unwrap_or(Locale::new("zh-CN"));
    let session_id = path.into_inner();
    let user_id = extract_user_id(&req)?;

    match launch_generation(
        pool.get_ref().clone(),
        i18n_manager.get_ref().clone(),
        locale,
        session_id,
        user_id,
        body.as_ref().and_then(|b| b.model.clone()),
    )
    .await
    {
        Ok(resp) => Ok(resp),
        Err(e) if e == "NO_USER_MESSAGE" => Ok(bad_request(
            "NO_USER_MESSAGE",
            "generate-response 前必须先在会话中发送一条用户消息",
        )),
        Err(e) => {
            common::telemetry::error!(
                "Failed to launch generation for session {}: {}",
                session_id,
                e
            );
            Ok(internal_error("DB_ERROR", GENERIC_INTERNAL_MSG))
        }
    }
}

/// POST /api/chat-sessions/{id}/regenerate — 重生成（协调者契约，M3 已调用）：
/// 软删最后一条 assistant 消息 + 清 meta → 触发新一轮生成（202 含 generation_id）。
pub async fn regenerate_response(
    pool: web::Data<PgPool>,
    i18n_manager: web::Data<I18nManagerRef>,
    req: HttpRequest,
    path: web::Path<i64>,
    // 可选 JSON 体（chat 模型切换）：无体/解析失败 → None = deep 档默认
    body: Option<web::Json<GenerateResponseRequest>>,
) -> Result<HttpResponse> {
    let locale = req
        .extensions()
        .get::<Locale>()
        .cloned()
        .unwrap_or(Locale::new("zh-CN"));
    let session_id = path.into_inner();
    let user_id = extract_user_id(&req)?;

    // 并发防护（P3）：该 session 已有进行中生成时拒绝重复 regenerate
    // （409 明确语义；前端 regenerate 与轮询并发/双击由 polling 收敛）
    {
        let cache = generation_cache().clone();
        let guard = cache.read().await;
        let in_flight = guard.iter().any(|((sid, _), entry)| {
            *sid == session_id && matches!(entry.status, GenerationStatus::Processing)
        });
        drop(guard);
        if in_flight {
            return Ok(HttpResponse::Conflict().json(serde_json::json!({
                "success": false,
                "error": {
                    "code": "GENERATION_IN_PROGRESS",
                    "message": "该会话已有生成进行中，请等待完成或先取消"
                }
            })));
        }
    }
    let ai_contact =
        DbAIContactAdapter::new(pool.get_ref().clone(), i18n_manager.get_ref().clone());
    let ai_contact_id = match ai_contact.resolve_ai_contact_id(&locale).await {
        Ok(Some(id)) => id,
        _ => {
            return Ok(internal_error(
                "AI_CONTACT_UNAVAILABLE",
                "AI contact 不可用，无法定位 assistant 消息",
            ))
        }
    };
    // 无最后 assistant 消息也可重生成（直接基于用户消息再来一轮）
    if let Err(e) =
        db_message_meta::soft_delete_last_assistant(pool.get_ref(), session_id, ai_contact_id).await
    {
        common::telemetry::error!("regenerate: 清理旧回复失败: {}", e);
        return Ok(internal_error("DB_ERROR", GENERIC_INTERNAL_MSG));
    }

    match launch_generation(
        pool.get_ref().clone(),
        i18n_manager.get_ref().clone(),
        locale,
        session_id,
        user_id,
        body.as_ref().and_then(|b| b.model.clone()),
    )
    .await
    {
        Ok(resp) => Ok(resp),
        Err(e) if e == "NO_USER_MESSAGE" => Ok(bad_request(
            "NO_USER_MESSAGE",
            "regenerate 前必须先在会话中发送一条用户消息",
        )),
        Err(e) => {
            common::telemetry::error!(
                "Failed to launch regeneration for session {}: {}",
                session_id,
                e
            );
            Ok(internal_error("DB_ERROR", GENERIC_INTERNAL_MSG))
        }
    }
}

/// DELETE /api/chat-sessions/messages/{msg_id} — 软删消息（协调者契约，M3 已调用）
pub async fn delete_message(
    pool: web::Data<PgPool>,
    _i18n_manager: web::Data<I18nManagerRef>,
    req: HttpRequest,
    path: web::Path<i64>,
) -> Result<HttpResponse> {
    let msg_id = path.into_inner();
    let user_id = extract_user_id(&req)?;

    let message_store = SqlxMessageAdapter::new(pool.get_ref().clone());
    match message_store.message_belongs_to_user(msg_id, user_id).await {
        Ok(true) => {}
        Ok(false) => {
            return Ok(not_found(
                "MESSAGE_NOT_FOUND",
                &format!("Message {} not found", msg_id),
            ))
        }
        Err(e) => {
            common::telemetry::error!("Failed to check message owner: {}", e);
            return Ok(internal_error("DB_ERROR", GENERIC_INTERNAL_MSG));
        }
    }

    match message_store.soft_delete_message(msg_id).await {
        Ok(true) => Ok(HttpResponse::NoContent().finish()),
        Ok(false) => Ok(not_found(
            "MESSAGE_NOT_FOUND",
            &format!("Message {} not found", msg_id),
        )),
        Err(e) => {
            common::telemetry::error!("Failed to delete message: {}", e);
            Ok(internal_error("DB_ERROR", GENERIC_INTERNAL_MSG))
        }
    }
}

/// GET /api/chat-sessions/{id}/response — Poll for async generation result
/// 支持 ?generation_id=<string> 精确轮询；无参 = 该 session 最新条目（向后兼容）。
pub async fn get_response_status(
    pool: web::Data<PgPool>,
    _i18n_manager: web::Data<I18nManagerRef>,
    req: HttpRequest,
    path: web::Path<i64>,
    query: web::Query<GenerationQuery>,
) -> Result<HttpResponse> {
    let session_id = path.into_inner();
    let user_id = extract_user_id(&req)?;

    // 属主校验（404 不区分不存在/非本人；与 cancel/delete 端点同形）
    match session_owned_by(pool.get_ref(), session_id, user_id).await {
        Ok(true) => {}
        Ok(false) => {
            return Ok(not_found(
                "SESSION_NOT_FOUND",
                &format!("Chat session {} not found", session_id),
            ))
        }
        Err(e) => {
            common::telemetry::error!("response poll: owner check failed: {}", e);
            return Ok(internal_error("DB_ERROR", GENERIC_INTERNAL_MSG));
        }
    }

    let cache = generation_cache().clone();
    let guard = cache.read().await;

    // key 解析：显式 generation_id 精确查；缺省 → session 最新条目
    let key = match query.generation_id {
        Some(gid) => Some((session_id, gid)),
        None => latest_generation_key(&guard, session_id),
    };
    let Some(key) = key else {
        return Ok(accepted(GenerationStatusResponse {
            status: "processing".to_string(),
            message: None,
            error: None,
        }));
    };

    match guard.get(&key) {
        Some(GenerationEntry {
            status: GenerationStatus::Completed(msg),
            ..
        }) => {
            let response = msg.clone();
            drop(guard);
            // Clean up cache entry after successful delivery
            let mut write_guard = cache.write().await;
            write_guard.remove(&key);
            Ok(success(GenerationStatusResponse {
                status: "completed".to_string(),
                message: Some(response),
                error: None,
            }))
        }
        Some(GenerationEntry {
            status: GenerationStatus::Failed(err),
            ..
        }) => {
            // 失败：200 + status:"failed"（前端轮询已处理该分支立即终止）。
            // 不读后即删——保留至 TTL，防并发/重试轮询拿到 None → 202 假象
            // （2026-09-01 实证：500 被 ApiClient 重试拦截器吞掉后失败证据消失）。
            // 过期条目由 cache_insert 的 GC 清理。
            let err_msg = err.clone();
            Ok(success(GenerationStatusResponse {
                status: "failed".to_string(),
                message: None,
                error: Some(err_msg),
            }))
        }
        Some(GenerationEntry {
            status: GenerationStatus::Processing,
            ..
        })
        | None => {
            // None means no generation has been triggered yet — treat as processing
            Ok(accepted(GenerationStatusResponse {
                status: "processing".to_string(),
                message: None,
                error: None,
            }))
        }
    }
}

/// 会话属主校验（poll/cancel 端点用）：session 存在且 created_by_id == user。
/// Ok(true) 属主；Ok(false) 不存在或非本人（404 不区分，防枚举）。
async fn session_owned_by(pool: &PgPool, session_id: i64, user_id: i64) -> Result<bool, String> {
    let owner: Option<i64> = sqlx::query_scalar(
        r#"SELECT created_by_id FROM isahl."zc_id_thre-ai_session"
           WHERE id = $1 AND deleted_at IS NULL"#,
    )
    .bind(session_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("session owner check failed: {}", e))?;
    Ok(owner == Some(user_id))
}

/// POST /api/chat-sessions/{id}/cancel — 取消当前后台生成（D2.9）
pub async fn cancel_generation(
    pool: web::Data<PgPool>,
    _i18n_manager: web::Data<I18nManagerRef>,
    req: HttpRequest,
    path: web::Path<i64>,
) -> Result<HttpResponse> {
    let session_id = path.into_inner();
    let user_id = extract_user_id(&req)?;

    // 属主校验（404 不区分不存在/非本人）
    match session_owned_by(pool.get_ref(), session_id, user_id).await {
        Ok(true) => {}
        Ok(false) => {
            return Ok(not_found(
                "SESSION_NOT_FOUND",
                &format!("Chat session {} not found", session_id),
            ))
        }
        Err(e) => {
            common::telemetry::error!("cancel: owner check failed: {}", e);
            return Ok(internal_error("DB_ERROR", GENERIC_INTERNAL_MSG));
        }
    }

    let cache = generation_cache().clone();
    let guard = cache.read().await;
    // D2.13：cancel 无 generation_id 参数 → 取消该 session 最新一条生成
    let key = latest_generation_key(&guard, session_id);
    let cancelled = match key.and_then(|k| guard.get(&k)) {
        Some(GenerationEntry {
            status: GenerationStatus::Processing,
            cancel_tx,
            ..
        }) => {
            let sent = cancel_tx.send(true);
            sent.is_ok()
        }
        // 已完成/失败/无条目：无可取消对象（前端按钮态由 polling 决定）
        _ => false,
    };
    drop(guard);

    Ok(success(serde_json::json!({
        "session_id": session_id.to_string(),
        "cancelled": cancelled
    })))
}

/// GET /api/chat-sessions/agents
/// List all available agents（合并数据库配置；含 code="auto" 自动路由入口）
pub async fn list_agents(
    pool: web::Data<PgPool>,
    i18n_manager: web::Data<I18nManagerRef>,
    req: HttpRequest,
) -> Result<HttpResponse> {
    let locale = req
        .extensions()
        .get::<Locale>()
        .cloned()
        .unwrap_or(Locale::new("zh-CN"))
        .to_string();
    let orchestrator = build_orchestrator(pool.get_ref(), i18n_manager.get_ref().clone());

    match orchestrator.list_agents(&locale).await {
        Ok(agents) => Ok(success(agents)),
        Err(e) => {
            common::telemetry::error!("Failed to list agents: {}", e);
            Ok(internal_error("AGENT_ERROR", GENERIC_INTERNAL_MSG))
        }
    }
}

/// GET /api/chat-sessions/model-options — 模型档位与实际模型名（chat 模型切换）
pub async fn model_options(
    pool: web::Data<PgPool>,
    i18n_manager: web::Data<I18nManagerRef>,
) -> Result<HttpResponse> {
    let orchestrator = build_orchestrator(pool.get_ref(), i18n_manager.get_ref().clone());

    match orchestrator.list_model_options().await {
        Ok(options) => Ok(success(options)),
        Err(e) => {
            common::telemetry::error!("Failed to load model options: {}", e);
            Ok(internal_error("LLM_CONFIG_ERROR", GENERIC_INTERNAL_MSG))
        }
    }
}

pub async fn switch_agent(
    pool: web::Data<PgPool>,
    i18n_manager: web::Data<I18nManagerRef>,
    req: HttpRequest,
    path: web::Path<i64>,
    body: web::Json<SwitchAgentRequest>,
) -> Result<HttpResponse> {
    let session_id = path.into_inner();
    let user_id = extract_user_id(&req)?;

    let orchestrator = build_orchestrator(pool.get_ref(), i18n_manager.get_ref().clone());

    match orchestrator
        .switch_agent(session_id, &body.agent_code, user_id)
        .await
    {
        Ok(()) => Ok(success(serde_json::json!({
            "session_id": session_id.to_string(),
            "agent_code": body.agent_code,
            "message": "Agent switched successfully"
        }))),
        Err(e) if e == "SESSION_NOT_FOUND" => {
            Ok(not_found("SESSION_NOT_FOUND", "Session not found"))
        }
        Err(e) if e.starts_with("Agent") => Ok(bad_request("INVALID_AGENT", &e)),
        Err(e) => {
            common::telemetry::error!("Failed to switch agent: {}", e);
            Ok(internal_error("DB_ERROR", GENERIC_INTERNAL_MSG))
        }
    }
}

/// POST /api/chat-sessions/{id}/execute-action
/// 执行业务动作（D2.1）：action_id = "agent_action:<type>" 或上一轮 structured
/// 建议动作 id；Explicit 级（batch_approve/status_transition）必须 confirmed=true。
pub async fn execute_action(
    pool: web::Data<PgPool>,
    i18n_manager: web::Data<I18nManagerRef>,
    req: HttpRequest,
    path: web::Path<i64>,
    body: web::Json<ExecuteActionRequest>,
) -> Result<HttpResponse> {
    let session_id = path.into_inner();
    let user_id = extract_user_id(&req)?;

    let orchestrator = build_orchestrator(pool.get_ref(), i18n_manager.get_ref().clone());
    match orchestrator
        .execute_action(
            session_id,
            &body.action_id,
            body.params.clone(),
            body.confirmed,
            user_id,
        )
        .await
    {
        Ok(resp) => Ok(success(resp)),
        Err(e) if e == "SESSION_NOT_FOUND" => {
            Ok(not_found("SESSION_NOT_FOUND", "Session not found"))
        }
        Err(e) if e.starts_with("CONFIRMATION_REQUIRED") => {
            Ok(bad_request("CONFIRMATION_REQUIRED", &e))
        }
        Err(e) if e.starts_with("ACTION_NOT_FOUND") => Ok(not_found(
            "ACTION_NOT_FOUND",
            &format!("Action '{}' not found in session state", body.action_id),
        )),
        Err(e)
            if e.starts_with("MISSING_")
                || e.starts_with("INVALID_")
                || e.starts_with("ACTION_DENIED")
                || e.starts_with("UNKNOWN_ACTION_TYPE") =>
        {
            Ok(bad_request("ACTION_ERROR", &e))
        }
        Err(e) => {
            common::telemetry::error!("execute-action failed (session {}): {}", session_id, e);
            Ok(internal_error(
                "ACTION_EXECUTION_ERROR",
                GENERIC_INTERNAL_MSG,
            ))
        }
    }
}

// ============================================
// Route Configuration
// ============================================

pub fn configure_routes(cfg: &mut web::ServiceConfig) {
    // D2.17 agent 管理面（NGAC admin 守卫在 handler 内，SSO-only）。
    // MUST 先于 /chat-sessions 主 scope 注册——actix-web 4 的 scope 前缀匹配
    // 不回退（实测：主 scope 的 /{id} 通配先吃 "admin" 段 → 后注册 admin scope
    // 404）。长前缀先行是 actix scope 并列的可靠次序。
    #[cfg(feature = "sso")]
    cfg.service(
        web::scope("/chat-sessions/admin/agents")
            .route("", web::get().to(admin_agents::list_agents))
            .route("", web::post().to(admin_agents::create_agent))
            .route("/{code}", web::patch().to(admin_agents::patch_agent))
            .route("/{code}", web::delete().to(admin_agents::delete_agent)),
    );

    cfg.service(
        web::scope("/chat-sessions")
            .route("", web::post().to(create_session))
            .route("", web::get().to(list_sessions))
            .route("/agents", web::get().to(list_agents))
            .route("/model-options", web::get().to(model_options))
            .route("/{id}", web::delete().to(delete_session))
            .route("/{id}", web::patch().to(rename_session))
            .route("/{id}/messages", web::post().to(add_message))
            .route("/{id}/messages", web::get().to(get_messages))
            .route(
                "/messages/{msg_id}/feedback",
                web::post().to(message_feedback),
            )
            .route("/messages/{msg_id}", web::delete().to(delete_message))
            .route("/{id}/generate-response", web::post().to(generate_response))
            .route("/{id}/regenerate", web::post().to(regenerate_response))
            .route("/{id}/cancel", web::post().to(cancel_generation))
            .route("/{id}/response", web::get().to(get_response_status))
            .route("/{id}/switch-agent", web::post().to(switch_agent))
            .route("/{id}/execute-action", web::post().to(execute_action))
            .route("/{id}/ws", web::get().to(ws_handler::ws_connect)),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// D2.13：多轮并存 key 精确轮询语义（cache 层，无 DB）
    #[tokio::test]
    async fn generation_cache_exact_and_latest_polling() {
        // 清场（静态缓存跨测试进程内共享）
        generation_cache().write().await.clear();

        let s = 335424851373000i64;
        let g1 = 335424851373001i64;
        let g2 = 335424851373002i64;
        let _rx1 = cache_insert(s, g1, GenerationStatus::Processing).await;
        let _rx2 = cache_insert(s, g2, GenerationStatus::Processing).await;

        // 精确轮询 g1 → Processing
        {
            let guard = generation_cache().read().await;
            let key = (s, g1);
            assert!(matches!(
                guard.get(&key).map(|e| &e.status),
                Some(GenerationStatus::Processing)
            ));
            // 无参 → session 最新 = g2
            let latest = latest_generation_key(&guard, s);
            assert_eq!(latest, Some((s, g2)));
        }

        // g2 完成后仅删精确 key；g1 仍在（多轮并存互不干扰）
        {
            let mut guard = generation_cache().write().await;
            guard.remove(&(s, g2));
        }
        {
            let guard = generation_cache().read().await;
            assert!(guard.get(&(s, g1)).is_some(), "g1 不得被 g2 完成删除波及");
            assert!(guard.get(&(s, g2)).is_none());
            // 删除后无参轮询回落 g1
            assert_eq!(latest_generation_key(&guard, s), Some((s, g1)));
        }
        generation_cache().write().await.clear();
    }

    /// D2.9：cancel 置位后接收端可见（取消语义 cache 层）
    #[tokio::test]
    async fn generation_cache_cancel_flag() {
        let s = 335424851373100i64;
        let g = 335424851373101i64;
        let mut rx = cache_insert(s, g, GenerationStatus::Processing).await;
        assert!(!*rx.borrow(), "初始未取消");

        let cache = generation_cache().clone();
        let guard = cache.read().await;
        let entry = guard.get(&(s, g)).expect("entry");
        assert!(entry.cancel_tx.send(true).is_ok());
        drop(guard);

        rx.changed().await.expect("signal");
        assert!(*rx.borrow(), "cancel 置位必须可见");
        generation_cache().write().await.clear();
    }

    /// R2：internal_error 响应结构不变（success/error.code/error.message），
    /// message 为泛化中文文案（无 SQL/DB 原文；code 保留供前端分支）。
    #[actix_web::test]
    async fn internal_error_generic_message_keeps_code() {
        let resp = internal_error("DB_ERROR", GENERIC_INTERNAL_MSG);
        assert_eq!(
            resp.status(),
            actix_web::http::StatusCode::INTERNAL_SERVER_ERROR
        );
        let body = actix_web::body::to_bytes(resp.into_body())
            .await
            .expect("body bytes");
        let parsed: serde_json::Value = serde_json::from_slice(&body).expect("json body");
        assert_eq!(parsed["success"], false);
        assert_eq!(parsed["error"]["code"], "DB_ERROR");
        assert_eq!(parsed["error"]["message"], GENERIC_INTERNAL_MSG);
        // 泛化文案不得含 SQL/表结构细节痕迹
        let msg = parsed["error"]["message"].as_str().unwrap();
        assert!(!msg.contains("error") && !msg.contains("sql") && !msg.contains("constraint"));
    }
}
