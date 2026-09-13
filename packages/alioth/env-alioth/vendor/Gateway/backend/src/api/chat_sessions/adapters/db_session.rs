use async_trait::async_trait;
use serde_json::Value;
use sqlx::PgPool;
use std::collections::HashMap;

use crate::api::chat_sessions::ports::{ChatSession, SessionStorePort};

pub struct SqlxSessionAdapter {
    pool: PgPool,
}

impl SqlxSessionAdapter {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl SessionStorePort for SqlxSessionAdapter {
    async fn get_session(
        &self,
        session_id: i64,
        user_id: i64,
    ) -> Result<Option<ChatSession>, String> {
        let row = sqlx::query_as::<
            _,
            (
                i64,
                Option<String>,
                Option<Value>,
                Option<Value>,
                Option<Value>,
                Option<chrono::DateTime<chrono::Utc>>,
                Option<chrono::DateTime<chrono::Utc>>,
            ),
        >(
            r#"SELECT id, notice, context, agent_state, permissions, created_at, updated_at
               FROM isahl."zc_id_thre-ai_session"
               WHERE id = $1 AND created_by_id = $2 AND deleted_at IS NULL"#,
        )
        .bind(session_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| format!("DB error: {}", e))?;
        Ok(row.map(
            |(id, notice, context, agent_state, permissions, created_at, updated_at)| ChatSession {
                id,
                title: notice.unwrap_or_default(),
                context,
                agent_state,
                permissions,
                created_at: created_at.unwrap_or_else(chrono::Utc::now),
                updated_at: updated_at.unwrap_or_else(chrono::Utc::now),
            },
        ))
    }

