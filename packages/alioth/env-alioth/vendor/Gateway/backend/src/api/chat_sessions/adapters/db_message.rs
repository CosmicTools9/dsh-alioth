use async_trait::async_trait;
use serde_json::Value;
use sqlx::PgPool;

use crate::api::chat_sessions::adapters::db_message_meta;
use crate::api::chat_sessions::ports::{MessageRow, MessageStorePort};

/// get_history/get_messages/get_last_user_message_row 共用 meta JOIN 列片段
/// （列序与 MessageRow FromRow 字段名一致）。
const META_SELECT: &str = r#"
    cm.agent_code AS agent_code,
    cm.structured AS structured,
    cm.usage AS usage,
    cm.knowledge_refs AS knowledge_refs,
    cm.attachments AS attachments"#;

const META_JOIN: &str = r#"LEFT JOIN isahl_auth.chat_message_meta cm ON cm.msg_id = m.id"#;

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
        // 坐标三元组（§6.12 声明即必须）：值经 ontology_binding 解析 code→ZUID，禁硬编码 ZUID
        let (dk_scene, dk_factor, dk_function) =
            ontology_binding::resolve(&self.pool, ("JE", "FRE", "↓_GG"))
                .await
                .map_err(|e| format!("Failed to resolve dk coords: {}", e))?;
        let row = sqlx::query_as::<_, MessageRow>(
            r#"INSERT INTO isahl."zc_id_msgs-chat_ai"
                   (fk_thread, content, "fk_sender-addr", dk_scene, dk_factor, dk_function)
               VALUES ($1, $2, $3, $4, $5, $6)
               RETURNING id, content, "fk_sender-addr", created_at"#,
        )
        .bind(session_id)
        .bind(content)
        .bind(sender_addr)
        .bind(dk_scene)
        .bind(dk_factor)
        .bind(dk_function)
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
        let sql = format!(
            r#"SELECT m.id, m.content, m."fk_sender-addr", m.created_at,
                      {}
               FROM isahl."zc_id_msgs-chat_ai" m
               JOIN isahl."zc_id_thre-ai_session" s ON s.id = m.fk_thread AND s.deleted_at IS NULL
               {}
               WHERE m.fk_thread = $1 AND s.created_by_id = $2 AND m.deleted_at IS NULL
               ORDER BY m.created_at DESC
               LIMIT $3"#,
            META_SELECT, META_JOIN
        );
        let mut rows = sqlx::query_as::<_, MessageRow>(sqlx::AssertSqlSafe(sql.as_str()))
            .bind(session_id)
            .bind(user_id)
            .bind(limit)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| format!("Failed to load history: {}", e))?;
        // D2.11：返回最近 limit 条（ASC 序）——窗口语义取最新 N 而非最老 N
        rows.reverse();
        Ok(rows)
    }

    async fn get_last_user_message_row(
        &self,
        session_id: i64,
        ai_contact_id: Option<i64>,
    ) -> Result<Option<MessageRow>, String> {
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

        let sql = format!(
            r#"SELECT m.id, m.content, m."fk_sender-addr", m.created_at,
                      {}
               FROM isahl."zc_id_msgs-chat_ai" m
               {}
               WHERE m.fk_thread = $1 AND m."fk_sender-addr" IS DISTINCT FROM $2
                 AND m.deleted_at IS NULL
               ORDER BY m.created_at DESC LIMIT 1"#,
            META_SELECT, META_JOIN
        );
        let row = sqlx::query_as::<_, MessageRow>(sqlx::AssertSqlSafe(sql.as_str()))
            .bind(session_id)
            .bind(ai_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| format!("DB error: {}", e))?;

        Ok(row)
    }

    async fn get_messages(
        &self,
        session_id: i64,
        user_id: i64,
        offset: i64,
        limit: i64,
    ) -> Result<Vec<MessageRow>, String> {
        let sql = format!(
            r#"SELECT m.id, m.content, m."fk_sender-addr", m.created_at,
                      {}
               FROM isahl."zc_id_msgs-chat_ai" m
               JOIN isahl."zc_id_thre-ai_session" s ON s.id = m.fk_thread AND s.deleted_at IS NULL
               {}
               WHERE m.fk_thread = $1 AND s.created_by_id = $2 AND m.deleted_at IS NULL
               ORDER BY m.created_at ASC
               OFFSET $3 LIMIT $4"#,
            META_SELECT, META_JOIN
        );
        let rows = sqlx::query_as::<_, MessageRow>(sqlx::AssertSqlSafe(sql.as_str()))
            .bind(session_id)
            .bind(user_id)
            .bind(offset)
            .bind(limit)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| format!("Failed to load messages: {}", e))?;

        Ok(rows)
    }

    async fn save_message_meta(
        &self,
        msg_id: i64,
        session_id: i64,
        agent_code: &str,
        structured: Option<&Value>,
        usage: Option<&Value>,
        attachments: Option<&Value>,
        knowledge_refs: Option<&Value>,
    ) -> Result<(), String> {
        db_message_meta::save_meta(
            &self.pool,
            msg_id,
            session_id,
            agent_code,
            structured,
            usage,
            attachments,
            knowledge_refs,
        )
        .await
    }

    async fn set_message_feedback(
        &self,
        msg_id: i64,
        user_id: i64,
        rating: &str,
        comment: Option<&str>,
    ) -> Result<Option<String>, String> {
        db_message_meta::set_feedback(&self.pool, msg_id, user_id, rating, comment).await
    }

    async fn message_belongs_to_user(&self, msg_id: i64, user_id: i64) -> Result<bool, String> {
        db_message_meta::belongs_to_user(&self.pool, msg_id, user_id).await
    }

    async fn soft_delete_message(&self, msg_id: i64) -> Result<bool, String> {
        db_message_meta::soft_delete(&self.pool, msg_id).await
    }
}
