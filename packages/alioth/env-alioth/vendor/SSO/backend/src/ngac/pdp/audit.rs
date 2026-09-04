use actix_web::{web, HttpRequest};
use sqlx::PgPool;

use super::*;

use crate::epp::{AccessEvent, EventHandler};

/// 将一次 NGAC PDP 决策异步写入审计事件表（`audit_events`），打通此前未接线的 EPP 管道。
///
/// 采用 fire-and-forget：通过 `tokio::spawn` + `EventHandler::handle_access_event` 后台落库，
/// 不阻塞 PDP 决策响应。失败仅记录日志，不影响决策返回。
pub(crate) fn audit_decision(
    pool: web::Data<PgPool>,
    user_id: i64,
    object_path: &str,
    operation: &str,
    decision: Decision,
    req: &HttpRequest,
) {
    let ip = req.peer_addr().map(|a| a.ip().to_string());
    let ua = req
        .headers()
        .get(actix_web::http::header::USER_AGENT)
        .and_then(|h| h.to_str().ok())
        .map(|s| s.to_string());

    // audit_events.user_email 为 NOT NULL；本路径仅有 user_id，沿用
    // handlers.rs / From<NgacEvent> 的占位值惯例（满足约束，避免 null-value 违规）。
    let mut event = AccessEvent::new(
        user_id,
        Some("audit@local".to_string()),
        object_path.to_string(),
        operation.to_string(),
        decision,
    );
    if let Some(ip) = ip {
        event = event.with_ip_address(ip);
    }
    if let Some(ua) = ua {
        event = event.with_user_agent(ua);
    }

    // 实时广播到 /ws/audit 订阅者（M2：websocket 审计流生产者）。
    // 跨 worker 共享的 AuditWsServer（WS_SERVER OnceLock）保证所有 worker 的客户端可见。
    if let Some(ws_server) = crate::websocket::get_ws_server() {
        ws_server.broadcast_audit_event(crate::websocket::AuditEvent {
            event_id: uuid::Uuid::from_u64_pair(0, event.id as u64),
            event_type: "access".to_string(),
            timestamp: event.timestamp,
            subject_id: uuid::Uuid::from_u64_pair(0, event.user_id as u64),
            object_id: uuid::Uuid::from_u64_pair(0, 0),
            object_type: event.object_path.clone(),
            operation: event.operation.clone(),
            success: event.decision == Decision::Permit,
            metadata: None,
        });
    }

    tokio::spawn(async move {
        let handler = EventHandler::new(pool.get_ref().clone());
        if let Err(e) = handler.handle_access_event(event).await {
            log::error!("audit_decision: failed to record access event: {}", e);
        }
    });
}
