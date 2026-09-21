use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::Value;

use ai_agent::agents::AgentConfig;

#[derive(Debug)]
pub struct ChatSession {
    pub id: i64,
    pub title: String,
    pub context: Option<Value>,
    pub agent_state: Option<Value>,
    pub permissions: Option<Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(sqlx::FromRow, Debug)]
pub struct MessageRow {
    pub id: i64,
    pub content: Option<String>,
    #[sqlx(rename = "fk_sender-addr")]
    pub fk_sender_addr: Option<i64>,
    pub created_at: DateTime<Utc>,
    /// meta 派生列（LEFT JOIN isahl_auth.chat_message_meta 恢复；无 JOIN/无 meta
    /// 行 → None）。#[sqlx(default)] 令 add_message 等无 JOIN 查询免补列。
    #[sqlx(default)]
    pub agent_code: Option<String>,
    #[sqlx(default)]
    pub structured: Option<Value>,
    #[sqlx(default)]
    pub usage: Option<Value>,
    #[sqlx(default)]
    pub knowledge_refs: Option<Value>,
    #[sqlx(default)]
    pub attachments: Option<Value>,
    /// 工具执行记录（批 ④ add-chat-ai-runtime-context）：E7 落库的
    /// `chat_message_meta.tool_calls`，供历史重建时回灌注记。
    #[sqlx(default)]
    pub tool_calls: Option<Value>,
}

#[async_trait]
pub trait SessionStorePort: Send + Sync {
    async fn get_session(
        &self,
        session_id: i64,
        user_id: i64,
    ) -> Result<Option<ChatSession>, String>;
    async fn create_session(
        &self,
        title: &str,
        context: Option<Value>,
        user_id: i64,
    ) -> Result<ChatSession, String>;
    async fn update_session_timestamp(&self, session_id: i64, _user_id: i64) -> Result<(), String>;
    /// 重命名会话（notice 列 = 会话标题；D2.8 PATCH /{id} + 自动标题共用）。
    /// Ok(true) = 更新成功；Ok(false) = 会话不存在/非本人。
    async fn update_session_title(
        &self,
        session_id: i64,
        user_id: i64,
        title: &str,
    ) -> Result<bool, String>;
    /// Replace the session's page_context snapshot wholesale (full-replacement
    /// semantics; called when a message carries fresh client context).
    async fn update_session_context(
        &self,
        session_id: i64,
        user_id: i64,
        context: Value,
    ) -> Result<(), String>;
    async fn update_session_state(
        &self,
        session_id: i64,
        _user_id: i64,
        state_patch: Value,
    ) -> Result<(), String>;
    async fn get_session_context(
        &self,
        session_id: i64,
        user_id: i64,
    ) -> Result<(Option<Value>, Option<Value>), String>;
    async fn list_sessions(&self, user_id: i64) -> Result<Vec<ChatSession>, String>;
    async fn delete_session(&self, session_id: i64, user_id: i64) -> Result<(), String>;
}

#[async_trait]
pub trait MessageStorePort: Send + Sync {
    /// `recipients` = 参与方联系方式 id（`zc_id_message_rr_recipients.ref_right`），
    /// 与消息同事务写入（refactor-chat-ai-subject-identity-memory D-2）。
    async fn add_message(
        &self,
        session_id: i64,
        content: &str,
        sender_addr: Option<i64>,
        recipients: &[i64],
    ) -> Result<MessageRow, String>;
    async fn get_history(
        &self,
        session_id: i64,
        user_id: i64,
        limit: i64,
    ) -> Result<Vec<MessageRow>, String>;
    async fn get_messages(
        &self,
        session_id: i64,
        user_id: i64,
        offset: i64,
        limit: i64,
    ) -> Result<Vec<MessageRow>, String>;
    /// 最新用户消息完整行（含 meta JOIN 列——本轮附件/知识引用由 process_turn
    /// 从该行读取；取代旧 get_last_user_message 的 content-only 查询）。
    /// 用户侧判定按「发送方不是智能体侧联系方式」——智能体侧 = `zc_id_contact_infos.code`
    /// 以 `agent-` 前缀者（refactor-chat-ai-subject-identity-memory D-3：无共享兜底联系人）。
    async fn get_last_user_message_row(
        &self,
        session_id: i64,
    ) -> Result<Option<MessageRow>, String>;
    /// 写消息级 meta（isahl_auth.chat_message_meta，upsert by msg_id）。
    /// 全部字段 Option：None 字段不覆盖既有值（attachments 语义同）。
    /// `tool_calls` = 本轮工具调用记录（E7；含截断输出）。读取侧经
    /// `META_SELECT` 回读后，由历史重建渲染**有界注记**（单调用 ≤300 / 单条 ≤600
    /// 字符，批 ④ `tool-trace-cross-turn`）——非原始输出整体回灌。详见
    /// `openspec/specs/chat-ai/spec.md` 的 `chat-ai-tool-result-persistence`。
    async fn save_message_meta(
        &self,
        msg_id: i64,
        session_id: i64,
        agent_code: &str,
        structured: Option<&Value>,
        usage: Option<&Value>,
        attachments: Option<&Value>,
        knowledge_refs: Option<&Value>,
        tool_calls: Option<&Value>,
    ) -> Result<(), String>;
    /// 用户消息反馈（isahl_auth.chat_message_feedback，toggle upsert）：
    /// 同 (msg_id, user_id) 同 rating 再点 → 删除（返回 None）；不同 rating/新增 → upsert。
    async fn set_message_feedback(
        &self,
        msg_id: i64,
        user_id: i64,
        rating: &str,
        comment: Option<&str>,
    ) -> Result<Option<String>, String>;
    /// 校验消息归属：msg 存在且所属 session 的 created_by_id == user_id。
    async fn message_belongs_to_user(&self, msg_id: i64, user_id: i64) -> Result<bool, String>;
    /// 软删消息（deleted_at；meta 保留但查询侧过滤——协调者契约：查询过滤，
    /// 不清 meta）。返回是否实际删除（不存在/已删 → false）。
    async fn soft_delete_message(&self, msg_id: i64) -> Result<bool, String>;
}

#[async_trait]
pub trait LlmConfigPort: Send + Sync {
    async fn load_service(&self) -> Result<llm::LlmService, String>;
    /// 历史窗口（D2.11）：llm settings.max_history_messages，缺省 50（无 DDL）。
    async fn max_history_messages(&self) -> i64 {
        50
    }
}

#[async_trait]
pub trait AgentDispatchPort: Send + Sync {
    async fn resolve_agent(
        &self,
        session_id: i64,
        user_message: &str,
        page_context: Option<Value>,
        history: &[(String, String)],
        locale: &str,
        llm: &llm::LlmService,
    ) -> Result<String, String>;
    async fn get_agent_config(&self, code: &str) -> Result<AgentConfig, String>;
    async fn agent_exists(&self, code: &str) -> bool;
    async fn list_agent_configs(&self) -> Result<Vec<AgentConfig>, String>;
}

#[async_trait]
pub trait AIContactPort: Send + Sync {
    /// 用户发送方地址（`zc_id_contact_infos` id）。
    /// 主体化后不再有共享 AI 联系人解析（见 `memory_scope`：每主体独立联系方式）。
    async fn resolve_user_contact_id(&self, user_id: i64) -> Result<Option<i64>, String>;
}
