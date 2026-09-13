//! 日程提醒 handler（framework-scheduler 注册，plan_code=`schedule-reminder`）
//!
//! 提醒设置由**预警事件**承载（service.rs `apply_reminder`）：`zc_id_even-alert` 行
//! `code='schedule-reminder'`、`qk_date` → `zc_id_scal-date`（提醒时刻），经
//! `zc_id_plan_rr_event` 桥关联计划；`comments` 不承载提醒数据（原 JSON 载体已废除）。
//! 本 handler 周期性扫描到点计划 → 写 `zc_id_msgs-system` 站内信（叶表；收件人
//! created_by_id，复用 sla_timeout 站内信模式）。
//!
//! 触发条件：
//! - 计划存在提醒事件（桥 + 叶表 + scal-date），事件时刻 remind_at = 计划起始 − offset
//! - now ∈ [remind_at, 计划起始]（起始之后不再提醒），且尚未提醒过（幂等：marker 查重）

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use framework_scheduler::{ScheduledHandler, SchedulerContext, SchedulerError, SchedulerResult};
use sqlx::PgPool;

/// 计划 code（zc_id_plan 种子行；scheduler 装配处注册）
pub const SCHEDULE_REMINDER_PLAN_CODE: &str = "schedule-reminder";

/// 待提醒计划行（提醒时刻来自预警事件，非 comments）
#[derive(Debug, Clone, sqlx::FromRow)]
struct ReminderPlanRow {
    id: i64,
    notice: Option<String>,
    created_by_id: Option<i64>,
    remind_at: Option<DateTime<Utc>>,
    start_at: Option<DateTime<Utc>>,
}

/// 日程提醒 handler
pub struct ScheduleReminderHandler {
    pool: PgPool,
}

impl ScheduleReminderHandler {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// 单轮提醒检查（pub：集成测试直调）
    pub async fn check_and_remind(&self) -> Result<u64, SchedulerError> {
        // 候选：存在提醒事件（桥 → 叶表 code=schedule-reminder）+ 有起始时刻 + 未发过提醒
        let due: Vec<ReminderPlanRow> = sqlx::query_as(
            r#"
            SELECT p.id, p.notice, p.created_by_id,
                   (SELECT sd.date FROM isahl.zc_id_plan_rr_event rpe
                    JOIN isahl."zc_id_even-alert" re
                      ON re.id = rpe.ref_right AND re.deleted_at IS NULL
                    JOIN isahl."zc_id_scal-date" sd ON sd.id = re.qk_date
                    WHERE rpe.ref_left = p.id AND rpe.deleted_at IS NULL
                      AND re.code = 'schedule-reminder'
                    ORDER BY rpe.id DESC LIMIT 1) AS remind_at,
                   (SELECT ds.date_st FROM isahl."zc_id_segm-date" ds
                    WHERE ds.id = p."qk_date-segm" AND ds.deleted_at IS NULL) AS start_at
            FROM isahl.zc_id_plan p
            WHERE p.deleted_at IS NULL
              AND p.created_by_id IS NOT NULL
              AND EXISTS (
                  SELECT 1 FROM isahl.zc_id_plan_rr_event rpe2
                  JOIN isahl."zc_id_even-alert" re2
                    ON re2.id = rpe2.ref_right AND re2.deleted_at IS NULL
                  WHERE rpe2.ref_left = p.id AND rpe2.deleted_at IS NULL
                    AND re2.code = 'schedule-reminder' AND re2.qk_date IS NOT NULL
              )
              AND NOT EXISTS (
                  SELECT 1 FROM isahl."zc_id_msgs-system" m
                  WHERE m.ak_benefit_user @> ARRAY[p.created_by_id::bigint]
                    AND m.comments LIKE '%schedule-reminder:' || p.id::text || '%'
                    AND m.deleted_at IS NULL
              )
            LIMIT 100
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(SchedulerError::Database)?;

        let now = Utc::now();
        let mut sent = 0u64;

        for plan in due {
            let (Some(remind_at), Some(start_at)) = (plan.remind_at, plan.start_at) else {
                continue;
            };
            // 提醒窗口：now ∈ [提醒事件时刻, 计划起始]（起始之后不再提醒）
            if now < remind_at || now > start_at {
                continue;
            }
            let offset_min = (start_at - remind_at).num_minutes();

            let plan_id = plan.id;
            let user_id = plan.created_by_id.unwrap_or(common::SYSTEM_USER_ID);
            let title = plan.notice.unwrap_or_default();
            // 幂等标记：comments 含 "schedule-reminder:{plan_id}"
            let marker = format!("schedule-reminder:{plan_id}");
            let body = format!(
                "日程提醒：{title}（提前 {offset_min} 分钟）\n\n[{}]",
                marker
            );
            // 坐标三元组（§6.12 声明即必须）：msgs-system 族坐标（JB/GHC/↓_KC，与
            // inbox / scheduler 同表既有写入一致），值经 ontology_binding 解析 code→ZUID
            let (dk_scene, dk_factor, dk_function) =
                ontology_binding::resolve(&self.pool, ("JB", "GHC", "↓_KC"))
                    .await
                    .map_err(SchedulerError::Database)?;
            let _ = sqlx::query(
                r#"
                INSERT INTO isahl."zc_id_msgs-system" (notice, comments, created_by_id, ak_benefit_user, dk_scene, dk_factor, dk_function)
                VALUES ($1, $2, $3, ARRAY[$4::bigint], $5, $6, $7)
                "#,
            )
            .bind(format!("日程提醒：{title}"))
            .bind(body)
            .bind(common::SYSTEM_USER_ID)
            .bind(user_id)
            .bind(dk_scene)
            .bind(dk_factor)
            .bind(dk_function)
            .execute(&self.pool)
            .await
            .map_err(SchedulerError::Database)?;
            sent += 1;
            common::telemetry::info!(
                "[schedule-reminder] 计划 {} 提醒已发送（用户 {}）",
                plan_id,
                user_id
            );
        }
        Ok(sent)
    }
}

#[async_trait]
impl ScheduledHandler for ScheduleReminderHandler {
    fn plan_code(&self) -> &str {
        SCHEDULE_REMINDER_PLAN_CODE
    }

    async fn run(&self, _ctx: &SchedulerContext) -> Result<SchedulerResult, SchedulerError> {
        let sent = self.check_and_remind().await?;
        Ok(SchedulerResult {
            summary: format!("日程提醒检查：发送 {sent} 条"),
            processed: sent,
        })
    }
}
