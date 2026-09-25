//! 消息级 meta / 反馈存储（isahl_auth.chat_message_meta / chat_message_feedback）。
//!
//! 消息正文与发送方在冻结的 isahl."zc_id_msgs-chat_ai"（零 DDL）；衍生数据
//! （agent_code/structured/usage/attachments/knowledge_refs）与用户反馈落
//! isahl_auth 配套工程 schema（019_chat_ai_meta.sql，fix-chat-ai-feature-gaps D2.5）。
//! 本文件只含 SQL；端口实现见 db_message.rs 的 SqlxMessageAdapter 委托。

use sqlx::PgPool;

use super::super::ports::MessageMetaFields;

/// upsert 消息 meta（fix-chat-ai-feature-gaps D2.12）。
///
/// 字段语义见 [`MessageMetaFields`]（None 字段不覆盖既有值；`agent_code`
/// 空串表示用户消息，与表 DEFAULT 同语义）。
pub async fn save_meta(
    pool: &PgPool,
    msg_id: i64,
    session_id: i64,
    fields: MessageMetaFields<'_>,
) -> Result<(), String> {
    sqlx::query(
        r#"INSERT INTO isahl_auth.chat_message_meta
               (msg_id, session_id, agent_code, structured, usage, attachments, knowledge_refs, tool_calls)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
           ON CONFLICT (msg_id) DO UPDATE SET
               session_id = EXCLUDED.session_id,
               agent_code = CASE WHEN EXCLUDED.agent_code <> '' THEN EXCLUDED.agent_code
                                 ELSE isahl_auth.chat_message_meta.agent_code END,
               structured = COALESCE(EXCLUDED.structured, isahl_auth.chat_message_meta.structured),
               usage = COALESCE(EXCLUDED.usage, isahl_auth.chat_message_meta.usage),
               attachments = COALESCE(EXCLUDED.attachments, isahl_auth.chat_message_meta.attachments),
               knowledge_refs = COALESCE(EXCLUDED.knowledge_refs, isahl_auth.chat_message_meta.knowledge_refs),
               tool_calls = COALESCE(EXCLUDED.tool_calls, isahl_auth.chat_message_meta.tool_calls)"#,
    )
    .bind(msg_id)
    .bind(session_id)
    .bind(fields.agent_code)
    .bind(fields.structured)
    .bind(fields.usage)
    .bind(fields.attachments)
    .bind(fields.knowledge_refs)
    .bind(fields.tool_calls)
    .execute(pool)
    .await
    .map_err(|e| format!("save message meta failed: {}", e))?;
    Ok(())
}

/// 消息反馈 upsert/toggle（fix-chat-ai-feature-gaps D2.16）。
///
/// 返回落库后的 rating（Some("up"|"down")）；toggle 命中（同 rating 再点）→
/// 删除并返回 None。owner 校验由调用方先做 message_belongs_to_user。
pub async fn set_feedback(
    pool: &PgPool,
    msg_id: i64,
    user_id: i64,
    rating: &str,
    comment: Option<&str>,
) -> Result<Option<String>, String> {
    let existing: Option<(String,)> = sqlx::query_as(
        r#"SELECT rating FROM isahl_auth.chat_message_feedback
           WHERE msg_id = $1 AND user_id = $2"#,
    )
    .bind(msg_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("feedback read failed: {}", e))?;

    match existing {
        Some((cur,)) if cur == rating => {
            // toggle 语义：同 rating 再点 → 撤销
            sqlx::query(
                r#"DELETE FROM isahl_auth.chat_message_feedback
                   WHERE msg_id = $1 AND user_id = $2"#,
            )
            .bind(msg_id)
            .bind(user_id)
            .execute(pool)
            .await
            .map_err(|e| format!("feedback delete failed: {}", e))?;
            Ok(None)
        }
        _ => {
            sqlx::query(
                r#"INSERT INTO isahl_auth.chat_message_feedback
                       (msg_id, user_id, rating, comment)
                   VALUES ($1, $2, $3, $4)
                   ON CONFLICT (msg_id, user_id) DO UPDATE SET
                       rating = EXCLUDED.rating,
                       comment = EXCLUDED.comment,
                       updated_at = NOW()"#,
            )
            .bind(msg_id)
            .bind(user_id)
            .bind(rating)
            .bind(comment)
            .execute(pool)
            .await
            .map_err(|e| format!("feedback upsert failed: {}", e))?;
            Ok(Some(rating.to_string()))
        }
    }
}

