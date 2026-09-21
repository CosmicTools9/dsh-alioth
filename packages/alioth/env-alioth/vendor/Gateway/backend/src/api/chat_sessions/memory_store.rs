//! Chat-AI 双层记忆存储（refactor-chat-ai-subject-identity-memory）。
//!
//! 两层均落 `isahl_auth`（Gateway 完全访问域；`isahl` 冻结、`isahl_meta` 禁访问）：
//! - L1 主体层 `isahl_auth.chat_ai_subject_memory`：键 = 智能体主体 id
//!   （`isahl.zc_id_empl-agent.id`），跨对话方共享累积；
//! - L2 对话方层 `isahl_auth.chat_ai_counterpart_memory`：键 = 主体 id × 联系人 id
//!   （`isahl.zc_id_contacts.id`），对话方私有。
//!
//! 覆盖语义：全量替换（`SET memory = EXCLUDED.memory`）——新的覆盖旧的，
//! 无合并层、无审阅层（用户裁决 2026-09-13）。键维度是主体，MUST NOT 按人类账号
//! （`user_id`）读写。

use serde_json::Value;
use sqlx::PgPool;

/// 一次读取返回的两层记忆（缺行 → 空对象）。
#[derive(Debug, Clone)]
pub struct MemoryLayers {
    /// L1 主体层（跨对话方共享）
    pub subject: Value,
    /// L2 对话方层（本对话方私有）；无对话方上下文时为 None
    pub counterpart: Option<Value>,
}

impl Default for MemoryLayers {
    /// 与 store 的「缺行 → `{}`」语义一致（`Value::default()` 是 `Null`，不可直接 derive）。
    fn default() -> Self {
        Self {
            subject: serde_json::json!({}),
            counterpart: None,
        }
    }
}

/// 双层记忆存储。
#[derive(Clone)]
pub struct ChatMemoryStore {
    pool: PgPool,
}

impl ChatMemoryStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// 加载两层记忆（L1 必备；L2 需 counterpart_id）。
    pub async fn load(
        &self,
        subject_id: i64,
        counterpart_id: Option<i64>,
    ) -> Result<MemoryLayers, String> {
        let subject = self.load_subject(subject_id).await?;
        let counterpart = match counterpart_id {
            Some(cid) => Some(self.load_counterpart(subject_id, cid).await?),
            None => None,
        };
        Ok(MemoryLayers {
            subject,
            counterpart,
        })
    }

    /// L1 主体层读取（无行 → `{}`）。
    pub async fn load_subject(&self, subject_id: i64) -> Result<Value, String> {
        let row: Option<(Value,)> = sqlx::query_as(
            r#"SELECT memory FROM isahl_auth.chat_ai_subject_memory WHERE subject_id = $1"#,
        )
        .bind(subject_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| format!("load subject memory failed: {}", e))?;
        Ok(row.map(|r| r.0).unwrap_or_else(|| serde_json::json!({})))
    }

    /// L2 对话方层读取（无行 → `{}`）。
    pub async fn load_counterpart(
        &self,
        subject_id: i64,
        counterpart_id: i64,
    ) -> Result<Value, String> {
        let row: Option<(Value,)> = sqlx::query_as(
            r#"SELECT memory FROM isahl_auth.chat_ai_counterpart_memory
               WHERE subject_id = $1 AND counterpart_id = $2"#,
        )
        .bind(subject_id)
        .bind(counterpart_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| format!("load counterpart memory failed: {}", e))?;
        Ok(row.map(|r| r.0).unwrap_or_else(|| serde_json::json!({})))
    }

    /// L1 主体层全量替换写入（upsert + version 递增）。
    pub async fn save_subject(&self, subject_id: i64, memory: Value) -> Result<(), String> {
        sqlx::query(
            r#"INSERT INTO isahl_auth.chat_ai_subject_memory (subject_id, memory, version, updated_at)
               VALUES ($1, $2, 1, NOW())
               ON CONFLICT (subject_id) DO UPDATE
               SET memory = EXCLUDED.memory,
                   version = isahl_auth.chat_ai_subject_memory.version + 1,
                   updated_at = NOW()"#,
        )
        .bind(subject_id)
        .bind(memory)
        .execute(&self.pool)
        .await
        .map_err(|e| format!("save subject memory failed: {}", e))?;
        Ok(())
    }

    /// L2 对话方层全量替换写入（upsert + version 递增）。
    pub async fn save_counterpart(
        &self,
        subject_id: i64,
        counterpart_id: i64,
        memory: Value,
    ) -> Result<(), String> {
        sqlx::query(
            r#"INSERT INTO isahl_auth.chat_ai_counterpart_memory
                   (subject_id, counterpart_id, memory, version, updated_at)
               VALUES ($1, $2, $3, 1, NOW())
               ON CONFLICT (subject_id, counterpart_id) DO UPDATE
               SET memory = EXCLUDED.memory,
                   version = isahl_auth.chat_ai_counterpart_memory.version + 1,
                   updated_at = NOW()"#,
        )
        .bind(subject_id)
        .bind(counterpart_id)
        .bind(memory)
        .execute(&self.pool)
        .await
        .map_err(|e| format!("save counterpart memory failed: {}", e))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_layers_default_is_empty() {
        let layers = MemoryLayers::default();
        assert_eq!(layers.subject, serde_json::json!({}));
        assert!(layers.counterpart.is_none());
    }
}
