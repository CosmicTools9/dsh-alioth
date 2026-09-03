//! SLA 超时自动处理
//!
//! 定时检查审批实例是否超过 SLA 时限，超时自动驳回。
//! SLA 通过 `qk_sla → zc_id_scal-duration.mark` 获取小时数（o_number 为
//! 触发器自动生成的业务编号，不承载数值）。
//!
//! 质量保障与审计语义（业务关注，非基础设施轮询）：
//! - 作为 `framework-scheduler` 的 `ApprovalSlaHandler` 注册（plan_code=`approval-sla-timeout`），
//!   调度循环写 `zc_id_oper-planing` 执行实例（业务可查询的质量活动记录）。
//! - 自动驳回经 `record_audit_event`（sla.auto_reject，SYSTEM_USER_ID 归属）审计留痕；
//!   驳回/超时数据可供 quality 模块 SLA 统计（node-durations/bottlenecks）查询。
//!
//! 系统身份：以 `SYSTEM_USER_ID`（1）为审计归属，经 NGAC 授权 + 审计监管。
//! 装配：Gateway main.rs 经 `SchedulerService::register` 注册，调度循环驱动。

use crate::handlers::approve_reject::{publish_approval_completed, update_lifecycle_status};
use crate::handlers::notify::{instance_title, notify_user};
use async_trait::async_trait;
use common::audit::{record_audit_event, Decision};
use common::event_bus::DomainEventBus;
use common::permissions::require_resource_access;
use common::SYSTEM_USER_ID;
use framework_scheduler::{ScheduledHandler, SchedulerContext, SchedulerError, SchedulerResult};
use sqlx::PgPool;
use std::sync::Arc;

/// 计划 code（zc_id_plan-perform 种子行）
pub const SLA_PLAN_CODE: &str = "approval-sla-timeout";
/// 驳回终态动作码——与人工驳回（approve_reject.rs REJECT_NOTICE）同一动作码，
/// 状态派生（fk_list = 实例 id + notice CASE）才能正确呈现 rejected。
const REJECT_NOTICE: &str = "审批驳回";
/// 自动驳回原因（写入 opinion 字段，留痕区分人工/自动）
const AUTO_REJECT_REASON: &str = "SLA 超时自动驳回";

/// SLA 超时自动驳回 handler（framework-scheduler 注册）
pub struct ApprovalSlaHandler {
    pool: PgPool,
    bus: Arc<dyn DomainEventBus>,
    /// 升级通知投递（可选注入；None 时跳过通知不阻断驳回）
    messaging: Option<Arc<dyn common::messaging::MessagingService>>,
}

impl ApprovalSlaHandler {
    pub fn new(pool: PgPool, bus: Arc<dyn DomainEventBus>) -> Self {
        Self {
            pool,
            bus,
            messaging: None,
        }
    }

    /// 注入消息服务（fix-approval-engine-semantics D7：SLA 超时升级通知）。
    /// 可选：未注入时跳过升级通知，驳回主流程不受影响。
    pub fn with_messaging(
        mut self,
        messaging: Arc<dyn common::messaging::MessagingService>,
    ) -> Self {
        self.messaging = Some(messaging);
        self
    }
}

