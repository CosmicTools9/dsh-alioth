//! 日程提醒 handler（framework-scheduler 注册，plan_code=`schedule-reminder`）
//!
//! S1 闭环：提醒设置（reminder_offset_min）落 plan comments JSON（service.rs
//! apply_reminder_to_comments），本 handler 周期性扫描到点计划 → 写 `zc_id_msgs-system`
//! 站内信（叶表；收件人 created_by_id，复用 sla_timeout 站内信模式）。
//!
//! 触发条件：
//! - plan.comments 含 `reminder_offset_min`（0..=1440 分钟）
//! - plan 开始时间（qk_date-segm → zc_id_segm-date.date_st/time_st）已到
//!   `now >= start - offset` 且尚未提醒过（幂等：同 plan 同分钟不重复）
//!
//! 容错约定：`zc_id_plan.comments` 被多业务共用——schedule 写 JSON
//! （`{"reminder_offset_min": N}`），AVIC 等业务写纯文本备注。故 SELECT 禁止
//! `comments::jsonb` cast（任一行非 JSON 会使整查询失败，历史事故），只做
//! LIKE 粗筛，JSON 解析在 Rust 侧容错（`serde_json::from_str` 失败即跳过）。

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use framework_scheduler::{ScheduledHandler, SchedulerContext, SchedulerError, SchedulerResult};
use sqlx::PgPool;

/// 计划 code（zc_id_plan 种子行；scheduler 装配处注册）
pub const SCHEDULE_REMINDER_PLAN_CODE: &str = "schedule-reminder";

/// 待提醒计划行
#[derive(Debug, Clone, sqlx::FromRow)]
struct ReminderPlanRow {
    id: i64,
    notice: Option<String>,
    comments: Option<String>,
    created_by_id: Option<i64>,
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
        // 候选：comments 含 reminder_offset_min + 开始时间已到提醒窗口 + 未发过提醒
        let due: Vec<ReminderPlanRow> = sqlx::query_as(
            r#"
            SELECT p.id, p.notice, p.comments, p.created_by_id,
                   (SELECT ds.date_st FROM isahl."zc_id_segm-date" ds
                    WHERE ds.id = p."qk_date-segm" AND ds.deleted_at IS NULL) AS start_at
            FROM isahl.zc_id_plan p
            WHERE p.deleted_at IS NULL
              AND p.comments IS NOT NULL
              AND p.comments LIKE '%"reminder_offset_min"%'
              AND p.created_by_id IS NOT NULL
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
            let Some(raw_comments) = plan.comments.as_deref() else {
                continue;
            };
            let Ok(v) = serde_json::from_str::<serde_json::Value>(raw_comments) else {
                continue;
            };
            let Some(offset_min) = v.get("reminder_offset_min").and_then(|x| x.as_i64()) else {
                continue;
            };
            let Some(start_at) = plan.start_at else {
                continue;
            };
            // 提醒窗口：now 在 [start - offset, start] 区间（start 后不再提醒）
            let remind_at = start_at - chrono::Duration::minutes(offset_min);
            if now < remind_at || now > start_at {
                continue;
            }

            let plan_id = plan.id;
            let user_id = plan.created_by_id.unwrap_or(common::SYSTEM_USER_ID);
            let title = plan.notice.unwrap_or_default();
            // 幂等标记：comments 含 "schedule-reminder:{plan_id}"
            let marker = format!("schedule-reminder:{plan_id}");
            let body = format!(
                "日程提醒：{title}（提前 {offset_min} 分钟）\n\n[{}]",
                marker
            );
            let _ = sqlx::query(
                r#"
                INSERT INTO isahl."zc_id_msgs-system" (notice, comments, created_by_id, ak_benefit_user)
                VALUES ($1, $2, $3, ARRAY[$4::bigint])
                "#,
            )
            .bind(format!("日程提醒：{title}"))
            .bind(body)
            .bind(common::SYSTEM_USER_ID)
            .bind(user_id)
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
