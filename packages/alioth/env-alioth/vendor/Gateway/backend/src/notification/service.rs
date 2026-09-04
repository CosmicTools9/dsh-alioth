//! 通知服务 — 数据变更后自动匹配订阅并推送站内信

use crate::notification::models::UserSubscription;
use crate::notification::repository::SubscriptionRepository;
use serde_json::Value;
use sqlx::PgPool;
use std::collections::HashMap;

use std::sync::Arc;

/// 通知服务。
///
/// 在触发器 after 链路中被调用，负责：
/// 1. 查询 auth_users.subscriptions JSONB 中匹配的订阅
/// 2. 通过 MessagingService 向订阅者发送站内信
#[derive(Clone)]
pub struct NotificationService {
    pool: PgPool,
    messaging: Option<Arc<dyn common::messaging::MessagingService>>,
}

impl NotificationService {
    /// 创建通知服务实例。
    pub fn new(
        pool: PgPool,
        messaging: Option<Arc<dyn common::messaging::MessagingService>>,
    ) -> Self {
        Self { pool, messaging }
    }

    /// 数据变更后触发通知。
    ///
    /// 由 trigger_crud 的 after_insert / after_update / after_delete 调用。
    pub async fn notify_data_change(
        &self,
        table_name: &str,
        record_id: i64,
        operation: &str, // "insert", "update", "delete"
        _record: &HashMap<String, Value>,
    ) {
        let repo = SubscriptionRepository::from(self.pool.clone());

        // 1. 查询匹配该表+事件类型的活跃订阅（跨所有用户）
        let matched = match repo
            .find_matching_subscriptions(table_name, operation)
            .await
        {
            Ok(m) => m,
            Err(e) => {
                common::telemetry::warn!(
                    "Failed to query subscriptions for {} {}: {}",
                    table_name,
                    operation,
                    e
                );
                return;
            }
        };

        if matched.is_empty() {
            return;
        }

        // 2. 过滤 sk_target_id 精确匹配（target_id 为 None 表示关注整表）
        let filtered: Vec<(i64, &UserSubscription)> = matched
            .iter()
            .filter(|(_, sub)| sub.target_id.is_none() || sub.target_id == Some(record_id))
            .map(|(uid, sub)| (*uid, sub))
            .collect();

        if filtered.is_empty() {
            return;
        }

        // 4. 逐个发送站内信
        for (user_id, sub) in filtered {
            if let Some(ref messaging) = self.messaging {
                let title = format!("[{}] 数据{}通知", table_name, operation_cn(operation));
                let content = format!(
                    "您关注的 {} (ID: {}) 发生了{}操作。",
                    table_name,
                    record_id,
                    operation_cn(operation)
                );
                if let Err(e) = messaging
                    .send_system_notification(user_id as u64, &title, &content)
                    .await
                {
                    common::telemetry::warn!(
                        "Failed to send notification to user {} for subscription {}: {}",
                        user_id,
                        sub.id,
                        e
                    );
                } else {
                    common::telemetry::info!(
                        "Notification sent to user {}: {} {} on {}",
                        user_id,
                        operation,
                        table_name,
                        record_id
                    );
                }
            } else {
                common::telemetry::info!(
                    "[MESSAGING_DISABLED] Would notify user {}: {} {} on {} (subscription {})",
                    user_id,
                    operation,
                    table_name,
                    record_id,
                    sub.id
                );
            }
        }
    }
}

fn operation_cn(op: &str) -> String {
    match op {
        "insert" => "新增".to_string(),
        "update" => "更新".to_string(),
        "delete" => "删除".to_string(),
        _ => op.to_string(),
    }
}
