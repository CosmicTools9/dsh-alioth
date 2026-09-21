//! AI 联系人端口适配：用户发送方地址解析。
//!
//! 主体化后**不再**有共享 AI 联系人（`llm-agent`）：每个智能体主体有自己的联系方式
//! （`agent-<agent_code>`，见 `memory_scope::resolve_subject_contact_id`），AI 回复的
//! 发送方 = 该主体的联系方式 id。故本适配器只剩「用户 → 发送方地址」一问。
//!
//! 历史行的识别（旧消息发送方仍是 `llm-agent`）由 `memory_scope::LEGACY_SHARED_AI_CONTACT_CODE`
//! 在**只读**判定面处理（角色判定 / 用户消息定位 / regenerate 定位），不在此新建行。

use async_trait::async_trait;
use sqlx::PgPool;

use crate::api::chat_sessions::ports::AIContactPort;

pub struct DbAIContactAdapter {
    pool: PgPool,
}

impl DbAIContactAdapter {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl AIContactPort for DbAIContactAdapter {
    /// 用户发送方地址（`zc_id_contact_infos` id）：账号 1:1 绑定实体 → 默认联系人 → 首选联系方式。
    /// 复用 framework_contacts 的联系链唯一实现；未绑定 / 实体无联系人 → None（发送方留空）。
    async fn resolve_user_contact_id(&self, user_id: i64) -> Result<Option<i64>, String> {
        Ok(
            framework_contacts::ContactsService::resolve_user_contact(&self.pool, user_id)
                .await?
                .and_then(|r| r.info_id),
        )
    }
}
