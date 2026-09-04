//! 用户订阅 Repository — 基于 auth_users.subscriptions JSONB

use crate::notification::models::{
    CreateSubscriptionRequest, UpdateSubscriptionRequest, UserSubscription,
};
use sqlx::PgPool;
use uuid::Uuid;

pub struct SubscriptionRepository {
    pool: PgPool,
}

impl From<PgPool> for SubscriptionRepository {
    fn from(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl SubscriptionRepository {
    /// 读取用户的订阅列表。
    pub async fn list_by_user(&self, user_id: i64) -> Result<Vec<UserSubscription>, sqlx::Error> {
        let row: Option<serde_json::Value> =
            sqlx::query_scalar(r#"SELECT subscriptions FROM isahl_auth.auth_users WHERE id = $1"#)
                .bind(user_id)
                .fetch_optional(&self.pool)
                .await?;

        match row {
            Some(Value::Array(arr)) => {
                let subs: Vec<UserSubscription> = arr
                    .into_iter()
                    .filter_map(|v| serde_json::from_value(v).ok())
                    .collect();
                Ok(subs)
            }
            _ => Ok(Vec::new()),
        }
    }

    /// 查找匹配特定表和事件类型的所有活跃订阅。
    ///
    /// 返回 (user_id, subscription) 列表。
    pub async fn find_matching_subscriptions(
        &self,
        target_table: &str,
        event_type: &str,
    ) -> Result<Vec<(i64, UserSubscription)>, sqlx::Error> {
        let rows: Vec<(i64, serde_json::Value)> = sqlx::query_as(
            r#"SELECT u.id, sub
               FROM isahl_auth.auth_users u,
               LATERAL jsonb_array_elements(u.subscriptions) sub
               WHERE sub->>'target_table' = $1
                 AND (sub->>'is_active')::boolean = true
                 AND sub->'event_types' ? $2"#,
        )
        .bind(target_table)
        .bind(event_type)
        .fetch_all(&self.pool)
        .await?;

        let mut result = Vec::new();
        for (user_id, sub_val) in rows {
            if let Ok(sub) = serde_json::from_value(sub_val) {
                result.push((user_id, sub));
            }
        }
        Ok(result)
    }

    /// 按订阅 ID 获取单条订阅（含 user_id）。
    pub async fn get_by_id(
        &self,
        user_id: i64,
        sub_id: &str,
    ) -> Result<Option<UserSubscription>, sqlx::Error> {
        let subs = self.list_by_user(user_id).await?;
        Ok(subs.into_iter().find(|s| s.id == sub_id))
    }

    /// 创建订阅 — append 到 JSONB 数组。
    pub async fn create(
        &self,
        user_id: i64,
        req: CreateSubscriptionRequest,
    ) -> Result<UserSubscription, sqlx::Error> {
        let sub = UserSubscription {
            id: Uuid::new_v4().to_string(),
            target_table: req.target_table,
            target_id: req.target_id,
            event_types: req.event_types,
            notice: req.notice,
            is_active: true,
            created_at: Utc::now(),
        };

        let sub_json = serde_json::to_value(&sub).unwrap_or(serde_json::Value::Null);

        sqlx::query(
            r#"UPDATE isahl_auth.auth_users
               SET subscriptions = COALESCE(subscriptions, '[]'::jsonb) || $2::jsonb,
                   updated_at = NOW()
               WHERE id = $1"#,
        )
        .bind(user_id)
        .bind(&sub_json)
        .execute(&self.pool)
        .await?;

        Ok(sub)
    }

    /// 更新订阅 — 读取-修改-写回。
    pub async fn update(
        &self,
        user_id: i64,
        sub_id: &str,
        req: UpdateSubscriptionRequest,
    ) -> Result<Option<UserSubscription>, sqlx::Error> {
        let mut subs = self.list_by_user(user_id).await?;
        let idx = subs.iter().position(|s| s.id == sub_id);

        let idx = match idx {
            Some(i) => i,
            None => return Ok(None),
        };

        let sub = &mut subs[idx];
        if let Some(v) = req.target_table {
            sub.target_table = v;
        }
        if req.target_id.is_some() {
            sub.target_id = req.target_id;
        }
        if let Some(v) = req.event_types {
            sub.event_types = v;
        }
        if req.notice.is_some() {
            sub.notice = req.notice;
        }
        if let Some(v) = req.is_active {
            sub.is_active = v;
        }

        let updated = sub.clone();
        self.save_all(user_id, &subs).await?;
        Ok(Some(updated))
    }

    /// 删除订阅 — 过滤后写回。
    pub async fn delete(&self, user_id: i64, sub_id: &str) -> Result<bool, sqlx::Error> {
        let subs = self.list_by_user(user_id).await?;
        let original_len = subs.len();
        let filtered: Vec<UserSubscription> = subs.into_iter().filter(|s| s.id != sub_id).collect();

        if filtered.len() == original_len {
            return Ok(false);
        }

        self.save_all(user_id, &filtered).await?;
        Ok(true)
    }

    /// 全量覆盖保存用户的订阅列表。
    async fn save_all(&self, user_id: i64, subs: &[UserSubscription]) -> Result<(), sqlx::Error> {
        let arr = serde_json::to_value(subs).unwrap_or(serde_json::Value::Array(vec![]));
        sqlx::query(
            r#"UPDATE isahl_auth.auth_users
               SET subscriptions = $2,
                   updated_at = NOW()
               WHERE id = $1"#,
        )
        .bind(user_id)
        .bind(&arr)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

use chrono::Utc;
use serde_json::Value;
