//! 注册审批自动通过 —— 配置项 `approval:auto-approve`
//!
//! 定位：注册/授权域审批的**免人工通道**。开关 = `isahl.zc_id_prot-env_config`
//! 行（`code = approval:auto-approve`，`settings.enabled`，模型级种子默认 `false`，
//! 读取器 `common::platform_config`，fail-closed）。
//!
//! 机制（与 `Framework/backend/approval/src/sla_timeout.rs` 同模板，均为
//! framework-scheduler 计划 handler）：
//! - 计划 `approval-auto-approve`（cron `*/1 * * * *`，模型级种子供给）；
//! - 每 tick 检查开关；关闭即返回（零查询副作用之外的写入）；
//! - 开启时扫**无终态**的注册类审批实例
//!   （`zc_id_oper-approve.code IN (user-register-approval, external-subject-register-approval)`），
//!   以系统身份走**与人工审批完全相同**的通过链路
//!   （[`crate::api::approvals::approve_instance`]：状态桥 → 激活 → 主体绑定 → 流程推进），
//!   意见留痕「自动审批通过（系统）」+ 审计事件 `approval.auto_approve`。
//!
//! 幂等与安全：
//! - 终态实例（approved/rejected）不在扫描集内，重复 tick 零处理；
//! - 实例 `fk_operator` 先归位系统用户再审批——`ApprovalService::execute` 的操作者鉴权
//!   要求 actor 与 `fk_operator` 一致，且归位使审批工作区可读地呈现「系统自动处理」；
//! - 开关默认关闭，读不到即关闭——MUST NOT 以「默认开启」兜底。

use crate::api::approvals::approve_instance;
use async_trait::async_trait;
use common::platform_config;
use common::SYSTEM_USER_ID;
use framework_scheduler::{ScheduledHandler, SchedulerContext, SchedulerError, SchedulerResult};
use sqlx::PgPool;

/// 调度计划 code（`zc_id_plan.code`，模型级种子行）
pub const AUTO_APPROVE_PLAN_CODE: &str = "approval-auto-approve";

/// 自动通过意见文本（留痕区分人工/自动；与 sla_timeout 的 `AUTO_REJECT_REASON` 对称）
const AUTO_APPROVE_OPINION: &str = "自动审批通过（系统）";

/// 单次 tick 处理上限（防大表长事务/长 tick；剩余实例由下一 tick 继续）
const BATCH_LIMIT: i64 = 200;

/// 注册审批自动通过 handler
///
/// bus：ApprovalCompleted 发布通道（fix-approval-gateway-event-publish）——
/// 与人工端点同链，终态事件驱动域订阅方（注册激活/员工入职等）。
pub struct AutoApproveHandler {
    pool: PgPool,
    bus: std::sync::Arc<dyn common::event_bus::DomainEventBus>,
}

impl AutoApproveHandler {
    pub fn new(pool: PgPool, bus: std::sync::Arc<dyn common::event_bus::DomainEventBus>) -> Self {
        Self { pool, bus }
    }

    /// 待自动通过的实例 id 集：注册类 code 且无终态主状态桥。
    async fn pending_registration_instances(&self) -> Result<Vec<i64>, sqlx::Error> {
        sqlx::query_scalar(
            r#"
            SELECT o.id
              FROM isahl."zc_id_oper-approve" o
             WHERE o.deleted_at IS NULL
               AND o.code IN ('user-register-approval', 'external-subject-register-approval')
               AND NOT EXISTS (
                     SELECT 1
                       FROM isahl."zc_id_lifecycle_r_primary-status" ps
                       JOIN isahl."zc_id_stus-approve" st ON st.id = ps.ref_right
                      WHERE ps.ref_left = o.id
                        AND ps.deleted_at IS NULL
                        AND st.code IN ('approved', 'rejected')
                   )
             ORDER BY o.id
             LIMIT $1
            "#,
        )
        .bind(BATCH_LIMIT)
        .fetch_all(&self.pool)
        .await
    }
}

#[async_trait]
impl ScheduledHandler for AutoApproveHandler {
    fn plan_code(&self) -> &str {
        AUTO_APPROVE_PLAN_CODE
    }

    async fn run(&self, ctx: &SchedulerContext) -> Result<SchedulerResult, SchedulerError> {
        if !platform_config::is_enabled(&self.pool, platform_config::AUTO_APPROVE_CODE).await {
            return Ok(SchedulerResult {
                summary: "自动审批通过开关关闭（approval:auto-approve）".to_string(),
                processed: 0,
            });
        }

        let instances = self
            .pending_registration_instances()
            .await
            .map_err(|e| SchedulerError::Internal(format!("扫描待自动通过实例失败: {e}")))?;

        let mut approved = 0u64;
        let mut failed = 0u64;
        for id in instances {
            // 操作者归位系统用户：execute 鉴权要求 actor == fk_operator；
            // 归位同时使审批工作区呈现「系统自动处理」而非悬空原操作者。
            if let Err(e) = sqlx::query(
                r#"UPDATE isahl."zc_id_oper-approve"
                      SET fk_operator = $1, updated_at = NOW()
                    WHERE id = $2 AND deleted_at IS NULL"#,
            )
            .bind(SYSTEM_USER_ID)
            .bind(id)
            .execute(&self.pool)
            .await
            {
                common::telemetry::warn!("自动审批通过：实例 {} 操作者归位失败: {}", id, e);
                failed += 1;
                continue;
            }

            let resp = approve_instance(
                &self.pool,
                id,
                Some(SYSTEM_USER_ID),
                Some(AUTO_APPROVE_OPINION.to_string()),
                Some(&self.bus),
            )
            .await;
            if resp.success {
                approved += 1;
                // 审计留痕（与 sla_timeout 的 sla.auto_reject 对称）
                let _ = common::audit::record_audit_event(
                    &self.pool,
                    SYSTEM_USER_ID,
                    "system@aliothstudio.local",
                    &format!("approval_instances:{id}"),
                    "approval.auto_approve",
                    &common::audit::Decision::Permit,
                )
                .await;
            } else {
                failed += 1;
                common::telemetry::warn!("自动审批通过：实例 {} 未通过: {}", id, resp.message);
            }
        }

        if failed > 0 {
            common::telemetry::warn!(
                "[{}] 自动审批通过：成功 {}，失败 {}",
                ctx.plan_code,
                approved,
                failed
            );
        }

        Ok(SchedulerResult {
            summary: format!("自动审批通过：成功 {approved}，失败 {failed}"),
            processed: approved,
        })
    }
}
