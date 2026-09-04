use async_trait::async_trait;
use sqlx::PgPool;

use crate::api::chat_sessions::ports::{MessageRow, MessageStorePort};

pub struct SqlxMessageAdapter {
    pool: PgPool,
}

impl SqlxMessageAdapter {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl MessageStorePort for SqlxMessageAdapter {
    async fn add_message(
        &self,
        session_id: i64,
        content: &str,
        sender_addr: Option<i64>,
    ) -> Result<MessageRow, String> {
        let row = sqlx::query_as::<_, MessageRow>(
            r#"INSERT INTO isahl."zc_id_msgs-chat_ai"
                   (fk_thread, content, "fk_sender-addr")
               VALUES ($1, $2, $3)
               RETURNING id, content, "fk_sender-addr", created_at"#,
        )
        .bind(session_id)
        .bind(content)
        .bind(sender_addr)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| format!("Failed to add message: {}", e))?;

        Ok(row)
    }

    async fn get_history(
        &self,
        session_id: i64,
        user_id: i64,
        limit: i64,
    ) -> Result<Vec<MessageRow>, String> {
        let rows = sqlx::query_as::<_, MessageRow>(
            r#"SELECT m.id, m.content, m."fk_sender-addr", m.created_at
               FROM isahl."zc_id_msgs-chat_ai" m
               JOIN isahl."zc_id_thre-ai_session" s ON s.id = m.fk_thread AND s.deleted_at IS NULL
               WHERE m.fk_thread = $1 AND s.created_by_id = $2
               ORDER BY m.created_at ASC
               LIMIT $3"#,
        )
        .bind(session_id)
        .bind(user_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| format!("Failed to load history: {}", e))?;

        Ok(rows)
    }

    async fn get_last_user_message(
        &self,
        session_id: i64,
        ai_contact_id: Option<i64>,
    ) -> Result<Option<String>, String> {
        let ai_id = match ai_contact_id {
            Some(id) => id,
            None => {
                // 回退到通过 code 查询
                let id = sqlx::query_scalar::<_, i64>(
                    r#"SELECT id FROM isahl.zc_id_contact_infos
                       WHERE code = $1 AND deleted_at IS NULL LIMIT 1"#,
                )
                .bind(super::db_ai_contact::AI_ASSISTANT_CODE)
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| format!("DB error: {}", e))?;
                match id {
                    Some(i) => i,
                    None => return Ok(None),
                }
            }
        };

        let row = sqlx::query_as::<_, (String,)>(
            r#"SELECT content FROM isahl."zc_id_msgs-chat_ai"
               WHERE fk_thread = $1 AND "fk_sender-addr" IS DISTINCT FROM $2
               ORDER BY created_at DESC LIMIT 1"#,
        )
        .bind(session_id)
        .bind(ai_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| format!("DB error: {}", e))?;

        Ok(row.map(|(content,)| content))
    }

    async fn get_messages(
        &self,
        session_id: i64,
        user_id: i64,
        offset: i64,
        limit: i64,
    ) -> Result<Vec<MessageRow>, String> {
        let rows = sqlx::query_as::<_, MessageRow>(
            r#"SELECT m.id, m.content, m."fk_sender-addr", m.created_at
               FROM isahl."zc_id_msgs-chat_ai" m
               JOIN isahl."zc_id_thre-ai_session" s ON s.id = m.fk_thread AND s.deleted_at IS NULL
               WHERE m.fk_thread = $1 AND s.created_by_id = $2
               ORDER BY m.created_at ASC
               OFFSET $3 LIMIT $4"#,
        )
        .bind(session_id)
        .bind(user_id)
        .bind(offset)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| format!("Failed to load messages: {}", e))?;

        Ok(rows)
    }
}
