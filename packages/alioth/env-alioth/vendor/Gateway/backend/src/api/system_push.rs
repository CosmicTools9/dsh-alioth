//! Gateway 系统推送 API（基础设施级）
//!
//! 提供 Gateway 层面的站内信和设备推送能力，供所有模块及基础设施调用。
//! 路由前缀: /api/system-push/*
//!
//! 通过 `MessagingService` seam 实现，若 MessagingService 未注入，所有接口返回 400。

use actix_web::{web, HttpResponse, Result};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use common::messaging::{AlertLevel, MessagingService};

// ============================================
// Request / Response Types
// ============================================

/// 系统站内信通知请求。
#[derive(Debug, Deserialize)]
pub struct SystemNotificationRequest {
    /// 目标用户 ID；为 None 时广播给所有在线用户。
    pub user_id: Option<u64>,
    pub title: String,
    pub content: String,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

/// 站内信广播请求。
#[derive(Debug, Deserialize)]
pub struct ImBroadcastRequest {
    pub content: String,
    #[serde(default)]
    pub from: Option<u64>,
}

/// 设备广播请求。
#[derive(Debug, Deserialize)]
pub struct DeviceBroadcastRequest {
    pub notification_type: String,
    #[serde(default)]
    pub payload: Option<serde_json::Value>,
}

/// 设备分组推送请求。
#[derive(Debug, Deserialize)]
pub struct DeviceGroupRequest {
    pub group_id: String,
    pub message_type: String,
    #[serde(default)]
    pub payload: Option<serde_json::Value>,
    #[serde(default = "default_qos_1")]
    pub qos: u8,
}

/// 批量设备推送请求。
#[derive(Debug, Deserialize)]
pub struct DeviceBatchRequest {
    pub device_ids: Vec<String>,
    pub message_type: String,
    #[serde(default)]
    pub payload: Option<serde_json::Value>,
}

/// 告警推送请求。
#[derive(Debug, Deserialize)]
pub struct AlertRequest {
    #[serde(default = "default_alert_warning")]
    pub level: String,
    pub title: String,
    pub content: String,
    #[serde(default)]
    pub source: Option<String>,
}

/// 统一响应体。
#[derive(Debug, Serialize)]
pub struct PushResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivered_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offline_queued_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failed: Option<Vec<String>>,
}

fn default_qos_1() -> u8 {
    1
}

fn default_alert_warning() -> String {
    "warning".to_string()
}

fn service_unavailable() -> HttpResponse {
    HttpResponse::BadRequest().json(serde_json::json!({
        "success": false,
        "error": "Messaging service is not available"
    }))
}

/// 将 MessagingService 错误映射为 HTTP 错误：
/// NotImplemented → 501（调用方可感知能力缺失，禁止静默成功）；
/// 其余 → 500。
fn map_messaging_err(e: common::error::AliothError) -> actix_web::Error {
    match e {
        common::error::AliothError::NotImplemented(msg) => {
            actix_web::error::ErrorNotImplemented(msg)
        }
        other => actix_web::error::ErrorInternalServerError(other.to_string()),
    }
}

// ============================================
// Handlers
// ============================================

/// POST /api/system-push/im/notification
/// 发送系统站内信通知（单播或广播）。
pub async fn send_system_notification(
    body: web::Json<SystemNotificationRequest>,
    messaging: Option<web::Data<Arc<dyn MessagingService>>>,
) -> Result<HttpResponse> {
    let req = body.into_inner();
    let service = match messaging.as_deref() {
        Some(s) => s,
        None => return Ok(service_unavailable()),
    };

    if let Some(uid) = req.user_id {
        service
            .send_system_notification(uid, &req.title, &req.content)
            .await
            .map_err(map_messaging_err)?;
    } else {
        service
            .broadcast(0, &format!("[{}] {}", req.title, req.content))
            .await
            .map_err(map_messaging_err)?;
    }

    Ok(HttpResponse::Ok().json(PushResponse {
        success: true,
        delivered_count: None,
        offline_queued_count: None,
        message: Some("Notification sent".to_string()),
        failed: None,
    }))
}

/// POST /api/system-push/im/broadcast
/// 站内信广播（兼容旧版 API）。
pub async fn broadcast_im(
    body: web::Json<ImBroadcastRequest>,
    messaging: Option<web::Data<Arc<dyn MessagingService>>>,
) -> Result<HttpResponse> {
    let req = body.into_inner();
    let service = match messaging.as_deref() {
        Some(s) => s,
        None => return Ok(service_unavailable()),
    };

    service
        .broadcast(req.from.unwrap_or(0), &req.content)
        .await
        .map_err(map_messaging_err)?;

    Ok(HttpResponse::Ok().json(PushResponse {
        success: true,
        delivered_count: None,
        offline_queued_count: None,
        message: Some("Broadcast sent".to_string()),
        failed: None,
    }))
}

/// POST /api/system-push/im/alert
/// 发送分级告警。
pub async fn send_alert(
    body: web::Json<AlertRequest>,
    messaging: Option<web::Data<Arc<dyn MessagingService>>>,
) -> Result<HttpResponse> {
    let req = body.into_inner();
    let service = match messaging.as_deref() {
        Some(s) => s,
        None => return Ok(service_unavailable()),
    };

    let level = match req.level.as_str() {
        "critical" => AlertLevel::Critical,
        "info" => AlertLevel::Info,
        _ => AlertLevel::Warning,
    };

    let content = if let Some(source) = req.source {
        format!("[{}] {}", source, req.content)
    } else {
        req.content
    };

    service
        .send_alert(level, &req.title, &content)
        .await
        .map_err(map_messaging_err)?;

    Ok(HttpResponse::Ok().json(PushResponse {
        success: true,
        delivered_count: None,
        offline_queued_count: None,
        message: Some(format!("alert [{}] sent", req.level)),
        failed: None,
    }))
}

