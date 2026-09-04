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
    async fn get_last_user_message(
        &self,
        session_id: i64,
        ai_contact_id: Option<i64>,
    ) -> Result<Option<String>, String>;
}

#[async_trait]
pub trait LlmConfigPort: Send + Sync {
    async fn load_service(&self) -> Result<llm::LlmService, String>;
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
}

#[async_trait]
pub trait AIContactPort: Send + Sync {
    async fn resolve_ai_contact_id(&self, locale: &i18n::Locale) -> Result<Option<i64>, String>;
    async fn resolve_user_contact_id(&self, user_id: i64) -> Result<Option<i64>, String>;
}