/// SLA 超时升级通知（fix-approval-engine-gap-closure D6 实装）：只投递**上级岗位**
/// 成员——升级目标读实例节点载体 `timeline.escalateTo`（publish 从设计器人工节点
/// 配置物化），成员经收敛解析（`common::ngac_org::resolve_member_user_ids`：
/// 指派 UA ∪ 岗位直管/任职持有者——认知派生读侧并入，NGAC_SPEC §2.2.3 同源）
/// 逐人投递；admin UA 是平台管理身份，不作为业务升级目标。
/// 未配置 escalateTo 或目标岗位无活跃成员 → 不投递，仅 warn 留痕（无目标即静默）。
/// 投递失败仅 warn——升级通知是增强投递，不是状态契约。
async fn escalate_timeout(
    pool: &PgPool,
    messaging: &Arc<dyn common::messaging::MessagingService>,
    instance_id: i64,
    node_id: i64,
) {
    let escalate_role: Option<String> = sqlx::query_scalar::<_, Option<String>>(
        r#"SELECT NULLIF(ea.timeline->>'escalateTo', '')
             FROM isahl."zc_id_even-approve" ea
            WHERE ea.id = $1 AND ea.deleted_at IS NULL
            LIMIT 1"#,
    )
    .bind(node_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
    .flatten()
    .filter(|r| !r.trim().is_empty());
    let Some(role) = escalate_role else {
        common::telemetry::warn!(
            "SLA 超时实例 {}：节点 {} 未配置 escalateTo 升级岗位，跳过升级通知（仅留痕）",
            instance_id,
            node_id
        );
        return;
    };
    // 升级岗位成员：收敛解析（指派 UA ∪ 岗位直管/任职持有者——common::ngac_org
    // 认知派生读侧并入，NGAC_SPEC §2.2.3 同源）
    let members: Vec<i64> = match pool.acquire().await {
        Ok(mut conn) => common::ngac_org::resolve_member_user_ids(&mut conn, &role, 200).await,
        Err(_) => Vec::new(),
    };
    if members.is_empty() {
        common::telemetry::warn!(
            "SLA 超时实例 {}：升级岗位 '{}' 无活跃成员，跳过升级通知（仅留痕）",
            instance_id,
            role
        );
        return;
    }
    let title = "审批超时升级";
    let label = instance_title(pool, instance_id)
        .await
        .unwrap_or_else(|| format!("审批实例 {}", instance_id));
    let content = format!(
        "审批实例「{}」已超 SLA 未处理，按配置升级至 {} 岗位，请跟进处理（节点 {}）",
        label, role, node_id
    );
    for uid in members {
        notify_user(messaging, uid, title, &content).await;
    }
}

#[async_trait]
impl ScheduledHandler for ApprovalSlaHandler {
    fn plan_code(&self) -> &str {
        SLA_PLAN_CODE
    }

    async fn run(&self, _ctx: &SchedulerContext) -> Result<SchedulerResult, SchedulerError> {
        check_and_reject_with(&self.pool, &self.bus, self.messaging.as_ref())
            .await
            .map_err(|_| SchedulerError::Internal("sla check failed".to_string()))?;
        Ok(SchedulerResult {
            summary: "审批 SLA 超时检查完成（超时实例已自动驳回）".to_string(),
            processed: 1,
        })
    }
}

/// 查询并处理超时实例
/// `pub`：集成测试（tests/）直接调用验证轮询链路
pub async fn check_and_reject(
    pool: &PgPool,
    bus: &Arc<dyn DomainEventBus>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    check_and_reject_with(pool, bus, None).await
}

/// 带升级通知的完整链路（fix-approval-engine-gap-closure D6）：messaging 注入时，
/// 超时自动驳回后按节点载体 timeline.escalateTo 岗位投递升级通知（未配置/无成员
/// 仅 warn 留痕）；None 时行为与原 check_and_reject 一致。
pub async fn check_and_reject_with(
    pool: &PgPool,
    bus: &Arc<dyn DomainEventBus>,
    messaging: Option<&Arc<dyn common::messaging::MessagingService>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let overdue = sqlx::query_as::<_, (i64,)>(
        r#"
        SELECT i.id
        FROM isahl."zc_id_oper-approve" i
        JOIN isahl.zc_id_operation_rr_event oe ON oe.ref_left = i.id AND oe.deleted_at IS NULL
        JOIN isahl."zc_id_even-approve" ev
          ON ev.id = oe.ref_right AND ev.deleted_at IS NULL
        JOIN isahl."zc_id_scal-duration" sd ON sd.id = ev.qk_sla
        WHERE i.deleted_at IS NULL
          AND NOT EXISTS (
              SELECT 1 FROM isahl."zc_id_lifecycle_r_primary-status" ls
              JOIN isahl."zc_id_stus-approve" s ON s.id = ls.ref_right
              WHERE ls.ref_left = i.id AND ls.deleted_at IS NULL
                AND s.code IN ('approved', 'rejected', 'withdrawn', 'cancelled', 'abstained')
          )
          AND i.created_at + (sd.mark * INTERVAL '1 hour') < NOW()
        LIMIT 100
    "#,
    )
    .fetch_all(pool)
    .await
    .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

    for (id,) in &overdue {
        let fk: Option<i64> = sqlx::query_scalar(
            r#"SELECT oe.ref_right FROM isahl.zc_id_operation_rr_event oe
               WHERE oe.ref_left = $1 AND oe.deleted_at IS NULL
               ORDER BY oe.created_at LIMIT 1"#,
        )
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?
        .flatten();

        // NGAC 授权：系统身份校验写权限（显式授权，未授权即拒绝）
        if let Err(e) = require_resource_access(
            pool,
            SYSTEM_USER_ID,
            "approval_actions",
            fk.unwrap_or(0),
            "create",
        )
        .await
        {
            common::telemetry::warn!("SLA auto-reject denied by NGAC for instance {}: {}", id, e);
            let _ = record_audit_event(
                pool,
                SYSTEM_USER_ID,
                "system@aliothstudio.local",
                &format!("approval_actions:{}", fk.unwrap_or(0)),
                "sla.auto_reject",
                &Decision::Deny,
            )
            .await;
            continue;
        }

        // 时间锚（flow-process-continuity 规约）：SLA 驳回意见写当日标量；解析失败仅跳过该实例
        let date_anchor = match crate::handlers::approve_reject::today_date_anchor(pool).await {
            Ok(v) => v,
            Err(e) => {
                common::telemetry::warn!(
                    "SLA auto-reject date anchor failed for instance {}: {}",
                    id,
                    e
                );
                continue;
            }
        };

        match sqlx::query(
            r#"INSERT INTO isahl."zc_id_deta-opinion"
               (id, notice, opinion, fk_list, fk_biller, qk_date, created_at)
               VALUES (isahl.gen_next_zuid(), $1, $2, $3, $4, $5, NOW())"#,
        )
        .bind(REJECT_NOTICE)
        .bind(AUTO_REJECT_REASON)
        .bind(*id)
        .bind(SYSTEM_USER_ID)
        .bind(date_anchor)
        .execute(pool)
        .await
        {
            Ok(_) => {
                // 操作级审计：系统自动驳回成功
                let _ = record_audit_event(
                    pool,
                    SYSTEM_USER_ID,
                    "system@aliothstudio.local",
                    &format!("approval_instances:{}", id),
                    "sla.auto_reject",
                    &Decision::Permit,
                )
                .await;

                // D6：与 reject 一致链路——主状态桥接 rejected + 事件发布
                if let Err(e) =
                    update_lifecycle_status(pool, *id, "rejected", "已拒绝", SYSTEM_USER_ID).await
                {
                    common::telemetry::error!(
                        "SLA auto-reject status update failed for instance {}: {}",
                        id,
                        e
                    );
                } else {
                    // ApprovalCompleted 事件（result=rejected / reason=SLA 超时）
                    publish_approval_completed(bus, pool, *id, "rejected", Some("SLA 超时")).await;
                    // 2026-09-02 A3：vote 实例 SLA 驳回同属终态动作 → 终局判定
                    if let Err(e) =
                        crate::advance::vote_terminal_advance(pool, *id, SYSTEM_USER_ID, Some(bus))
                            .await
                    {
                        common::telemetry::warn!(
                            "vote terminal advance after SLA reject failed for instance {}: {}",
                            id,
                            e
                        );
                    }

                    // D7 升级通知：admin 成员可见性（失败仅 warn，不阻断）
                    if let Some(m) = messaging {
                        escalate_timeout(pool, m, *id, fk.unwrap_or(0)).await;
                    }
                }
            }
            Err(e) => {
                common::telemetry::error!(
                    "SLA auto-reject write failed for instance {}: {}",
                    id,
                    e
                );
            }
        }
    }

    Ok(())
}
