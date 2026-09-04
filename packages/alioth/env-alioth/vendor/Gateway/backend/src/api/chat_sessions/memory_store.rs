//! 用户级 AI memory 存储（add-agent-pool-user-memory）。
//!
//! 读写 `isahl_auth.gateway_user_memory`：用户长期记忆（跨 session），
//! 由 AgentPool 实例懒加载、prompt 注入个性化。按 user_id 隔离。

use serde_json::Value;
use sqlx::PgPool;

/// 用户 memory 存储。
#[derive(Clone)]
pub struct UserMemoryStore {
    pool: PgPool,
}

impl UserMemoryStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// 加载用户 memory（无行 → 默认 `{}`）。
    pub async fn load(&self, user_id: i64) -> Result<Value, String> {
        let row: Option<(Value,)> = sqlx::query_as(
            r#"SELECT memory FROM isahl_auth.gateway_user_memory WHERE user_id = $1"#,
        )
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| format!("load user memory failed: {}", e))?;

        Ok(row.map(|r| r.0).unwrap_or_else(|| serde_json::json!({})))
    }

    /// 保存用户 memory（upsert + version 递增）。
    ///
    /// 当前由注入链路消费（load）；对话后记忆沉淀（从对话提取偏好写入）
    /// 为独立后续——API 就绪，暂标 dead_code 避免 warning。
    #[allow(dead_code)]
    pub async fn save(&self, user_id: i64, memory: Value) -> Result<(), String> {
        sqlx::query(
            r#"INSERT INTO isahl_auth.gateway_user_memory (user_id, memory, version, updated_at)
               VALUES ($1, $2, 1, NOW())
               ON CONFLICT (user_id) DO UPDATE
               SET memory = EXCLUDED.memory,
                   version = isahl_auth.gateway_user_memory.version + 1,
                   updated_at = NOW()"#,
        )
        .bind(user_id)
        .bind(memory)
        .execute(&self.pool)
        .await
        .map_err(|e| format!("save user memory failed: {}", e))?;
        Ok(())
    }
}
