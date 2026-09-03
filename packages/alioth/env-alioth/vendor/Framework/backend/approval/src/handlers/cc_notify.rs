//! 审批抄送通知消费（fix-approval-engine-gap-closure D9）
//!
//! 消费引擎推进 cc 节点发布的 `ApprovalCc` 领域事件：payload.resolvedUsers
//! （结构化收件人解析出的用户 id）逐人投递系统通知（MessagingService，与
//! SLA 升级/转交/催办同通道）；resolvedUsers 空（legacy 纯文本收件人）→
//! warn 跳过；投递失败仅 warn——通知是增强投递，不阻断发布方主流程。
//! 无 messaging 注入（降级）：仅记录事件，不投递。

use common::event_bus::{DomainEvent, DomainEventBus};
use common::messaging::MessagingService;
use sqlx::PgPool;
use std::sync::Arc;

use super::notify::notify_user;

async fn handle_event(
    pool: &PgPool,
    messaging: &Option<Arc<dyn MessagingService>>,
    event: DomainEvent,
) {
    if event.event_type != "ApprovalCc" {
        return;
    }
    let flow_id = event
        .payload
        .get("flow_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let node_id = event
        .payload
        .get("node_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let label = event
        .payload
        .get("label")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let users: Vec<i64> = event
        .payload
        .get("resolvedUsers")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|u| u.as_i64()).collect())
        .unwrap_or_default();
    let mut anchor = format!(
        "节点「{}」",
        if label.is_empty() {
            "(未命名)"
        } else {
            &label
        }
    );
    if !node_id.is_empty() {
        anchor.push_str(&format!("（节点 {}）", node_id));
    }
    if !flow_id.is_empty() {
        anchor.push_str(&format!("·流程 {}", flow_id));
    }
    if users.is_empty() {
        common::telemetry::warn!(
            "ApprovalCc {} resolvedUsers 为空——legacy 纯文本收件人，跳过通知投递",
            anchor
        );
        return;
    }
    let Some(messaging) = messaging.as_ref() else {
        common::telemetry::warn!(
            "ApprovalCc {} resolvedUsers 命中 {} 人但无 messaging 注入——降级仅记录，不投递",
            anchor,
            users.len()
        );
        return;
    };
    // 通知正文：流程名优先（DB 读失败不阻断，退回 flow_id 呈现）
    let flow_part = match flow_id.parse::<i64>() {
        Ok(fid) => {
            match sqlx::query_scalar::<_, String>(
                "SELECT notice FROM isahl.zc_id_process WHERE id = $1 AND deleted_at IS NULL",
            )
            .bind(fid)
            .fetch_optional(pool)
            .await
            {
                Ok(Some(name)) => name,
                _ => flow_id.clone(),
            }
        }
        Err(_) => flow_id.clone(),
    };
    let title = "审批抄送";
    let content = format!(
        "审批抄送：节点「{}」的审批已流转至流程「{}」，请你知悉跟进。",
        if label.is_empty() {
            "(未命名)"
        } else {
            &label
        },
        flow_part
    );
    for uid in users {
        notify_user(messaging, uid, title, &content).await;
    }
}

/// 订阅 ApprovalCc（Gateway 装配：event_bus 就绪后 spawn；messaging 复用 SLA
/// 注入的实例；None = 降级仅记录）。
pub fn subscribe_cc_notify(
    bus: Arc<dyn DomainEventBus>,
    pool: PgPool,
    messaging: Option<Arc<dyn MessagingService>>,
) {
    actix_web::rt::spawn(async move {
        let mut subscriber = match bus.subscribe("ApprovalCc").await {
            Ok(s) => s,
            Err(e) => {
                common::telemetry::error!("cc-notify: subscribe ApprovalCc failed: {}", e);
                return;
            }
        };
        loop {
            match subscriber.recv().await {
                Ok(evt) => handle_event(&pool, &messaging, evt).await,
                Err(_) => continue,
            }
        }
    });
}