    async fn create_session(
        &self,
        title: &str,
        context: Option<Value>,
        user_id: i64,
    ) -> Result<ChatSession, String> {
        let mut record = HashMap::new();
        record.insert("notice".to_string(), Value::String(title.to_string()));
        // created_by_id 显式写入（否则为 NULL——get_session 按 created_by_id=user_id
        // 校验 owner 会 SESSION_NOT_FOUND，WS 对话无法使用：实测 335424851373150）
        record.insert(
            "created_by_id".to_string(),
            Value::Number(serde_json::Number::from(user_id)),
        );
        if let Some(ctx) = &context {
            record.insert("context".to_string(), ctx.clone());
        }

        let permissions = match crate::ngac::resolve_user_permissions(&self.pool, user_id).await {
            Ok(perm) => {
                record.insert("permissions".to_string(), perm.clone());
                Some(perm)
            }
            Err(e) => {
                common::telemetry::warn!(
                    "Failed to resolve NGAC permissions for user {}: {}",
                    user_id,
                    e
                );
                None
            }
        };

        let result_map = crate::trigger_crud::insert_with_triggers(
            &self.pool,
            "zc_id_thre-ai_session",
            record,
            Some(user_id),
        )
        .await
        .map_err(|e| format!("Failed to create session: {}", e))?;

        let session_id = result_map.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
        let created_at = result_map
            .get("created_at")
            .and_then(|v| v.as_str())
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&chrono::Utc))
            .unwrap_or_else(chrono::Utc::now);
        let updated_at = result_map
            .get("updated_at")
            .and_then(|v| v.as_str())
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&chrono::Utc))
            .unwrap_or_else(chrono::Utc::now);

        Ok(ChatSession {
            id: session_id,
            title: title.to_string(),
            context,
            agent_state: None,
            permissions,
            created_at,
            updated_at,
        })
    }

    async fn update_session_timestamp(&self, session_id: i64, _user_id: i64) -> Result<(), String> {
        sqlx::query(r#"UPDATE isahl."zc_id_thre-ai_session" SET updated_at = NOW() WHERE id = $1 AND created_by_id = $2"#)
            .bind(session_id)
            .bind(_user_id)
            .execute(&self.pool)
            .await
            .map_err(|e| format!("DB error: {}", e))?;
        Ok(())
    }

    async fn update_session_title(
        &self,
        session_id: i64,
        user_id: i64,
        title: &str,
    ) -> Result<bool, String> {
        let rows_affected = sqlx::query(
            r#"UPDATE isahl."zc_id_thre-ai_session"
               SET notice = $1, updated_at = NOW()
               WHERE id = $2 AND created_by_id = $3 AND deleted_at IS NULL"#,
        )
        .bind(title)
        .bind(session_id)
        .bind(user_id)
        .execute(&self.pool)
        .await
        .map_err(|e| format!("DB error: {}", e))?
        .rows_affected();
        Ok(rows_affected > 0)
    }

    async fn update_session_context(
        &self,
        session_id: i64,
        user_id: i64,
        context: Value,
    ) -> Result<(), String> {
        sqlx::query(r#"UPDATE isahl."zc_id_thre-ai_session" SET context = $1, updated_at = NOW() WHERE id = $2 AND created_by_id = $3"#)
            .bind(context)
            .bind(session_id)
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(|e| format!("DB error: {}", e))?;
        Ok(())
    }

    async fn update_session_state(
        &self,
        session_id: i64,
        _user_id: i64,
        state_patch: Value,
    ) -> Result<(), String> {
        // COALESCE：agent_state 初始 NULL 时 jsonb || 结果仍为 NULL（PG 语义），
        // 首次写入会静默丢失——以 {} 垫底
        sqlx::query(
            r#"UPDATE isahl."zc_id_thre-ai_session"
               SET agent_state = COALESCE(agent_state, '{}'::jsonb) || $1
               WHERE id = $2 AND created_by_id = $3"#,
        )
        .bind(state_patch)
        .bind(session_id)
        .bind(_user_id)
        .execute(&self.pool)
        .await
        .map_err(|e| format!("Failed to save agent state: {}", e))?;
        Ok(())
    }

    async fn get_session_context(
        &self,
        session_id: i64,
        user_id: i64,
    ) -> Result<(Option<Value>, Option<Value>), String> {
        let row = sqlx::query_as::<_, (Option<Value>, Option<Value>)>(
            r#"SELECT context, agent_state FROM isahl."zc_id_thre-ai_session"
               WHERE id = $1 AND created_by_id = $2"#,
        )
        .bind(session_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| format!("DB error: {}", e))?;

        Ok(row.unwrap_or((None, None)))
    }

    async fn list_sessions(&self, user_id: i64) -> Result<Vec<ChatSession>, String> {
        let rows = sqlx::query_as::<
            _,
            (
                i64,
                Option<String>,
                Option<Value>,
                Option<Value>,
                Option<Value>,
                Option<chrono::DateTime<chrono::Utc>>,
                Option<chrono::DateTime<chrono::Utc>>,
            ),
        >(
            r#"SELECT id, notice, context, agent_state, permissions, created_at, updated_at
               FROM isahl."zc_id_thre-ai_session"
               WHERE created_by_id = $1 AND deleted_at IS NULL
               ORDER BY updated_at DESC"#,
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| format!("DB error: {}", e))?;

        Ok(rows
            .into_iter()
            .map(
                |(id, notice, context, agent_state, permissions, created_at, updated_at)| {
                    ChatSession {
                        id,
                        title: notice.unwrap_or_default(),
                        context,
                        agent_state,
                        permissions,
                        created_at: created_at.unwrap_or_else(chrono::Utc::now),
                        updated_at: updated_at.unwrap_or_else(chrono::Utc::now),
                    }
                },
            )
            .collect())
    }

    /// 会话软删 + 同事务级联（cascade-chat-session-delete D1/D2/D3）：
    /// ① 会话行软删（deleted_at + deleted_by_id）；
    /// ② 该会话消息软删（fk_thread 范围、deleted_at IS NULL，带 deleted_by_id）；
    /// ③ 无软删列的衍生行清理（chat_message_meta / chat_message_feedback——
    ///    沿用 soft_delete_last_assistant 的「清 meta」先例；先清衍生再软删消息）。
    /// 归属守卫在事务内先行：会话行 0 命中（非本人/不存在/已删）→ SESSION_NOT_FOUND
    /// 回滚，零级联写入（D4）。
    async fn delete_session(&self, session_id: i64, user_id: i64) -> Result<(), String> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| format!("DB error: {}", e))?;

        let rows_affected = sqlx::query(
            r#"UPDATE isahl."zc_id_thre-ai_session"
               SET deleted_at = NOW(), deleted_by_id = $2
               WHERE id = $1 AND created_by_id = $2 AND deleted_at IS NULL"#,
        )
        .bind(session_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("DB error: {}", e))?
        .rows_affected();

        if rows_affected == 0 {
            tx.rollback()
                .await
                .map_err(|e| format!("DB error: {}", e))?;
            return Err("SESSION_NOT_FOUND".to_string());
        }

        sqlx::query(
            r#"DELETE FROM isahl_auth.chat_message_meta
               WHERE msg_id IN (SELECT id FROM isahl."zc_id_msgs-chat_ai" WHERE fk_thread = $1)"#,
        )
        .bind(session_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("clear session message meta failed: {}", e))?;

        sqlx::query(
            r#"DELETE FROM isahl_auth.chat_message_feedback
               WHERE msg_id IN (SELECT id FROM isahl."zc_id_msgs-chat_ai" WHERE fk_thread = $1)"#,
        )
        .bind(session_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("clear session message feedback failed: {}", e))?;

        sqlx::query(
            r#"UPDATE isahl."zc_id_msgs-chat_ai"
               SET deleted_at = NOW(), deleted_by_id = $2
               WHERE fk_thread = $1 AND deleted_at IS NULL"#,
        )
        .bind(session_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("soft delete session messages failed: {}", e))?;

        tx.commit().await.map_err(|e| format!("DB error: {}", e))
    }
}