/// 消息归属校验：msg 存在、未被删，且所属 session 的 created_by_id == user_id。
pub async fn belongs_to_user(pool: &PgPool, msg_id: i64, user_id: i64) -> Result<bool, String> {
    let row: Option<(i64,)> = sqlx::query_as(
        r#"SELECT m.id
           FROM isahl."zc_id_msgs-chat_ai" m
           JOIN isahl."zc_id_thre-ai_session" s ON s.id = m.fk_thread AND s.deleted_at IS NULL
           WHERE m.id = $1 AND m.deleted_at IS NULL AND s.created_by_id = $2"#,
    )
    .bind(msg_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("message owner check failed: {}", e))?;
    Ok(row.is_some())
}

/// 软删消息（deleted_at）。meta 保留——查询侧（get_messages/get_history/
/// get_last_user_message_row）统一 m.deleted_at IS NULL 过滤，不清 meta。
/// 返回是否实际删除（不存在/已删 → false）。
pub async fn soft_delete(pool: &PgPool, msg_id: i64) -> Result<bool, String> {
    let rows_affected = sqlx::query(
        r#"UPDATE isahl."zc_id_msgs-chat_ai"
           SET deleted_at = NOW()
           WHERE id = $1 AND deleted_at IS NULL"#,
    )
    .bind(msg_id)
    .execute(pool)
    .await
    .map_err(|e| format!("soft delete message failed: {}", e))?
    .rows_affected();
    Ok(rows_affected > 0)
}

/// regenerate 用：软删某 session 最后一条 assistant 消息并清其 meta 行。
/// 返回被删消息 id（无 assistant 消息 → None）。
///
/// assistant 侧判定 = 发送方为智能体侧联系方式（`agent-<code>` 前缀 ∪ 历史 `llm-agent`），
/// 与 `memory_scope::agent_contact_id_set` 同口径（refactor-chat-ai-subject-identity-memory E1）。
pub async fn soft_delete_last_assistant(
    pool: &PgPool,
    session_id: i64,
) -> Result<Option<i64>, String> {
    let msg_id: Option<i64> = sqlx::query_scalar(
        r#"SELECT m.id FROM isahl."zc_id_msgs-chat_ai" m
           WHERE m.fk_thread = $1
             AND m.deleted_at IS NULL
             AND EXISTS (
                 SELECT 1 FROM isahl.zc_id_contact_infos ci
                  WHERE ci.id = m."fk_sender-addr"
                    AND ci.deleted_at IS NULL
                    AND (ci.code LIKE $2 OR ci.code = $3)
             )
           ORDER BY m.created_at DESC, m.id DESC LIMIT 1"#,
    )
    .bind(session_id)
    .bind(super::super::memory_scope::subject_contact_code_prefix_like())
    .bind(super::super::memory_scope::LEGACY_SHARED_AI_CONTACT_CODE)
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("last assistant lookup failed: {}", e))?;

    if let Some(id) = msg_id {
        sqlx::query(r#"UPDATE isahl."zc_id_msgs-chat_ai" SET deleted_at = NOW() WHERE id = $1"#)
            .bind(id)
            .execute(pool)
            .await
            .map_err(|e| format!("soft delete assistant message failed: {}", e))?;
        // 清 meta（软删消息不再回显；FK 级联仅覆盖物理删，此处置显式清理）
        sqlx::query("DELETE FROM isahl_auth.chat_message_meta WHERE msg_id = $1")
            .bind(id)
            .execute(pool)
            .await
            .map_err(|e| format!("clear message meta failed: {}", e))?;
    }
    Ok(msg_id)
}
