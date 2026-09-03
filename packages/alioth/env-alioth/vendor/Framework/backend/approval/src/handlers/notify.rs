//! 审批动作通知投递（fix-approval-action-chain P2-8）
//!
//! 催办/转交/抄送从「只留痕」升级为「留痕 + 系统通知投递」。
//! 投递经 `MessagingService::send_system_notification`；发送失败仅 warn 不阻断
//! 动作主流程——通知是增强投递，不是状态契约。

use common::messaging::MessagingService;
use sqlx::PgPool;
use std::sync::Arc;

/// 解析实例当前审批人：fk_operator 优先，回退 fk_subject
/// （与 analytics approver_workloads 的 COALESCE 语义一致）。
pub(crate) async fn current_operator(pool: &PgPool, instance_id: i64) -> Option<i64> {
    sqlx::query_scalar::<_, Option<i64>>(
        r#"SELECT COALESCE(fk_operator, fk_subject) FROM isahl."zc_id_oper-approve"
           WHERE id = $1 AND deleted_at IS NULL"#,
    )
    .bind(instance_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
    .flatten()
}

/// 实例标题（通知正文定位用）
pub(crate) async fn instance_title(pool: &PgPool, instance_id: i64) -> Option<String> {
    sqlx::query_scalar::<_, Option<String>>(
        r#"SELECT notice FROM isahl."zc_id_oper-approve"
           WHERE id = $1 AND deleted_at IS NULL"#,
    )
    .bind(instance_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
    .flatten()
}

/// 投递系统通知；失败仅 warn 留痕，不阻断调用方主流程。
pub(crate) async fn notify_user(
    messaging: &Arc<dyn MessagingService>,
    to: i64,
    title: &str,
    content: &str,
) {
    if let Err(e) = messaging
        .send_system_notification(to as u64, title, content)
        .await
    {
        common::telemetry::warn!("approval 通知投递失败（to={}）: {}（不影响主流程）", to, e);
    }
}