// ============================================================
// R3（D2.6）agent_state 有界 append——routing 决策环形保留最近 cap 条
// ============================================================

/// routing 决策环形上限（spec chat-ai-agent-state-bounded：最近 10 条）
pub(crate) const ROUTING_STATE_CAP: usize = 10;

/// 纯函数：数组追加 + 超 cap 截断（只留最近 cap 条，丢弃最旧）。
/// 独立出来便于单测 100 轮场景（append_bounded_state 的读写核心）。
pub(crate) fn push_bounded(entries: Vec<Value>, entry: Value, cap: usize) -> Vec<Value> {
    let mut out = entries;
    out.push(entry);
    if out.len() > cap {
        out.drain(..out.len() - cap);
    }
    out
}

/// bounded append 落库：agent_state.<key> 读-改-写（行锁事务防并发交错）——
/// key 缺省视为空数组；超 cap 截断保留最近 cap 条。routing 决策环形写入口
/// （agent_dispatch resolve 成功处）；last_result/turn_count 等覆盖式键仍走
/// SessionStorePort::update_session_state（COALESCE || 按 key 覆盖，不经过此函数）。
pub(crate) async fn append_bounded_state(
    pool: &PgPool,
    session_id: i64,
    key: &str,
    entry: Value,
    cap: usize,
) -> Result<(), String> {
    let mut tx = pool.begin().await.map_err(|e| format!("DB error: {}", e))?;
    let current: Option<Value> = sqlx::query_scalar(
        r#"SELECT COALESCE(agent_state, '{}'::jsonb) FROM isahl."zc_id_thre-ai_session"
           WHERE id = $1 AND deleted_at IS NULL
           FOR UPDATE"#,
    )
    .bind(session_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| format!("DB error: {}", e))?;

    let mut state = current.unwrap_or_else(|| serde_json::json!({}));
    if state.is_null() {
        // agent_state 列 NULL（未初始化的会话）→ 视为空对象
        state = serde_json::json!({});
    }
    let arr = state
        .get(key)
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let arr = push_bounded(arr, entry, cap);
    state[key] = Value::Array(arr);

    sqlx::query(
        r#"UPDATE isahl."zc_id_thre-ai_session"
           SET agent_state = $1, updated_at = NOW()
           WHERE id = $2 AND deleted_at IS NULL"#,
    )
    .bind(state)
    .bind(session_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| format!("DB error: {}", e))?;

    tx.commit().await.map_err(|e| format!("DB error: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// R3 场景：100 轮后 routing 数组长度 ≤ 10，且保留的是最近 10 条（有序）。
    #[test]
    fn push_bounded_ring_keeps_last_ten_of_hundred() {
        let mut entries: Vec<Value> = Vec::new();
        for i in 0..100 {
            entries = push_bounded(
                entries,
                serde_json::json!({ "n": i, "agent": "general" }),
                ROUTING_STATE_CAP,
            );
        }
        assert_eq!(entries.len(), ROUTING_STATE_CAP, "100 轮后 routing ≤10");
        assert_eq!(
            entries.first().and_then(|v| v.get("n")),
            Some(&serde_json::json!(90))
        );
        assert_eq!(
            entries.last().and_then(|v| v.get("n")),
            Some(&serde_json::json!(99))
        );
    }

    #[test]
    fn push_bounded_below_cap_no_truncation() {
        let mut entries = vec![serde_json::json!({"n": 0})];
        entries = push_bounded(entries, serde_json::json!({"n": 1}), ROUTING_STATE_CAP);
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn push_bounded_at_cap_drops_oldest() {
        let entries: Vec<Value> = (0..ROUTING_STATE_CAP as i64)
            .map(|i| serde_json::json!({ "n": i }))
            .collect();
        let entries = push_bounded(entries, serde_json::json!({"n": 10}), ROUTING_STATE_CAP);
        assert_eq!(entries.len(), ROUTING_STATE_CAP);
        assert_eq!(
            entries.first().and_then(|v| v.get("n")),
            Some(&serde_json::json!(1)),
            "恰满时追加丢最旧、保留新条目"
        );
        assert_eq!(
            entries.last().and_then(|v| v.get("n")),
            Some(&serde_json::json!(10))
        );
    }
}
