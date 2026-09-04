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
    db_llm_config::DbLlmConfigAdapter, db_message::SqlxMessageAdapter,
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

/// 缓存条目：状态 + 写入时间戳（TTL 判定）
#[derive(Debug, Clone)]
struct GenerationEntry {
    status: GenerationStatus,
    created_at: std::time::Instant,
}

impl GenerationEntry {
    fn new(status: GenerationStatus) -> Self {
        Self {
            status,
            created_at: std::time::Instant::now(),
        }
    }
    fn expired(&self) -> bool {
        self.created_at.elapsed() > GENERATION_ENTRY_TTL
    }
}

/// 全局生成状态缓存：session_id → GenerationStatus
static GENERATION_CACHE: std::sync::LazyLock<Arc<RwLock<HashMap<i64, GenerationEntry>>>> =
    std::sync::LazyLock::new(|| Arc::new(RwLock::new(HashMap::new())));

fn generation_cache() -> &'static Arc<RwLock<HashMap<i64, GenerationEntry>>> {
    &GENERATION_CACHE
}

/// 写入新条目并顺带 GC 过期项（防内存膨胀；写路径单点调用）。
async fn cache_insert(session_id: i64, status: GenerationStatus) {
    let cache = generation_cache().clone();
    let mut guard = cache.write().await;
    guard.retain(|_, entry| !entry.expired());
    guard.insert(session_id, GenerationEntry::new(status));
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
    #[serde(with = "common::serde_zuid::opt")]
    pub offset: Option<i64>,
    #[serde(with = "common::serde_zuid::opt")]
    pub limit: Option<i64>,
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

fn not_implemented(code: &str, message: &str) -> HttpResponse {
    error(code, message, actix_web::http::StatusCode::NOT_IMPLEMENTED)
}

fn internal_error(code: &str, message: &str) -> HttpResponse {
    error(
        code,
        message,
        actix_web::http::StatusCode::INTERNAL_SERVER_ERROR,
    )
}

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
            Ok(internal_error(
                "DB_ERROR",
                &format!("Failed to create session: {}", e),
            ))
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
            Ok(internal_error("DB_ERROR", &e))
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
            Ok(internal_error("DB_ERROR", &e))
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
        .add_message(session_id, &req.content, req.context.clone(), user_id)
        .await
    {
        Ok(response) => Ok(success(response)),
        Err(e) if e == "SESSION_NOT_FOUND" => Ok(not_found(
            "SESSION_NOT_FOUND",
            &format!("Chat session {} not found", session_id),
        )),
        Err(e) => {
            common::telemetry::error!("Failed to add chat message: {}", e);
            Ok(internal_error(
                "DB_ERROR",
                &format!("Failed to add message: {}", e),
            ))
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
                        agent_code: if is_assistant { "general" } else { "" }.to_string(),
                        structured: None,
                        requires_input: false,
                        suggested_actions: vec![],
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
            Ok(internal_error("DB_ERROR", &e))
        }
    }
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
    let locale_str = locale.to_string();
    let session_id = path.into_inner();
    let user_id = extract_user_id(&req)?;

    // Mark as processing
    cache_insert(session_id, GenerationStatus::Processing).await;

    let pool_clone = pool.get_ref().clone();
    let i18n_clone = i18n_manager.get_ref().clone();

    // Spawn background task — do NOT block the HTTP worker
    tokio::spawn(async move {
        let orchestrator = build_orchestrator(&pool_clone, i18n_clone);

        let input = TurnInput {
            session_id,
            user_id,
            locale: locale_str,
            model: body.as_ref().and_then(|b| b.model.clone()),
        };

        let result = orchestrator.process_turn(input, None).await;

        match result {
            Ok(turn_result) => {
                cache_insert(session_id, GenerationStatus::Completed(turn_result.message)).await;
            }
            Err(e) => {
                common::telemetry::error!(
                    "Background generation failed for session {}: {}",
                    session_id,
                    e
                );
                cache_insert(session_id, GenerationStatus::Failed(e)).await;
            }
        }
    });

    // Return 202 Accepted immediately
    Ok(accepted(serde_json::json!({
        "session_id": session_id.to_string(),
        "status": "processing"
    })))
}

/// GET /api/chat-sessions/{id}/response — Poll for async generation result
pub async fn get_response_status(
    _pool: web::Data<PgPool>,
    _i18n_manager: web::Data<I18nManagerRef>,
    req: HttpRequest,
    path: web::Path<i64>,
) -> Result<HttpResponse> {
    let session_id = path.into_inner();
    let _user_id = extract_user_id(&req)?;

    let cache = generation_cache().clone();
    let guard = cache.read().await;

    match guard.get(&session_id) {
        Some(GenerationEntry {
            status: GenerationStatus::Completed(msg),
            ..
        }) => {
            let response = msg.clone();
            drop(guard);
            // Clean up cache entry after successful delivery
            let mut write_guard = cache.write().await;
            write_guard.remove(&session_id);
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

/// GET /api/chat-sessions/agents
/// List all available agents（合并数据库配置）
pub async fn list_agents(
    pool: web::Data<PgPool>,
    i18n_manager: web::Data<I18nManagerRef>,
) -> Result<HttpResponse> {
    let orchestrator = build_orchestrator(pool.get_ref(), i18n_manager.get_ref().clone());

    match orchestrator.list_agents().await {
        Ok(agents) => Ok(success(agents)),
        Err(e) => {
            common::telemetry::error!("Failed to list agents: {}", e);
            Ok(internal_error("AGENT_ERROR", &e))
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
            Ok(internal_error("LLM_CONFIG_ERROR", &e))
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
            Ok(internal_error(
                "DB_ERROR",
                &format!("Failed to switch agent: {}", e),
            ))
        }
    }
}

/// POST /api/chat-sessions/{id}/execute-action
/// Direct action execution is not yet implemented; returns 501.
pub async fn execute_action(
    _pool: web::Data<PgPool>,
    _i18n_manager: web::Data<I18nManagerRef>,
    _req: HttpRequest,
    _path: web::Path<i64>,
    _body: web::Json<ExecuteActionRequest>,
) -> Result<HttpResponse> {
    Ok(not_implemented(
        "NOT_IMPLEMENTED",
        "Direct action execution is not yet implemented. Use the message + generate-response flow instead.",
    ))
}

// ============================================
// Route Configuration
// ============================================

pub fn configure_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/chat-sessions")
            .route("", web::post().to(create_session))
            .route("", web::get().to(list_sessions))
            .route("/agents", web::get().to(list_agents))
            .route("/model-options", web::get().to(model_options))
            .route("/{id}", web::delete().to(delete_session))
            .route("/{id}/messages", web::post().to(add_message))
            .route("/{id}/messages", web::get().to(get_messages))
            .route("/{id}/generate-response", web::post().to(generate_response))
            .route("/{id}/response", web::get().to(get_response_status))
            .route("/{id}/switch-agent", web::post().to(switch_agent))
            .route("/{id}/execute-action", web::post().to(execute_action))
            .route("/{id}/ws", web::get().to(ws_handler::ws_connect)),
    );
}
