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
        sqlx::query(
            r#"UPDATE isahl."zc_id_thre-ai_session"
               SET agent_state = agent_state || $1
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

    async fn delete_session(&self, session_id: i64, user_id: i64) -> Result<(), String> {
        let rows_affected = sqlx::query(
            r#"UPDATE isahl."zc_id_thre-ai_session"
               SET deleted_at = NOW()
               WHERE id = $1 AND created_by_id = $2 AND deleted_at IS NULL"#,
        )
        .bind(session_id)
        .bind(user_id)
        .execute(&self.pool)
        .await
        .map_err(|e| format!("DB error: {}", e))?
        .rows_affected();

        if rows_affected == 0 {
            return Err("SESSION_NOT_FOUND".to_string());
        }
        Ok(())
    }
}