/// POST /api/system-push/device/broadcast
/// 向所有已接入设备广播通知。
pub async fn broadcast_device(
    body: web::Json<DeviceBroadcastRequest>,
    messaging: Option<web::Data<Arc<dyn MessagingService>>>,
) -> Result<HttpResponse> {
    let req = body.into_inner();
    let service = match messaging.as_deref() {
        Some(s) => s,
        None => return Ok(service_unavailable()),
    };

    let payload = req.payload.unwrap_or(serde_json::Value::Null);
    let envelope = serde_json::json!({
        "notification_type": req.notification_type,
        "payload": payload,
        "timestamp": chrono::Utc::now().timestamp_millis(),
    });
    let body = serde_json::to_vec(&envelope).map_err(|e| {
        actix_web::error::ErrorInternalServerError(format!("Serialize failed: {}", e))
    })?;

    let info = service
        .send_raw("device/all/notification", body, 1)
        .await
        .map_err(map_messaging_err)?;

    Ok(HttpResponse::Ok().json(PushResponse {
        success: true,
        delivered_count: Some(info.delivered_count),
        offline_queued_count: Some(info.offline_queued_count),
        message: Some(format!(
            "device broadcast delivered={}, offline_queued={}",
            info.delivered_count, info.offline_queued_count
        )),
        failed: None,
    }))
}

/// POST /api/system-push/device/group
/// 向设备分组推送。
pub async fn push_device_group(
    body: web::Json<DeviceGroupRequest>,
    messaging: Option<web::Data<Arc<dyn MessagingService>>>,
) -> Result<HttpResponse> {
    let req = body.into_inner();
    let service = match messaging.as_deref() {
        Some(s) => s,
        None => return Ok(service_unavailable()),
    };

    let payload = req.payload.unwrap_or(serde_json::Value::Null);
    let envelope = serde_json::json!({
        "message_type": req.message_type,
        "payload": payload,
        "timestamp": chrono::Utc::now().timestamp_millis(),
    });
    let body = serde_json::to_vec(&envelope).map_err(|e| {
        actix_web::error::ErrorInternalServerError(format!("Serialize failed: {}", e))
    })?;

    let topic = format!("device/group/{}", req.group_id);
    let info = service
        .send_raw(&topic, body, req.qos.min(2))
        .await
        .map_err(map_messaging_err)?;

    Ok(HttpResponse::Ok().json(PushResponse {
        success: true,
        delivered_count: Some(info.delivered_count),
        offline_queued_count: Some(info.offline_queued_count),
        message: Some(format!(
            "group [{}] delivered={}, offline_queued={}",
            req.group_id, info.delivered_count, info.offline_queued_count
        )),
        failed: None,
    }))
}

/// POST /api/system-push/device/batch
/// 向多个指定设备批量推送。
pub async fn push_device_batch(
    body: web::Json<DeviceBatchRequest>,
    messaging: Option<web::Data<Arc<dyn MessagingService>>>,
) -> Result<HttpResponse> {
    let req = body.into_inner();
    let service = match messaging.as_deref() {
        Some(s) => s,
        None => return Ok(service_unavailable()),
    };

    let payload = req.payload.unwrap_or(serde_json::Value::Null);
    let mut total_delivered = 0;
    let mut total_offline_queued = 0;
    let mut failed = Vec::new();

    for device_id in &req.device_ids {
        let topic = format!("device/{}/{}", device_id, req.message_type);
        let body = match serde_json::to_vec(&payload) {
            Ok(b) => b,
            Err(e) => {
                failed.push(device_id.clone());
                common::telemetry::warn!("Serialize failed for device {}: {}", device_id, e);
                continue;
            }
        };

        match service.send_raw(&topic, body, 1).await {
            Ok(info) => {
                total_delivered += info.delivered_count;
                total_offline_queued += info.offline_queued_count;
            }
            Err(e) => {
                failed.push(device_id.clone());
                common::telemetry::warn!("Send failed for device {}: {}", device_id, e);
            }
        }
    }

    Ok(HttpResponse::Ok().json(PushResponse {
        success: failed.is_empty(),
        delivered_count: Some(total_delivered),
        offline_queued_count: Some(total_offline_queued),
        message: Some(format!(
            "batch {} devices: delivered={}, offline_queued={}, failed={}",
            req.device_ids.len(),
            total_delivered,
            total_offline_queued,
            failed.len()
        )),
        failed: if failed.is_empty() {
            None
        } else {
            Some(failed)
        },
    }))
}

// ============================================
// Route Configuration
// ============================================

pub fn configure_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/system-push")
            // 站内信推送
            .route("/im/notification", web::post().to(send_system_notification))
            .route("/im/broadcast", web::post().to(broadcast_im))
            .route("/im/alert", web::post().to(send_alert))
            // 设备推送
            .route("/device/broadcast", web::post().to(broadcast_device))
            .route("/device/group", web::post().to(push_device_group))
            .route("/device/batch", web::post().to(push_device_batch)),
    );
}
