//! 站内信公共类型

use serde::{Deserialize, Serialize};

/// 站内信操作响应
#[derive(Serialize)]
pub struct InboxActionResponse {
    pub success: bool,
    pub message: String,
}

impl InboxActionResponse {
    pub fn ok(msg: impl Into<String>) -> Self {
        Self {
            success: true,
            message: msg.into(),
        }
    }
    pub fn fail(msg: impl Into<String>) -> Self {
        Self {
            success: false,
            message: msg.into(),
        }
    }
}

/// 发送站内信请求
#[derive(Debug, Deserialize)]
pub struct SendMessageRequest {
    /// 消息标题
    pub title: String,
    /// 消息内容
    pub content: String,
    /// 收件人 contact/user IDs
    #[serde(with = "common::serde_zuid::seq")]
    pub recipient_ids: Vec<i64>,
    /// 回复的消息 ID（回复时传入，继承 thread）
    #[serde(default)]
    #[serde(with = "common::serde_zuid::opt")]
    pub previous_id: Option<i64>,
}

impl SendMessageRequest {
    pub fn validate(&self) -> Result<(), String> {
        if self.title.trim().is_empty() {
            return Err("标题不能为空".into());
        }
        if self.recipient_ids.is_empty() {
            return Err("收件人不能为空".into());
        }
        Ok(())
    }
}
