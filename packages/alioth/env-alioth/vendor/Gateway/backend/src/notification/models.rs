//! Notification 模块数据模型

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// 用户订阅记录（存储于 auth_users.subscriptions JSONB 数组中）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserSubscription {
    /// UUID v4，客户端生成或后端生成均可。
    pub id: String,
    pub target_table: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(with = "common::serde_zuid::opt")]
    pub target_id: Option<i64>,
    pub event_types: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notice: Option<String>,
    #[serde(default = "default_true")]
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
}

/// 创建订阅请求。
#[derive(Debug, Deserialize)]
pub struct CreateSubscriptionRequest {
    pub target_table: String,
    #[serde(with = "common::serde_zuid::opt")]
    pub target_id: Option<i64>,
    #[serde(default = "default_event_types")]
    pub event_types: Vec<String>,
    pub notice: Option<String>,
}

/// 更新订阅请求。
#[derive(Debug, Deserialize)]
pub struct UpdateSubscriptionRequest {
    pub target_table: Option<String>,
    #[serde(with = "common::serde_zuid::opt")]
    pub target_id: Option<i64>,
    pub event_types: Option<Vec<String>>,
    pub notice: Option<String>,
    pub is_active: Option<bool>,
}

/// 订阅列表响应。
#[derive(Debug, Serialize)]
pub struct SubscriptionListResponse {
    pub items: Vec<UserSubscription>,
}

fn default_true() -> bool {
    true
}

fn default_event_types() -> Vec<String> {
    vec![
        "insert".to_string(),
        "update".to_string(),
        "delete".to_string(),
    ]
}
