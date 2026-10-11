//! PEP 内部 seam — 审计主体解析与 fire-and-forget 审计投递（评审候选 5 分缝之一：audit）。
//!
//! 自 `pep/middleware.rs` 内部抽出；投递含义务携带（`obligations_triggered` 回执）。

use super::obligations::execute_obligation_ops;
use crate::epp::{record_audit_event_with_obligations, Decision as AuditDecision};
use common::telemetry::warn;
use sqlx::PgPool;

/// NGAC 审计主体标识（fix-ngac-audit-subject-identity）：**username 优先**；
/// 缺失/空 → `user:{user_id}`（保留唯一性，禁止跨主体共享常量占位）。
/// 服务令牌（`sub=client:*`）以 client 标识为主体。
pub(super) fn resolve_audit_subject(
    username: &str,
    user_id: i64,
    is_service_token: bool,
    service_id: &str,
) -> String {
    // 服务令牌主体 = client 标识（区别于自然人 username 空间）；
    // 自然人主体走跨审计面唯一实现（common::audit::resolve_subject）。
    if is_service_token && !service_id.is_empty() {
        return service_id.to_string();
    }
    common::audit::resolve_subject(username, user_id)
}

/// PEP 执法审计投递参数（义务携带版 spawn 的字段束——参数面不随字段增长膨胀）。
pub(super) struct AuditSpawn<'a> {
    pub(super) subject: &'a str,
    pub(super) email: &'a str,
    pub(super) resource: &'a str,
    pub(super) action: &'a str,
    pub(super) decision: AuditDecision,
    pub(super) obligations: Vec<String>,
}

/// 授权审计 fire-and-forget（NGAC_SPEC §8「每个决策产生审计（fire-and-forget，
/// 量级 = 请求率）」+ SECURITY_SPEC §11.2）：写库不阻塞请求响应，写入失败仅 telemetry
/// 记录、不改变响应结果。「已决策未落库」丢失窗口见 SECURITY_SPEC §11.2。
///
/// 主体标识 = `subject`（username 优先，回落 `user:{id}`，规则见 `resolve_audit_subject`）；
/// email 属联系方式，存在时随 `metadata.email` 保留（不作为标识）。
pub(super) fn spawn_audit_event(pool: PgPool, user_id: i64, ev: AuditSpawn<'_>) {
    let (subject, email, resource, action, decision, obligations) = (
        ev.subject,
        ev.email,
        ev.resource,
        ev.action,
        ev.decision,
        ev.obligations,
    );
    let subject = subject.to_string();
    let resource = resource.to_string();
    let action = action.to_string();
    let metadata = if email.is_empty() {
        serde_json::json!({})
    } else {
        serde_json::json!({ "email": email })
    };
    tokio::spawn(async move {
        // 先持有义务引用供执行面消费（写入任务内：audit:log 的执行 = 审计行本体）
        let ops_for_exec = obligations.clone();
        match record_audit_event_with_obligations(
            &pool,
            crate::epp::DecisionAudit {
                user_id,
                subject,
                object_path: resource.clone(),
                operation: action.clone(),
                decision,
                metadata,
                obligations,
            },
        )
        .await
        {
            Ok(()) => execute_obligation_ops(user_id, &resource, &ops_for_exec),
            Err(e) => warn!("NGAC audit event write failed: {}", e),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audit_subject_prefers_username_over_email() {
        // 唯一性标识 = username；即使 email 可用也不作为主体（fix-ngac-audit-subject-identity）
        assert_eq!(resolve_audit_subject("alice", 42, false, "42"), "alice");
    }

    #[test]
    fn audit_subject_falls_back_to_unique_user_id() {
        // username 缺失/空 → user:{id}（保唯一，不同用户不折叠）
        assert_eq!(resolve_audit_subject("", 42, false, "42"), "user:42");
        assert_eq!(resolve_audit_subject("   ", 42, false, "42"), "user:42");
        assert_ne!(
            resolve_audit_subject("", 42, false, "42"),
            resolve_audit_subject("", 43, false, "43"),
            "回落值必须对每个主体唯一"
        );
    }

    #[test]
    fn audit_subject_service_token_uses_client_identity() {
        // 服务令牌主体为 client 标识（区别于自然人 username 空间）
        assert_eq!(
            resolve_audit_subject("", 7, true, "client:demo"),
            "client:demo"
        );
    }
}
