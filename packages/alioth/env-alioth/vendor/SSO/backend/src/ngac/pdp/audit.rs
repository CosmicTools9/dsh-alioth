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

    // audit_events.user_email 为 NOT NULL；本路径仅有 user_id。
    // 主体标识口径（fix-ngac-audit-subject-identity）：username 优先、回落
    // `user:{id}`——此处无 username，用唯一回落；**禁止**跨主体共享常量
    // （原 "audit@local" 会把全部用户折叠成同一审计主体）。
    let mut event = AccessEvent::new(
        user_id,
        Some(format!("user:{user_id}")),
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

    // 审计开关（change `wire-org-policy-declaration-fields`）：资源类型按 PEP 同源口径
    // `"{resource_type}:{id}"`（`auth/middleware.rs::map_pep_resource`）解析；命中
    // 「active 且在生效窗口内的 class 白名单覆盖该类型且 `audit_required = FALSE`」
    // ⇒ 本次决策**不产生任何审计事件**（DB 行 / WS 帧 / 义务上报），判决不受影响。
    let resource_type = object_path
        .split(':')
        .next()
        .unwrap_or_default()
        .to_string();

    tokio::spawn(async move {
        let pool = pool.get_ref().clone();

        // 抑制判据在审计任务内求值（保持本函数同步/fire-and-forget 语义）；查询失败 ⇒ 照常审计。
        if ngac_host::audit_suppressed_for_resource_type(&pool, &resource_type).await {
            return;
        }

        // 实时广播到 /ws/audit 订阅者（M2：websocket 审计流生产者）。
        // 跨 worker 共享的 AuditWsServer（WS_SERVER OnceLock）保证所有 worker 的客户端可见。
        // 与落库同处抑制判据之内：抑制 = 「无审计」，不留 DB 与流不一致面。
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

        // 义务求值（NGAC_SPEC §3.3）：**判决不变**，仅产出 `obligations_triggered` 列表
        // （平台唯一表达式引擎 = Rhai；失败封闭，见 `ngac::obligation` 模块文档）。
        let obligations = crate::ngac::obligation::evaluate_obligations(
            &pool,
            &crate::ngac::obligation::ObligationEvalContext {
                user_id: event.user_id,
                resource: &event.object_path,
                action: &event.operation,
                decision: &event.decision.to_string(),
                subject_attributes: &event.subject_attributes,
                object_attributes: &event.object_attributes,
            },
        )
        .await;
        let event = event.with_obligations(obligations);
        let handler = EventHandler::new(pool);
        if let Err(e) = handler.handle_access_event(event).await {
            log::error!("audit_decision: failed to record access event: {}", e);
        }
    });
}
