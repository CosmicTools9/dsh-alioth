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
    /// 当前主状态 code（zc_id_stus-task；无桥行 = None → 初始态 PENDING）
    status_code: Option<String>,
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
        // 到期未完成：qk_period → segm-date.date_ed < now，且未达完成/取消终态。
        // 完成判定（fix-avic-pm-residual-gaps 修正）：任务实际字典 zc_id_stus-task
        // （TASK-DONE/TASK-CANCELLED）；保留 stus-event「完成/end」判定为 OR 兼容
        // （历史口径——避免其他 ns 存量任务行为回归）。
        // 站内信去重 marker 移至行级判定（此前在 SQL 内排除会导致「已提醒但未迁移
        // OVERDUE」的存量任务永远错过状态迁移）。
        let due: Vec<DueTaskRow> = sqlx::query_as(
            r#"
            SELECT t.id, t.notice, t.created_by_id,
                   (SELECT s.code FROM isahl."zc_id_lifecycle_r_primary-status" ps
                    JOIN isahl."zc_id_stus-task" s ON s.id = ps.ref_right AND s.deleted_at IS NULL
                    WHERE ps.ref_left = t.id AND ps.deleted_at IS NULL LIMIT 1) AS status_code
            FROM isahl.zc_id_task t
            JOIN isahl."zc_id_segm-date" ds ON ds.id = t.qk_period AND ds.deleted_at IS NULL
            WHERE t.deleted_at IS NULL
              AND t.qk_period IS NOT NULL
              AND ds.date_ed < NOW()
              AND NOT EXISTS (
                  SELECT 1 FROM isahl."zc_id_lifecycle_r_primary-status" ps
                  JOIN isahl."zc_id_stus-task" st ON st.id = ps.ref_right
                  WHERE ps.ref_left = t.id AND ps.deleted_at IS NULL
                    AND st.deleted_at IS NULL
                    AND st.code IN ('TASK-DONE', 'TASK-CANCELLED')
              )
              AND NOT EXISTS (
                  SELECT 1 FROM isahl."zc_id_lifecycle_r_primary-status" ps
                  JOIN isahl."zc_id_stus-event" st ON st.id = ps.ref_right
                  WHERE ps.ref_left = t.id AND ps.deleted_at IS NULL
                    AND st.deleted_at IS NULL
                    AND st.notice = '完成' AND st.flag = 'end'
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

            // 状态迁移（系统驱动）：PENDING（含无桥初始态）到期 → TASK-OVERDUE；
            // ACTIVE（已启动/逾期重启）不回写——保住「逾期重启后可继续工作」。
            // 字典未播种 TASK-OVERDUE（其他 ns）→ warn 跳过，不阻断提醒。
            if task.status_code.as_deref().unwrap_or("TASK-PENDING") == "TASK-PENDING" {
                self.transition_overdue(task.id).await;
            }

            // 站内信提醒（msgs-system 叶表；marker 幂等——已提醒过则跳过）
            let marker = format!("task-deadline:{}", task.id);
            let already: bool = sqlx::query_scalar(
                r#"SELECT EXISTS (
                       SELECT 1 FROM isahl."zc_id_msgs-system" m
                       WHERE m.ak_benefit_user @> ARRAY[$1::bigint]
                         AND m.comments LIKE '%task-deadline:' || $2::text || '%'
                         AND m.deleted_at IS NULL)"#,
            )
            .bind(user_id)
            .bind(task.id)
            .fetch_one(&self.pool)
            .await
            .unwrap_or(false);
            if already {
                continue;
            }

            // 坐标三元组（§6.12 声明即必须）：值经 ontology_binding 解析 code→ZUID，禁硬编码 ZUID
            let (op_dk_scene, op_dk_factor, op_dk_function) =
                ontology_binding::resolve(&self.pool, ("JE", "FMA", "↓_CH"))
                    .await
                    .map_err(SchedulerError::Database)?;
            let (msg_dk_scene, msg_dk_factor, msg_dk_function) =
                ontology_binding::resolve(&self.pool, ("JB", "GHC", "↓_KC"))
                    .await
                    .map_err(SchedulerError::Database)?;

            // 到期动作痕迹（oper-planing：任务到期的操作记录——任务驱动引擎的 trigger 产物）
            let op_id: Option<i64> = sqlx::query_scalar(
                r#"INSERT INTO isahl."zc_id_oper-planing"
                   (notice, code, fk_subject, fk_operator, created_by_id, dk_scene, dk_factor, dk_function)
                   VALUES ($1, $2, $3, $4, $4, $5, $6, $7) RETURNING id"#,
            )
            .bind(format!("任务到期：{title}"))
            .bind(format!("task-due-{}", task.id))
            .bind(task.id)
            .bind(common::SYSTEM_USER_ID)
            .bind(op_dk_scene)
            .bind(op_dk_factor)
            .bind(op_dk_function)
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

            let _ = sqlx::query(
                r#"INSERT INTO isahl."zc_id_msgs-system" (notice, comments, created_by_id, ak_benefit_user, dk_scene, dk_factor, dk_function)
                   VALUES ($1, $2, $3, ARRAY[$4::bigint], $5, $6, $7)"#,
            )
            .bind(format!("任务已到期：{title}"))
            .bind(format!("任务「{title}」已到期，请处理。\n\n[{}]", marker))
            .bind(common::SYSTEM_USER_ID)
            .bind(user_id)
            .bind(msg_dk_scene)
            .bind(msg_dk_factor)
            .bind(msg_dk_function)
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

    /// PENDING → TASK-OVERDUE 系统迁移（UPDATE 单行 / 无行 INSERT，同事务语义
    /// 与 orchestration TaskStatusRepository 一致；失败仅 warn 不阻断）。
    async fn transition_overdue(&self, task_id: i64) {
        let overdue_id: Option<i64> = sqlx::query_scalar(
            r#"SELECT id FROM isahl."zc_id_stus-task"
               WHERE code = 'TASK-OVERDUE' AND deleted_at IS NULL LIMIT 1"#,
        )
        .fetch_optional(&self.pool)
        .await
        .ok()
        .flatten();
        let Some(overdue_id) = overdue_id else {
            common::telemetry::warn!(
                "[task-deadline] 字典 zc_id_stus-task 缺 TASK-OVERDUE——任务 {} 跳过状态迁移（提醒不受影响）",
                task_id
            );
            return;
        };
        let updated = sqlx::query(
            r#"UPDATE isahl."zc_id_lifecycle_r_primary-status"
               SET ref_right = $1, updated_by_id = $2, updated_at = NOW()
               WHERE ref_left = $3 AND deleted_at IS NULL"#,
        )
        .bind(overdue_id)
        .bind(common::SYSTEM_USER_ID)
        .bind(task_id)
        .execute(&self.pool)
        .await
        .map(|r| r.rows_affected())
        .unwrap_or(0);
        if updated == 0 {
            let _ = sqlx::query(
                r#"INSERT INTO isahl."zc_id_lifecycle_r_primary-status"
                   (id, ref_left, ref_right, created_by_id)
                   VALUES (isahl.gen_next_zuid(), $1, $2, $3)"#,
            )
            .bind(task_id)
            .bind(overdue_id)
            .bind(common::SYSTEM_USER_ID)
            .execute(&self.pool)
            .await;
        }
        common::telemetry::info!("[task-deadline] 任务 {} 到期迁移 TASK-OVERDUE", task_id);
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
