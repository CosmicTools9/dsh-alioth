//! 消息服务 seam
//!
//! 提供站内信（IM）和设备命令的高层接口。
//! 模块通过 `MessagingService` 发送消息，无需了解底层是 ZChat、MQTT 还是其他实现。
//!
//! 错误语义标注：
//! - HTTP 层错误（如参数非法）→ `AliothError::BadRequest` / `AliothError::NotFound`
//! - ZChat 子系统错误 → `AliothError::External { source: "ZChat", ... }`
//! - MQTT 子系统错误 → `AliothError::External { source: "MQTT", ... }`

use crate::error::AliothError;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// 消息服务 trait。
///
/// Gateway 启动时根据配置注入具体实现（r-chess Adapter 或 Noop）。
/// 模块 handler 通过 `web::Data<Option<Arc<dyn MessagingService>>>` 获取。
#[async_trait]
pub trait MessagingService: Send + Sync + 'static {
    /// 发送单聊站内信。
    async fn send_direct(&self, from: u64, to: u64, content: &str) -> Result<(), AliothError>;

    /// 发送群聊站内信。
    async fn send_group(
        &self,
        from: u64,
        conversation_id: u64,
        content: &str,
    ) -> Result<(), AliothError>;

    /// 广播消息到全部在线用户。
    async fn broadcast(&self, from: u64, content: &str) -> Result<(), AliothError>;

    /// 向指定用户发送系统通知。
    async fn send_system_notification(
        &self,
        to: u64,
        title: &str,
        content: &str,
    ) -> Result<(), AliothError>;

    /// 发送分级告警消息。
    async fn send_alert(
        &self,
        level: AlertLevel,
        title: &str,
        content: &str,
    ) -> Result<(), AliothError>;

    /// 向指定设备下发指令。
    async fn send_device_command(
        &self,
        device_id: &str,
        command: DeviceCommand,
    ) -> Result<(), AliothError>;

    /// 广播指令到所有设备。
    async fn broadcast_device_command(&self, command: DeviceCommand) -> Result<(), AliothError>;

    /// 向指定 topic 发送原始消息（基础设施级，供管理后台使用）。
    ///
    /// 错误语义：MQTT 子系统错误 → `External { source: "MQTT", ... }`
    async fn send_raw(
        &self,
        topic: &str,
        payload: Vec<u8>,
        qos: u8,
    ) -> Result<MessageDeliveryInfo, AliothError>;
}

/// 消息投递结果。
#[derive(Debug, Clone)]
pub struct MessageDeliveryInfo {
    pub delivered_count: usize,
    pub offline_queued_count: usize,
}

/// 告警级别。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum AlertLevel {
    #[serde(rename = "critical")]
    Critical,
    #[serde(rename = "warning")]
    Warning,
    #[serde(rename = "info")]
    Info,
}

impl AlertLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            AlertLevel::Critical => "critical",
            AlertLevel::Warning => "warning",
            AlertLevel::Info => "info",
        }
    }
}

/// 设备指令。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceCommand {
    pub command_id: String,
    pub command_type: String,
    pub params: serde_json::Value,
}

impl DeviceCommand {
    pub fn new(command_id: impl Into<String>, command_type: impl Into<String>) -> Self {
        Self {
            command_id: command_id.into(),
            command_type: command_type.into(),
            params: serde_json::Value::Null,
        }
    }

    pub fn with_params(mut self, params: serde_json::Value) -> Self {
        self.params = params;
        self
    }
}
