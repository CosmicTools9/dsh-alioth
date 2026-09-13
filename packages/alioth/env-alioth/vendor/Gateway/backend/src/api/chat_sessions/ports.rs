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
    async fn add_message(
        &self,
        session_id: i64,
        content: &str,
        sender_addr: Option<i64>,
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
    async fn get_last_user_message_row(
        &self,
        session_id: i64,
        ai_contact_id: Option<i64>,
    ) -> Result<Option<MessageRow>, String>;
    /// 写消息级 meta（isahl_auth.chat_message_meta，upsert by msg_id）。
    /// 全部字段 Option：None 字段不覆盖既有值（attachments 语义同）。
    async fn save_message_meta(
        &self,
        msg_id: i64,
        session_id: i64,
        agent_code: &str,
        structured: Option<&Value>,
        usage: Option<&Value>,
        attachments: Option<&Value>,
        knowledge_refs: Option<&Value>,
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
    /// 加载用户级 AI memory（add-agent-pool-user-memory）。
    /// 默认实现返回空对象（未接入池的 adapter 兼容）。
    async fn load_user_memory(&self, _user_id: i64) -> Result<serde_json::Value, String> {
        Ok(serde_json::json!({}))
    }
    /// 记忆沉淀同步（D2.14）：UserMemoryStore.save 落库后同步池内
    /// (user_id, agent_code) 实例 memory。默认 no-op（未接入池的 adapter 兼容）。
    async fn sync_user_memory(
        &self,
        _user_id: i64,
        _agent_code: &str,
        _memory: serde_json::Value,
    ) -> Result<(), String> {
        Ok(())
    }
}

#[async_trait]
pub trait AIContactPort: Send + Sync {
    async fn resolve_ai_contact_id(&self, locale: &i18n::Locale) -> Result<Option<i64>, String>;
    async fn resolve_user_contact_id(&self, user_id: i64) -> Result<Option<i64>, String>;
}
