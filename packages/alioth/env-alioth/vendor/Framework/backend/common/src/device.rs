//! 设备消息处理接口
//!
//! 供模块 backend 实现，处理来自 MQTT 的设备上报数据。

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// 设备指令执行结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CommandResult {
    Success { output: Option<String> },
    Failed { reason: String },
    Timeout,
}

/// 设备消息处理器 trait。
///
/// 模块 backend 实现此 trait 并注册到 Gateway 的 DeviceMessageHub，
/// 即可接收设备通过 MQTT 上报的遥测数据和指令响应。
#[async_trait]
pub trait DeviceMessageHandler: Send + Sync {
    /// 设备遥测数据上报。
    ///
    /// `device_id` — 设备在 AliothStudio 中的 zuid（或 mqtt_client_id）。
    /// `payload`   — MQTT PUBLISH payload（通常为 JSON / protobuf / 二进制传感器数据）。
    async fn on_telemetry(&self, device_id: &str, payload: &[u8]);

    /// 设备指令执行响应。
    ///
    /// 当模块 backend 通过 MQTT 向设备下发指令后，设备执行完毕返回结果。
    async fn on_command_response(&self, device_id: &str, command_id: &str, result: CommandResult);

    /// 设备在线状态变化。
    async fn on_online_status_change(&self, device_id: &str, online: bool);
}
