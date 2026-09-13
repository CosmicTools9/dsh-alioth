use async_trait::async_trait;
use sqlx::PgPool;
use std::sync::OnceLock;

use crate::api::chat_sessions::ports::AIContactPort;
use crate::i18n::I18nManagerRef;

pub const AI_ASSISTANT_CODE: &str = "llm-agent";

static AI_CONTACT_ID: OnceLock<Option<i64>> = OnceLock::new();

fn cached_ai_contact_id() -> Option<i64> {
    AI_CONTACT_ID.get().and_then(|opt| *opt)
}

pub struct DbAIContactAdapter {
    pool: PgPool,
    i18n: I18nManagerRef,
}

impl DbAIContactAdapter {
    pub fn new(pool: PgPool, i18n: I18nManagerRef) -> Self {
        Self { pool, i18n }
    }
}

#[async_trait]
impl AIContactPort for DbAIContactAdapter {
    async fn resolve_ai_contact_id(&self, locale: &i18n::Locale) -> Result<Option<i64>, String> {
        if let Some(id) = cached_ai_contact_id() {
            return Ok(Some(id));
        }

        let row = sqlx::query_scalar::<_, i64>(
            r#"SELECT id FROM isahl.zc_id_contact_infos
               WHERE code = $1 AND deleted_at IS NULL LIMIT 1"#,
        )
        .bind(AI_ASSISTANT_CODE)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| format!("DB error: {}", e))?;

        if let Some(id) = row {
            let _ = AI_CONTACT_ID.set(Some(id));
            return Ok(Some(id));
        }

        let i18n = self.i18n.read().await;
        let ai_name = i18n
            .get(locale, "chat.session.aiContactName")
            .unwrap_or("EmpAgent");

        let row = sqlx::query_scalar::<_, i64>(
            r#"INSERT INTO isahl."zc_id_info-isahl" (code, notice)
               VALUES ($1, $2)
               RETURNING id"#,
        )
        .bind(AI_ASSISTANT_CODE)
        .bind(ai_name)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| format!("Failed to create AI contact: {}", e))?;

        let _ = AI_CONTACT_ID.set(Some(row));
        Ok(Some(row))
    }

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
