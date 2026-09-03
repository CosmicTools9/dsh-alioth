//! 任务到期引擎（framework-scheduler 注册，plan_code=`task-deadline-check`）
//!
//! 时间对称模型的任务侧驱动：事件引擎监听过去切片（event），本 handler 推进
//! 未来切片（task）——扫描 `zc_id_task` 中 qk_period（→ zc_id_segm-date.date_ed）
//! 已到期且未完成的任务，写操作痕迹（oper-planing）+ 站内信提醒
//! （`zc_id_msgs-system` 叶表）——任务到期触发动作，补齐任务驱动引擎。

use crate::{ScheduledHandler, SchedulerContext, SchedulerError, SchedulerResult};
use async_trait::async_trait;
use sqlx::PgPool;

/// 计划 code（zc_id_plan-perform 种子行）
pub const TASK_DEADLINE_PLAN_CODE: &str = "task-deadline-check";

/// 到期任务行
#[derive(Debug, Clone, sqlx::FromRow)]
struct DueTaskRow {
    id: i64,
    notice: Option<String>,
    created_by_id: Option<i64>,
}

/// 任务到期 handler
pub struct TaskDeadlineHandler {
    pool: PgPool,
}

impl TaskDeadlineHandler {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// 单轮到期检查（pub：集成测试直调）。返回（到期数, 通知数）。
    pub async fn check_and_notify(&self) -> Result<(u64, u64), SchedulerError> {
        // 到期未完成：qk_period → segm-date.date_ed < now，且无 completed 主状态
        let due: Vec<DueTaskRow> = sqlx::query_as(
            r#"
            SELECT t.id, t.notice, t.created_by_id
            FROM isahl.zc_id_task t
            JOIN isahl."zc_id_segm-date" ds ON ds.id = t.qk_period AND ds.deleted_at IS NULL
            WHERE t.deleted_at IS NULL
              AND t.qk_period IS NOT NULL
              AND ds.date_ed < NOW()
              AND NOT EXISTS (
                  SELECT 1 FROM isahl."zc_id_lifecycle_r_primary-status" ps
                  JOIN isahl."zc_id_stus-event" st ON st.id = ps.ref_right
                  WHERE ps.ref_left = t.id AND ps.deleted_at IS NULL
                    AND st.notice = '完成' AND st.flag = 'end'
              )
              AND NOT EXISTS (
                  SELECT 1 FROM isahl."zc_id_msgs-system" m
                  WHERE m.ak_benefit_user @> ARRAY[COALESCE(t.created_by_id, 1)::bigint]
                    AND m.comments LIKE '%task-deadline:' || t.id::text || '%'
                    AND m.deleted_at IS NULL
              )
            LIMIT 100
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(SchedulerError::Database)?;

        let total = due.len() as u64;
        let mut notified = 0u64;

        for task in due {
            let user_id = task.created_by_id.unwrap_or(common::SYSTEM_USER_ID);
            let title = task.notice.unwrap_or_default();

            // 到期动作痕迹（oper-planing：任务到期的操作记录——任务驱动引擎的 trigger 产物）
            let op_id: Option<i64> = sqlx::query_scalar(
                r#"INSERT INTO isahl."zc_id_oper-planing"
                   (notice, code, fk_subject, fk_operator, created_by_id)
                   VALUES ($1, $2, $3, $4, $4) RETURNING id"#,
            )
            .bind(format!("任务到期：{title}"))
            .bind(format!("task-due-{}", task.id))
            .bind(task.id)
            .bind(common::SYSTEM_USER_ID)
            .fetch_one(&self.pool)
            .await
            .map_err(SchedulerError::Database)
            .ok();

            // 动作归属任务（operation_rr_task 正桥）
            if let Some(oid) = op_id {
                let _ = sqlx::query(
                    r#"INSERT INTO isahl."zc_id_operation_rr_task" (notice, ref_left, ref_right, created_by_id)
                       VALUES ($1, $2, $3, $4)"#,
                )
                .bind(format!("task-due：{title}"))
                .bind(oid)
                .bind(task.id)
                .bind(common::SYSTEM_USER_ID)
                .execute(&self.pool)
                .await;
            }

            // 站内信提醒（msgs-system 叶表；marker 幂等）
            let marker = format!("task-deadline:{}", task.id);
            let _ = sqlx::query(
                r#"INSERT INTO isahl."zc_id_msgs-system" (notice, comments, created_by_id, ak_benefit_user)
                   VALUES ($1, $2, $3, ARRAY[$4::bigint])"#,
            )
            .bind(format!("任务已到期：{title}"))
            .bind(format!("任务「{title}」已到期，请处理。\n\n[{}]", marker))
            .bind(common::SYSTEM_USER_ID)
            .bind(user_id)
            .execute(&self.pool)
            .await;
            notified += 1;
            common::telemetry::info!(
                "[task-deadline] 任务 {} 到期已通知（用户 {}）",
                task.id,
                user_id
            );
        }
        Ok((total, notified))
    }
}

#[async_trait]
impl ScheduledHandler for TaskDeadlineHandler {
    fn plan_code(&self) -> &str {
        TASK_DEADLINE_PLAN_CODE
    }

    async fn run(&self, _ctx: &SchedulerContext) -> Result<SchedulerResult, SchedulerError> {
        let (total, notified) = self.check_and_notify().await?;
        Ok(SchedulerResult {
            summary: format!("任务到期检查：{total} 项到期，通知 {notified} 人"),
            processed: notified,
        })
    }
}
