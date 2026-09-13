//! Schedule Service
//!
//! 日程管理服务层，基于 zc_id_plan + zc_id_event 双表模型。
//! 核心查询通过 zc_id_plan_rr_event 关联 Plan 与 Event，并 JOIN 场所/主体/审批表。
//!
//! 迁移自 Framework/backend/schedule/src/service.rs
//!
//! 聚合查询边界（P1-5 预研结论）：list_schedule_items 属跨表多跳聚合
//! （plan→plan_rr_event→event→place/subjects/even-approve→segm-date + done
//! 子查询 + COALESCE 派生列），_refs 机制（HasReferenceJoins 单实体单跳）不覆盖；
//! convention_checker 确立「CRUD 归 list_refs/get_refs、聚合查询归 service 直写 SQL」
//! 分界。维持现状，勿重构为 _refs。

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use sqlx::{AssertSqlSafe, FromRow, PgPool};

use super::models::*;

/// 提醒事件识别码（`zc_id_even-alert.code`）：日程提醒 = 计划起始前的预警事件。
///
/// 载体（零 DDL，全既有结构）：`zc_id_even-alert`（叶表）行 `qk_date` → `zc_id_scal-date`
/// 承载提醒时刻，`zc_id_plan_rr_event`（ref_left=计划）桥关联；`comments` 不承载任何提醒数据。
pub(crate) const REMINDER_EVENT_CODE: &str = "schedule-reminder";

/// 提醒换算：`offset = 计划起始时刻 − 提醒事件时刻`（分钟）。
/// 合法域 0..=1440（越界视为无提醒，与 DTO 校验域一致）。
fn reminder_from_times(
    start_at: DateTime<Utc>,
    remind_at: DateTime<Utc>,
) -> Option<ReminderResponse> {
    let offset = (start_at - remind_at).num_minutes();
    (0..=1440).contains(&offset).then(|| ReminderResponse {
        offset: offset as i32,
        channel: "app".to_string(),
    })
}

// ============================================
// Errors
// ============================================

#[derive(Debug, thiserror::Error)]
pub enum ScheduleError {
    #[error("Schedule item not found: {0}")]
    NotFound(i64),
    #[error("Validation error: {0}")]
    Validation(String),
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("Internal error: {0}")]
    Internal(String),
}

impl actix_web::ResponseError for ScheduleError {
    fn error_response(&self) -> actix_web::HttpResponse {
        match self {
            ScheduleError::NotFound(id) => actix_web::HttpResponse::NotFound()
                .json(serde_json::json!({"error": "not_found", "message": format!("Schedule item not found: {}", id) })),
            ScheduleError::Validation(msg) => actix_web::HttpResponse::BadRequest()
                .json(serde_json::json!({"error": "validation_error", "message": msg })),
            ScheduleError::Database(e) => actix_web::HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": "database_error", "message": e.to_string() })),
            ScheduleError::Internal(msg) => actix_web::HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": "internal_error", "message": msg })),
        }
    }
}

/// 参与人查询行（替代元组以规避 sqlx Type 约束）
#[derive(Debug, Clone, FromRow)]
pub(crate) struct ParticipantRow {
    pub id: i64,
    pub notice: Option<String>,
    pub resp_type: Option<serde_json::Value>,
}

/// 事件客体查询行（替代元组以规避 sqlx Type 约束）
#[derive(Debug, Clone, FromRow)]
pub(crate) struct EventObjectRow {
    pub id: i64,
    pub notice: Option<String>,
}

/// 原始日程项行（list_schedule_items 查询直接映射）
#[derive(Debug, Clone, FromRow)]
pub struct RawScheduleItem {
    pub plan_id: i64,
    pub plan_notice: Option<String>,
    pub plan_type: Option<String>,
    pub plan_cron: Option<String>,
    pub plan_comments: Option<String>,
    /// 提醒事件时刻（预警事件 `qk_date` → `zc_id_scal-date.date`，经 `zc_id_plan_rr_event` 桥）
    pub remind_at: Option<chrono::DateTime<chrono::Utc>>,
    pub event_id: Option<i64>,
    pub event_fk_place: Option<i64>,
    pub event_fk_subject: Option<i64>,
    pub place_name: Option<String>,
    pub subject_name: Option<String>,
    pub approval_status: Option<String>,
    pub approval_title: Option<String>,
    pub segm_date_st: Option<chrono::DateTime<chrono::Utc>>,
    pub segm_date_ed: Option<chrono::DateTime<chrono::Utc>>,
    pub segm_time_st: Option<chrono::NaiveTime>,
    pub segm_time_ed: Option<chrono::NaiveTime>,
    pub done: bool,
}

/// 原始待办项行（list_todo_items 查询直接映射）
#[derive(Debug, Clone, FromRow)]
pub(crate) struct RawTodoItem {
    pub event_id: i64,
    pub event_notice: Option<String>,
    pub subject_name: Option<String>,
    pub status_notice: Option<String>,
    pub status_flag: Option<String>,
}

// ============================================
// Repository
// ============================================

/// 待办行判定谓词（用户可见的待办行）——徽标计数与待办清单共用的**唯一**来源。
///
/// `get_pending_todo_count`（`/schedule/overview` 徽标计数）与
/// `list_todo_items`（`/schedule/todos` 待办清单）MUST 共用本片段：
/// 两者曾各自维护过滤条件而漂移——2026-09-09 仅在清单侧排除系统翻转事实
/// （`record_slice_flip` 产物：`code` 前缀 `flip-`、`notice` 形如「完成：实体状态推进」），
/// 徽标仍把对账切片计为待办（AVIC-CAASEC/pre 实证：徽标 8 / 清单可见行 0）。
/// 任何行级过滤条件的新增或修改只改此处，禁止单侧加条件。
///
/// 依赖调用方 FROM 中的别名：`e`（`zc_id_even-alert`）。
const TODO_ROW_PREDICATE: &str = r#"
              AND e.deleted_at IS NULL
              AND e.code NOT LIKE 'flip-%'
              AND e.notice IS NOT NULL AND e.notice <> ''"#;

/// 「未完成」状态判定——**仅**徽标计数追加，不用于清单：
/// 清单须保留已完成行以支撑面板「显示已完成」展开与 `completedCount/totalCount` 头部统计
/// （防误勾后无处恢复），未完成由前端 `items.filter(!done)` 呈现。
///
/// 契约：**徽标数 == 清单中 `done == false` 的行数**。
/// 展开为三值逻辑安全形式：无主状态行（`st` 为 NULL）时 `NOT(...)` 得 NULL
/// 会被 WHERE 排除，导致未完成待办漏计（曾返回 0）。
/// 依赖调用方 FROM 中的别名：`st`（`zc_id_stus-event`）。
const TODO_PENDING_PREDICATE: &str = r#"
              AND (st.notice IS NULL OR st.notice <> '完成' OR st.flag <> 'end')"#;

/// 日程项多跳聚合 SELECT（[`ScheduleRepository::list_schedule_items`] /
/// [`ScheduleRepository::get_schedule_item`] 共用，禁止各自复制——列与聚合语义漂移会让
/// 单条读与列表读出现字段级分歧）。
///
/// 语义：plan → plan_rr_event（关联事件 + 提醒事件）→ event → place/subjects/even-approve
/// → segm-date（`COALESCE("qk_date-segm", "qk_time-segm")`），含 approval_status / done 派生列；
/// 计划维度有且仅有一行（两个 `LEFT JOIN LATERAL ... LIMIT 1`）。
/// 行级过滤由调用方在 `WHERE p.deleted_at IS NULL` 之后以参数绑定追加。
const SCHEDULE_ITEM_SELECT: &str = r#"SELECT
                p.id as plan_id,
                p.notice as plan_notice,
                p.code as plan_type,
                p.cron::text as plan_cron,
                p.comments as plan_comments,
                rm.remind_at as remind_at,
                e.id as event_id,
                e.fk_place as event_fk_place,
                e.fk_subject as event_fk_subject,
                pl.notice as place_name,
                s.notice as subject_name,
                (SELECT st.code FROM isahl."zc_id_lifecycle_r_primary-status" aps
                 JOIN isahl."zc_id_stus-approve" st ON st.id = aps.ref_right
                 WHERE aps.ref_left = e.id AND aps.deleted_at IS NULL
                   AND st.deleted_at IS NULL
                 ORDER BY aps.id DESC LIMIT 1) as approval_status,
                a.notice as approval_title,
                ds.date_st as segm_date_st,
                ds.date_ed as segm_date_ed,
                ds.time_st as segm_time_st,
                ds.time_ed as segm_time_ed,
                (SELECT COUNT(*) > 0 FROM isahl."zc_id_lifecycle_r_primary-status" ps
                 JOIN isahl."zc_id_stus-plan" st ON st.id = ps.ref_right
                 WHERE ps.ref_left = p.id AND ps.deleted_at IS NULL
                   AND st.code = 'completed' AND st.deleted_at IS NULL) as done
            FROM isahl.zc_id_plan p
            LEFT JOIN LATERAL (
                SELECT ev.id, ev.fk_place, ev.fk_subject
                FROM isahl.zc_id_plan_rr_event pre
                JOIN isahl.zc_id_event ev ON ev.id = pre.ref_right AND ev.deleted_at IS NULL
                WHERE pre.ref_left = p.id AND pre.deleted_at IS NULL
                  -- 提醒事件（code=schedule-reminder）语义是提醒，不参与「关联事件」选取
                  AND NOT EXISTS (
                      SELECT 1 FROM isahl."zc_id_even-alert" rmv
                      WHERE rmv.id = ev.id AND rmv.code = 'schedule-reminder'
                  )
                ORDER BY pre.id DESC LIMIT 1
            ) e ON true
            LEFT JOIN isahl.zc_id_place pl ON pl.id = e.fk_place
            LEFT JOIN isahl.zc_id_subjects s ON s.id = e.fk_subject
            LEFT JOIN isahl."zc_id_even-approve" a ON a.id = e.id
            LEFT JOIN LATERAL (
                SELECT sd.date AS remind_at
                FROM isahl.zc_id_plan_rr_event rpe
                JOIN isahl."zc_id_even-alert" re ON re.id = rpe.ref_right AND re.deleted_at IS NULL
                JOIN isahl."zc_id_scal-date" sd ON sd.id = re.qk_date
                WHERE rpe.ref_left = p.id AND rpe.deleted_at IS NULL
                  AND re.code = 'schedule-reminder'
                ORDER BY rpe.id DESC LIMIT 1
            ) rm ON true
            LEFT JOIN isahl."zc_id_segm-date" ds ON ds.id = COALESCE(p."qk_date-segm", p."qk_time-segm")
            WHERE p.deleted_at IS NULL"#;

#[derive(Clone)]
pub struct ScheduleRepository {
    pool: PgPool,
}

impl ScheduleRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub fn from_arc(pool: std::sync::Arc<PgPool>) -> Self {
        Self {
            pool: (*pool).clone(),
        }
    }

    // ----- Plan + Event 组装查询 -----

    /// 列出日程项（Plan + 关联 Event + segm-date 跨度 + done 状态）
    pub async fn list_schedule_items(
        &self,
        query: &ScheduleListQuery,
        visible_ids: Option<&[i64]>,
    ) -> Result<Vec<RawScheduleItem>, sqlx::Error> {
        let mut sql = String::from(SCHEDULE_ITEM_SELECT);

        // 用户输入一律参数绑定，禁止 format! 拼接（SQL 注入）；qk_date-segm 是标量引用 ID（bigint）
        enum Param {
            Id(i64),
            IdArray(Vec<i64>),
            Text(String),
        }
        let mut params: Vec<Param> = Vec::new();
        let mut param_idx = 1usize;
        // RLS（wire-schedule-rls）：NGAC 可见 plan ID 集 → p.id = ANY($n)
        if let Some(ids) = visible_ids {
            if !ids.is_empty() {
                sql.push_str(&format!(" AND p.id = ANY(${})", param_idx));
                params.push(Param::IdArray(ids.to_vec()));
                param_idx += 1;
            }
        }
        if let Some(v) = query.qk_date_segm {
            sql.push_str(&format!(" AND p.\"qk_date-segm\" = ${}", param_idx));
            params.push(Param::Id(v));
            param_idx += 1;
        }
        if let Some(v) = query.start_date_segm {
            sql.push_str(&format!(" AND p.\"qk_date-segm\" >= ${}", param_idx));
            params.push(Param::Id(v));
            param_idx += 1;
        }
        if let Some(v) = query.end_date_segm {
            sql.push_str(&format!(" AND p.\"qk_date-segm\" <= ${}", param_idx));
            params.push(Param::Id(v));
            param_idx += 1;
        }
        if let Some(v) = &query._t_ {
            sql.push_str(&format!(" AND p.code = ${}", param_idx));
            params.push(Param::Text(v.clone()));
            param_idx += 1;
        }
        if let Some(done_filter) = query.done {
            // done filter via subquery — plans without a completed primary-status are not done
            if done_filter {
                sql.push_str(
                    r#" AND EXISTS (
                        SELECT 1 FROM isahl."zc_id_lifecycle_r_primary-status" ps
                        JOIN isahl."zc_id_stus-plan" st ON st.id = ps.ref_right
                        WHERE ps.ref_left = p.id AND ps.deleted_at IS NULL
                          AND st.code = 'completed' AND st.deleted_at IS NULL
                    )"#,
                );
            } else {
                sql.push_str(
                    r#" AND NOT EXISTS (
                        SELECT 1 FROM isahl."zc_id_lifecycle_r_primary-status" ps
                        JOIN isahl."zc_id_stus-plan" st ON st.id = ps.ref_right
                        WHERE ps.ref_left = p.id AND ps.deleted_at IS NULL
                          AND st.code = 'completed' AND st.deleted_at IS NULL
                    )"#,
                );
            }
        }
        sql.push_str(
            " ORDER BY ds.date_st ASC NULLS LAST, ds.time_st ASC NULLS LAST, p.sort ASC NULLS LAST",
        );
        // LIMIT/OFFSET 同样参数绑定
        let limit_param = param_idx;
        let offset_param = param_idx + 1;
        sql.push_str(&format!(" LIMIT ${} OFFSET ${}", limit_param, offset_param));

        let mut q = sqlx::query_as(AssertSqlSafe(sql.as_str()));
        for p in &params {
            match p {
                Param::Id(v) => q = q.bind(*v),
                Param::IdArray(v) => q = q.bind(v.clone()),
                Param::Text(v) => q = q.bind(v.as_str()),
            }
        }
        q = q.bind(query.limit).bind(query.offset);
        q.fetch_all(&self.pool).await
    }

    /// 按计划 ID 精确取单条日程项（`ScheduleService::find_item` 的取数路径）。
    ///
    /// 与 [`Self::list_schedule_items`] 共用 [`SCHEDULE_ITEM_SELECT`]，仅追加
    /// `AND p.id = $1`——修复「`list_items(limit = 1)` + Rust 侧 `find`」对非首行计划
    /// 恒 `None` 的缺陷（Gateway `GET /schedule/items/{id}` 与
    /// `POST /schedule/items/{id}/event` 前置校验因此误判 404/失败）。
    ///
    /// 软删除（`deleted_at IS NOT NULL`）计划返回 `None`；计划维度唯一一行，
    /// 无需 ORDER BY。
    pub async fn get_schedule_item(
        &self,
        plan_id: i64,
    ) -> Result<Option<RawScheduleItem>, sqlx::Error> {
        let sql = format!("{} AND p.id = $1", SCHEDULE_ITEM_SELECT);
        sqlx::query_as(AssertSqlSafe(sql.as_str()))
            .bind(plan_id)
            .fetch_optional(&self.pool)
            .await
    }

    /// 获取单个日程项的参与人列表
    pub(crate) async fn get_participants(
        &self,
        plan_id: i64,
    ) -> Result<Vec<ParticipantRow>, sqlx::Error> {
        sqlx::query_as(
            r#"SELECT 
                s.id,
                s.notice,
                pp."resp-type" as resp_type
            FROM isahl.zc_id_plan_rr_participants pp
            JOIN isahl.zc_id_subjects s ON s.id = pp.ref_right
            WHERE pp.ref_left = $1"#,
        )
        .bind(plan_id)
        .fetch_all(&self.pool)
        .await
    }

    // ----- Plan CRUD -----

    pub async fn find_plan_by_id(&self, id: i64) -> Result<Option<Plan>, sqlx::Error> {
        let sql = format!(
            "SELECT {} FROM isahl.zc_id_plan WHERE id = $1 AND deleted_at IS NULL",
            Plan::SELECT_FIELDS
        );
        sqlx::query_as(AssertSqlSafe(sql.as_str()))
            .bind(id)
            .fetch_optional(&self.pool)
            .await
    }

    // ----- 提醒（预警事件路线）-----

    /// 读取计划提醒：提醒事件时刻 → `offset = 计划起始 − 事件时刻`（分钟）；无事件/无起始 → None
    pub(crate) async fn read_reminder(
        &self,
        plan_id: i64,
    ) -> Result<Option<ReminderResponse>, sqlx::Error> {
        let row: Option<(Option<DateTime<Utc>>, Option<DateTime<Utc>>)> = sqlx::query_as(
            r#"SELECT ds.date_st,
                      (SELECT sd.date FROM isahl.zc_id_plan_rr_event rpe
                       JOIN isahl."zc_id_even-alert" re
                         ON re.id = rpe.ref_right AND re.deleted_at IS NULL
                       JOIN isahl."zc_id_scal-date" sd ON sd.id = re.qk_date
                       WHERE rpe.ref_left = p.id AND rpe.deleted_at IS NULL
                         AND re.code = $2
                       ORDER BY rpe.id DESC LIMIT 1) AS remind_at
               FROM isahl.zc_id_plan p
               LEFT JOIN isahl."zc_id_segm-date" ds
                 ON ds.id = COALESCE(p."qk_date-segm", p."qk_time-segm")
               WHERE p.id = $1 AND p.deleted_at IS NULL"#,
        )
        .bind(plan_id)
        .bind(REMINDER_EVENT_CODE)
        .fetch_optional(&self.pool)
        .await?;
        Ok(
            row.and_then(|(start_at, remind_at)| match (start_at, remind_at) {
                (Some(start), Some(at)) => reminder_from_times(start, at),
                _ => None,
            }),
        )
    }

    /// 计划起始时刻（`qk_date-segm`/`qk_time-segm` → `zc_id_segm-date.date_st`）；无 → None
    async fn plan_start_at(&self, plan_id: i64) -> Result<Option<DateTime<Utc>>, sqlx::Error> {
        let start_at: Option<DateTime<Utc>> = sqlx::query_scalar::<_, Option<DateTime<Utc>>>(
            r#"SELECT ds.date_st FROM isahl.zc_id_plan p
               LEFT JOIN isahl."zc_id_segm-date" ds
                 ON ds.id = COALESCE(p."qk_date-segm", p."qk_time-segm")
               WHERE p.id = $1 AND p.deleted_at IS NULL"#,
        )
        .bind(plan_id)
        .fetch_optional(&self.pool)
        .await?
        .flatten();
        Ok(start_at)
    }

    /// 写入/清除提醒（`reminder_offset_min` ↔ 预警事件 `qk_date`）；comments 零触碰。
    ///
    /// - `Some(n)`（n>0）：事件 `qk_date` = 计划起始 − n 分钟（缺起始 → 报错，不静默降级）
    /// - `Some(0)`：清除（软删事件与桥；与前端 `reminder.none` 语义一致）
    /// - `None`：不修改
    pub(crate) async fn apply_reminder(
        &self,
        plan_id: i64,
        reminder_offset_min: Option<i32>,
    ) -> Result<(), sqlx::Error> {
        let Some(offset) = reminder_offset_min else {
            return Ok(());
        };
        let now = Utc::now();
        let existing: Option<(i64, i64)> = sqlx::query_as(
            r#"SELECT rpe.id, re.id FROM isahl.zc_id_plan_rr_event rpe
               JOIN isahl."zc_id_even-alert" re ON re.id = rpe.ref_right AND re.deleted_at IS NULL
               WHERE rpe.ref_left = $1 AND rpe.deleted_at IS NULL AND re.code = $2
               ORDER BY rpe.id DESC LIMIT 1"#,
        )
        .bind(plan_id)
        .bind(REMINDER_EVENT_CODE)
        .fetch_optional(&self.pool)
        .await?;

        if offset <= 0 {
            if let Some((bridge_id, event_id)) = existing {
                sqlx::query(
                    r#"UPDATE isahl.zc_id_plan_rr_event SET deleted_at = $1, updated_at = $1 WHERE id = $2"#,
                )
                .bind(now)
                .bind(bridge_id)
                .execute(&self.pool)
                .await?;
                sqlx::query(
                    r#"UPDATE isahl."zc_id_even-alert" SET deleted_at = $1, updated_at = $1 WHERE id = $2"#,
                )
                .bind(now)
                .bind(event_id)
                .execute(&self.pool)
                .await?;
            }
            return Ok(());
        }
        if offset > 1440 {
            return Err(sqlx::Error::Protocol(format!(
                "reminder_offset_min 超出合法域 0..=1440: {offset}"
            )));
        }

        let start_at = self.plan_start_at(plan_id).await?.ok_or_else(|| {
            sqlx::Error::Protocol(
                "reminder_offset_min 需要计划起始时刻（qk_date-segm/qk_time-segm → zc_id_segm-date.date_st）"
                    .to_string(),
            )
        })?;
        let remind_at = start_at - chrono::Duration::minutes(offset as i64);

        // 提醒时刻 → zc_id_scal-date 标量行（存在即复用，缺失则新建）
        let scal_id: Option<i64> = sqlx::query_scalar(
            r#"SELECT id FROM isahl."zc_id_scal-date" WHERE date = $1 ORDER BY id LIMIT 1"#,
        )
        .bind(remind_at)
        .fetch_optional(&self.pool)
        .await?;
        let scal_id =
            match scal_id {
                Some(id) => id,
                None => sqlx::query_scalar(
                    r#"INSERT INTO isahl."zc_id_scal-date" (notice, date, created_at, updated_at)
                       VALUES ($1, $2, $3, $3) RETURNING id"#,
                )
                .bind(format!("日程提醒 {}", remind_at.format("%Y-%m-%d %H:%M")))
                .bind(remind_at)
                .bind(now)
                .fetch_one(&self.pool)
                .await?,
            };

        match existing {
            Some((_bridge_id, event_id)) => {
                sqlx::query(
                    r#"UPDATE isahl."zc_id_even-alert" SET qk_date = $1, updated_at = $2 WHERE id = $3"#,
                )
                .bind(scal_id)
                .bind(now)
                .bind(event_id)
                .execute(&self.pool)
                .await?;
            }
            None => {
                let notice: Option<String> = sqlx::query_scalar::<_, Option<String>>(
                    r#"SELECT notice FROM isahl.zc_id_plan WHERE id = $1"#,
                )
                .bind(plan_id)
                .fetch_optional(&self.pool)
                .await?
                .flatten();
                // 叶表坐标（§6.12）：值经 ontology_binding 解析 code→ZUID（禁硬编码 ZUID）
                let (dk_scene, dk_factor, dk_function) =
                    ontology_binding::resolve(&self.pool, ("JE", "FBB", "↓_EE")).await?;
                let event_id: i64 = sqlx::query_scalar(
                    r#"INSERT INTO isahl."zc_id_even-alert" (notice, code, qk_date, created_at, updated_at, dk_scene, dk_factor, dk_function)
                       VALUES ($1, $2, $3, $4, $4, $5, $6, $7) RETURNING id"#,
                )
                .bind(format!("日程提醒：{}", notice.unwrap_or_default()))
                .bind(REMINDER_EVENT_CODE)
                .bind(scal_id)
                .bind(now)
                .bind(dk_scene)
                .bind(dk_factor)
                .bind(dk_function)
                .fetch_one(&self.pool)
                .await?;
                sqlx::query(
                    r#"INSERT INTO isahl.zc_id_plan_rr_event (ref_left, ref_right, code, created_at, updated_at)
                       VALUES ($1, $2, $3, $4, $4)"#,
                )
                .bind(plan_id)
                .bind(event_id)
                .bind(REMINDER_EVENT_CODE)
                .bind(now)
                .execute(&self.pool)
                .await?;
            }
        }
        Ok(())
    }

    pub async fn create_plan(&self, req: &CreatePlanRequest) -> Result<Plan, sqlx::Error> {
        let now = Utc::now();
        // 前端 QuickAdd 字段解析（workspace-dock 契约）：
        // title||notice → notice；type||code → code；日期时间 → segm 标量
        let notice = req.notice.clone().or_else(|| req.title.clone());
        let code = req.code.clone().or_else(|| req.r#type.clone());

        // 日期时间 → zc_id_segm-date 标量（仅当前端字段存在且未显式给 qk_date_segm 时）
        let mut qk_date_segm = req.qk_date_segm;
        let mut qk_time_segm = req.qk_time_segm;
        if qk_date_segm.is_none()
            && (req.date_start.is_some()
                || req.date_end.is_some()
                || req.time_start.is_some()
                || req.time_end.is_some())
        {
            qk_date_segm = Some(
                self.resolve_or_create_date_segm(
                    req.date_start.as_deref(),
                    req.date_end.as_deref(),
                    req.time_start.as_deref(),
                    req.time_end.as_deref(),
                )
                .await?,
            );
            qk_time_segm = qk_date_segm; // 时间并入同一 segm 行
        }

        // 根据业务类型路由到对应叶表
        // meeting → zc_id_thre-meeting，其余兜底到 zc_id_plan-personal
        let table = match code.as_deref() {
            Some("meeting") => r#"isahl."zc_id_thre-meeting""#,
            _ => r#"isahl."zc_id_plan-personal""#,
        };
        // _t_ / _f_ 由 dk_scene/dk_factor/dk_function 坐标触发器自动赋值
        let sql = format!(
            r#"INSERT INTO {table}
                (notice, code, "qk_date-segm", "qk_time-segm", cron, exclude, sort, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $8)
            RETURNING {}"#,
            Plan::SELECT_FIELDS
        );
        let plan: Plan = sqlx::query_as(AssertSqlSafe(sql.as_str()))
            .bind(&notice)
            .bind(&code)
            .bind(qk_date_segm)
            .bind(qk_time_segm)
            .bind(&req.cron)
            .bind(&req.exclude)
            .bind(req.sort)
            .bind(now)
            .fetch_one(&self.pool)
            .await?;
        // 提醒设置 → 预警事件（comments 零触碰）
        self.apply_reminder(plan.id, req.reminder_offset_min)
            .await?;
        Ok(plan)
    }

    /// 解析前端日期时间字符串为 `zc_id_segm-date` 标量行 id。
    ///
    /// 按 (date_st, date_ed, time_st, time_ed) 四字段精确查重（复用既有行），
    /// 无则 INSERT。空字段不参与匹配。日期格式 "YYYY-MM-DD"、时间格式 "HH:MM"，
    /// 解析失败返回 sqlx::Error（由调用方映射为 400）。
    pub async fn resolve_or_create_date_segm(
        &self,
        date_start: Option<&str>,
        date_end: Option<&str>,
        time_start: Option<&str>,
        time_end: Option<&str>,
    ) -> Result<i64, sqlx::Error> {
        // 解析并规范化为可比较字符串（date: "YYYY-MM-DD" / time: "HH:MM"）
        let parse_field = |col: &str, v: Option<&str>| -> Result<Option<String>, sqlx::Error> {
            match v {
                Some(raw) => {
                    let parsed = match col {
                        "time_st" | "time_ed" => chrono::NaiveTime::parse_from_str(raw, "%H:%M")
                            .map(|t| t.format("%H:%M").to_string()),
                        _ => chrono::NaiveDate::parse_from_str(raw, "%Y-%m-%d")
                            .map(|d| d.format("%Y-%m-%d").to_string()),
                    }
                    .map_err(|e| {
                        sqlx::Error::Protocol(format!("invalid {} '{}': {}", col, raw, e))
                    })?;
                    Ok(Some(parsed))
                }
                None => Ok(None),
            }
        };

        let ds = parse_field("date_st", date_start)?;
        let de = parse_field("date_ed", date_end)?;
        let ts = parse_field("time_st", time_start)?;
        let te = parse_field("time_ed", time_end)?;

        let all_none = ds.is_none() && de.is_none() && ts.is_none() && te.is_none();

        // 查重：非空字段参与匹配
        if !all_none {
            let mut n = 1usize;
            let mut dyn_sql =
                String::from(r#"SELECT id FROM isahl."zc_id_segm-date" WHERE deleted_at IS NULL"#);
            let mut bind_values: Vec<Option<&str>> = Vec::new();
            for (col, val) in [
                ("date_st", ds.as_deref()),
                ("date_ed", de.as_deref()),
                ("time_st", ts.as_deref()),
                ("time_ed", te.as_deref()),
            ] {
                if let Some(v) = val {
                    // date 列存 timestamp → 比较日期部分；time 列比较时间部分
                    let extract = match col {
                        "time_st" | "time_ed" => "::time::text",
                        _ => "::date::text",
                    };
                    dyn_sql.push_str(&format!(" AND {}{} = ${}", col, extract, n));
                    n += 1;
                    bind_values.push(Some(v));
                }
            }
            let mut query = sqlx::query_scalar::<_, i64>(AssertSqlSafe(dyn_sql.as_str()));
            for bv in &bind_values {
                query = query.bind(*bv);
            }
            if let Some(id) = query.fetch_optional(&self.pool).await? {
                return Ok(id);
            }
        }

        // 无匹配（或全空）→ 新建：date 列 timestamp、time 列 time
        let date_to_ts = |s: Option<String>| {
            s.and_then(|d| chrono::NaiveDate::parse_from_str(&d, "%Y-%m-%d").ok())
                .map(|d| d.and_hms_opt(0, 0, 0).unwrap_or_default())
        };
        let time_to_time =
            |s: Option<String>| s.and_then(|t| chrono::NaiveTime::parse_from_str(&t, "%H:%M").ok());
        let id: i64 = sqlx::query_scalar(
            r#"INSERT INTO isahl."zc_id_segm-date"
               (notice, date_st, date_ed, time_st, time_ed, created_at, updated_at)
               VALUES ('日程', $1, $2, $3, $4, NOW(), NOW()) RETURNING id"#,
        )
        .bind(date_to_ts(ds))
        .bind(date_to_ts(de))
        .bind(time_to_time(ts))
        .bind(time_to_time(te))
        .fetch_one(&self.pool)
        .await?;
        Ok(id)
    }

    pub async fn update_plan(
        &self,
        id: i64,
        req: &UpdatePlanRequest,
    ) -> Result<Option<Plan>, sqlx::Error> {
        let now = Utc::now();
        let sql = format!(
            r#"UPDATE isahl.zc_id_plan SET
                notice = COALESCE($1, notice),
                code = COALESCE($2, code),
                "qk_date-segm" = COALESCE($3, "qk_date-segm"),
                "qk_time-segm" = COALESCE($4, "qk_time-segm"),
                cron = COALESCE($5::text, cron),
                exclude = COALESCE($6::json, exclude),
                sort = COALESCE($7, sort),
                updated_at = $8
            WHERE id = $9 AND deleted_at IS NULL
            RETURNING {}"#,
            Plan::SELECT_FIELDS
        );
        let updated: Option<Plan> = sqlx::query_as(AssertSqlSafe(sql.as_str()))
            .bind(&req.notice)
            .bind(&req.code)
            .bind(req.qk_date_segm)
            .bind(req.qk_time_segm)
            .bind(&req.cron)
            .bind(&req.exclude)
            .bind(req.sort)
            .bind(now)
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        // 提醒设置 → 预警事件（comments 零触碰）
        if let Some(plan) = &updated {
            self.apply_reminder(plan.id, req.reminder_offset_min)
                .await?;
        }
        Ok(updated)
    }

    pub async fn delete_plan(&self, id: i64) -> Result<u64, sqlx::Error> {
        let now = Utc::now();
        let result = sqlx::query(
            "UPDATE isahl.zc_id_plan SET deleted_at = $1 WHERE id = $2 AND deleted_at IS NULL",
        )
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    /// Toggle plan done status via primary-status relationship
    /// Design: D6 — 主状态行（ref_left 单列唯一）已指向 completed → 软删（unmark）；
    /// 否则 ref_left 锚定 upsert 置为 completed（活行覆盖/软删行 restore/无行 INSERT，
    /// 与 approval/inbox 写路径同语义，规避 ref_left 唯一约束 duplicate key）
    pub async fn toggle_plan_done(&self, id: i64) -> Result<Option<Plan>, sqlx::Error> {
        let now = Utc::now();

        // 存在性检查：id 不是活跃 plan 时禁止写入 ref_left 域（handler 会分流到
        // toggle_event_done 处理 event id）。此前缺失该检查导致 plan 路径与 event
        // 路径双重写入、unmark 永远无法生效（两侧 is_done 判定用的状态 id 不同表）。
        if self.find_plan_by_id(id).await?.is_none() {
            return Ok(None);
        }

        // Step 1: find the "completed" status record in zc_id_stus-plan
        //   与 ApprovalService 同一先例：状态记录缺失时自动补种（zc_id 生命周期链，
        //   gen_next_zuid 合规）；避免功能因种子数据缺失而静默失效。
        let completed_status_id: Option<i64> = sqlx::query_scalar(
            r#"SELECT id FROM isahl."zc_id_stus-plan"
               WHERE code = 'completed' AND deleted_at IS NULL
               LIMIT 1"#,
        )
        .fetch_optional(&self.pool)
        .await?;

        let status_id = match completed_status_id {
            Some(id) => id,
            None => {
                sqlx::query_scalar::<_, i64>(
                    r#"INSERT INTO isahl."zc_id_stus-plan" (id, code, notice)
                       VALUES (isahl.gen_next_zuid(), 'completed', '已完成') RETURNING id"#,
                )
                .fetch_one(&self.pool)
                .await?
            }
        };

        // Step 2: 读取当前主状态行。ref_left 单列唯一（uq_zc_id_lifecycle_r_primary-status_ref_left）：
        // 一个实体至多一条主状态记录，与 approval/inbox 写路径同一语义。按 (ref_left, ref_right)
        // 判定 exists 会在「活行指向其他状态」时误判为未完成，随后 INSERT 撞 ref_left 唯一约束
        // （duplicate key → 500）。
        let current: Option<(i64, Option<DateTime<Utc>>)> = sqlx::query_as(
            r#"SELECT ref_right, deleted_at FROM isahl."zc_id_lifecycle_r_primary-status"
               WHERE ref_left = $1"#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        // 已完成判定：主状态行存在、未软删且指向 completed
        let is_done = matches!(&current, Some((ref_right, None)) if *ref_right == status_id);

        // Step 3: toggle — is_done means "done", soft-delete the relationship (unmark done)
        //        not done means "not done", update/insert the relationship (mark done)
        if is_done {
            // Unmark done: soft-delete the primary-status relationship
            sqlx::query(
                r#"UPDATE isahl."zc_id_lifecycle_r_primary-status"
                   SET deleted_at = $1
                   WHERE ref_left = $2 AND ref_right = $3 AND deleted_at IS NULL"#,
            )
            .bind(now)
            .bind(id)
            .bind(status_id)
            .execute(&self.pool)
            .await?;

            // ADR D-010：主状态撤销审计（失败不回滚业务）
            if let Err(e) =
                crud::audit_outbox::audit_primary_status_delete(&self.pool, id, status_id, None)
                    .await
            {
                common::telemetry::warn!(
                    "audit_primary_status_delete enqueue failed (plan {}): {}",
                    id,
                    e
                );
            }

            // P5 执行实例化：撤销完成也是可审计的执行事实（fail-open，不阻断业务）
            if let Err(e) = common::plan_execution::record_plan_execution(
                &self.pool,
                id,
                "日程撤销完成（unmark done）",
                None,
                common::SYSTEM_USER_ID,
            )
            .await
            {
                common::telemetry::warn!("plan_execution record failed (plan {}): {}", id, e);
            }
        } else {
            // Mark done: 按 ref_left 锚定 upsert——活行覆盖 ref_right（UPDATE 不产生新行，
            // 规避 ref_left 单列唯一约束冲突）、软删行 restore、无行 INSERT。
            let updated = sqlx::query(
                r#"UPDATE isahl."zc_id_lifecycle_r_primary-status"
                   SET deleted_at = NULL, ref_right = $1, updated_at = $2
                   WHERE ref_left = $3
                   RETURNING id"#,
            )
            .bind(status_id)
            .bind(now)
            .bind(id)
            .execute(&self.pool)
            .await?;

            if updated.rows_affected() == 0 {
                // 无任何主状态行，插入新记录
                let insert = sqlx::query(
                    r#"INSERT INTO isahl."zc_id_lifecycle_r_primary-status"
                       (ref_left, ref_right, created_at, updated_at)
                       VALUES ($1, $2, $3, $3)"#,
                )
                .bind(id)
                .bind(status_id)
                .bind(now)
                .execute(&self.pool)
                .await;

                match insert {
                    Ok(_) => {}
                    Err(sqlx::Error::Database(db_err)) if db_err.is_unique_violation() => {
                        // 并发竞态兜底：另一事务已先落 ref_left 行（双击/并行 toggle 或
                        // 其他写路径）→ 降级为覆盖 UPDATE，避免 duplicate key 500
                        sqlx::query(
                            r#"UPDATE isahl."zc_id_lifecycle_r_primary-status"
                               SET deleted_at = NULL, ref_right = $1, updated_at = $2
                               WHERE ref_left = $3 AND deleted_at IS NULL"#,
                        )
                        .bind(status_id)
                        .bind(now)
                        .bind(id)
                        .execute(&self.pool)
                        .await?;
                    }
                    Err(e) => return Err(e),
                }
            }

            // ADR D-010：主状态标记审计（restore/insert 统一记为状态建立）
            if let Err(e) =
                crud::audit_outbox::audit_primary_status(&self.pool, id, None, status_id, None)
                    .await
            {
                common::telemetry::warn!(
                    "audit_primary_status enqueue failed (plan {}): {}",
                    id,
                    e
                );
            }

            // P5 执行实例化：完成动作留执行事实（fail-open）
            if let Err(e) = common::plan_execution::record_plan_execution(
                &self.pool,
                id,
                "日程标记完成（mark done）",
                None,
                common::SYSTEM_USER_ID,
            )
            .await
            {
                common::telemetry::warn!("plan_execution record failed (plan {}): {}", id, e);
            }

            // 时间对称完备：切片翻转（task→event）——完成落 even-alert 事件 +
            // plan_rr_event 关联（过去切片可回溯；fail-open）
            if let Err(e) = common::plan_execution::record_slice_flip(
                &self.pool,
                id,
                None,
                "日程完成",
                common::SYSTEM_USER_ID,
            )
            .await
            {
                common::telemetry::warn!("slice_flip failed (plan {}): {}", id, e);
            }
        }

        // Return updated plan
        self.find_plan_by_id(id).await
    }

    /// Toggle event done status via primary-status relationship
    /// /schedule/todos 为 event-centric：前端待办 checkbox 传 event id 到 /toggle。
    /// 语义与 toggle_plan_done 一致（ref_left 锚定 upsert + unique_violation 兜底）；
    /// 完成状态按读路径判定（notice='完成' AND flag='end'），缺失时补种（同 plan 先例）。
    pub async fn toggle_event_done(&self, id: i64) -> Result<Option<Event>, sqlx::Error> {
        // event 必须存在且未软删（否则返回 None → handler 404）
        let event_exists: bool = sqlx::query_scalar(
            r#"SELECT COUNT(*) > 0 FROM isahl.zc_id_event
               WHERE id = $1 AND deleted_at IS NULL"#,
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await?;
        if !event_exists {
            return Ok(None);
        }

        let now = Utc::now();

        // Step 1: 完成状态行（读路径判定 notice='完成' AND flag='end'；缺失补种）
        let completed_status_id: Option<i64> = sqlx::query_scalar(
            r#"SELECT id FROM isahl."zc_id_stus-event"
               WHERE notice = '完成' AND flag = 'end' AND deleted_at IS NULL
               LIMIT 1"#,
        )
        .fetch_optional(&self.pool)
        .await?;

        let status_id = match completed_status_id {
            Some(id) => id,
            None => {
                sqlx::query_scalar::<_, i64>(
                    r#"INSERT INTO isahl."zc_id_stus-event" (id, code, notice, flag)
                       VALUES (isahl.gen_next_zuid(), 'completed', '完成', 'end')
                       RETURNING id"#,
                )
                .fetch_one(&self.pool)
                .await?
            }
        };

        // Step 2: 当前主状态行（ref_left 单列唯一语义）
        let current: Option<(i64, Option<DateTime<Utc>>)> = sqlx::query_as(
            r#"SELECT ref_right, deleted_at FROM isahl."zc_id_lifecycle_r_primary-status"
               WHERE ref_left = $1"#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        // 已完成判定：主状态行存在、未软删且指向完成状态
        let is_done = matches!(&current, Some((ref_right, None)) if *ref_right == status_id);

        if is_done {
            // Unmark done: soft-delete the primary-status relationship
            sqlx::query(
                r#"UPDATE isahl."zc_id_lifecycle_r_primary-status"
                   SET deleted_at = $1
                   WHERE ref_left = $2 AND ref_right = $3 AND deleted_at IS NULL"#,
            )
            .bind(now)
            .bind(id)
            .bind(status_id)
            .execute(&self.pool)
            .await?;
        } else {
            // Mark done: ref_left 锚定 upsert（活行覆盖/软删行 restore/无行 INSERT）
            let updated = sqlx::query(
                r#"UPDATE isahl."zc_id_lifecycle_r_primary-status"
                   SET deleted_at = NULL, ref_right = $1, updated_at = $2
                   WHERE ref_left = $3
                   RETURNING id"#,
            )
            .bind(status_id)
            .bind(now)
            .bind(id)
            .execute(&self.pool)
            .await?;

            if updated.rows_affected() == 0 {
                let insert = sqlx::query(
                    r#"INSERT INTO isahl."zc_id_lifecycle_r_primary-status"
                       (ref_left, ref_right, created_at, updated_at)
                       VALUES ($1, $2, $3, $3)"#,
                )
                .bind(id)
                .bind(status_id)
                .bind(now)
                .execute(&self.pool)
                .await;

                match insert {
                    Ok(_) => {}
                    Err(sqlx::Error::Database(db_err)) if db_err.is_unique_violation() => {
                        // 并发竞态兜底：另一事务已先落 ref_left 行 → 覆盖 UPDATE
                        sqlx::query(
                            r#"UPDATE isahl."zc_id_lifecycle_r_primary-status"
                               SET deleted_at = NULL, ref_right = $1, updated_at = $2
                               WHERE ref_left = $3 AND deleted_at IS NULL"#,
                        )
                        .bind(status_id)
                        .bind(now)
                        .bind(id)
                        .execute(&self.pool)
                        .await?;
                    }
                    Err(e) => return Err(e),
                }
            }
        }

        // 返回 event 实体（调用方无需区分 plan/event 路径）
        let sql = format!(
            "SELECT {} FROM isahl.zc_id_event WHERE id = $1 AND deleted_at IS NULL",
            Event::SELECT_FIELDS
        );
        sqlx::query_as(AssertSqlSafe(sql.as_str()))
            .bind(id)
            .fetch_optional(&self.pool)
            .await
    }

    // ----- Event CRUD -----

    pub async fn create_event(&self, req: &CreateEventRequest) -> Result<Event, sqlx::Error> {
        let now = Utc::now();
        // 叶表坐标（§6.12）：值经 ontology_binding 解析 code→ZUID（禁硬编码 ZUID）
        let (dk_scene, dk_factor, dk_function) =
            ontology_binding::resolve(&self.pool, ("JE", "FBB", "↓_EE")).await?;
        let sql = format!(
            r#"INSERT INTO "isahl.zc_id_even-alert" 
                (notice, fk_place, fk_subject, qk_date, created_at, updated_at, dk_scene, dk_factor, dk_function)
            VALUES ($1, $2, $3, $4, $5, $5, $6, $7, $8)
            RETURNING {}"#,
            Event::SELECT_FIELDS
        );
        sqlx::query_as(AssertSqlSafe(sql.as_str()))
            .bind(&req.notice)
            .bind(req.fk_place)
            .bind(req.fk_subject)
            .bind(req.qk_date)
            .bind(now)
            .bind(dk_scene)
            .bind(dk_factor)
            .bind(dk_function)
            .fetch_one(&self.pool)
            .await
    }

    pub async fn create_event_for_plan(
        &self,
        plan_id: i64,
        req: &CreateEventRequest,
    ) -> Result<Event, sqlx::Error> {
        let event = self.create_event(req).await?;
        let now = Utc::now();

        // 建立 Plan ↔ Event 关联（忽略失败，允许重复关联）
        let _ = sqlx::query(
            r#"INSERT INTO isahl.zc_id_plan_rr_event 
                (ref_left, ref_right, created_at, updated_at)
            VALUES ($1, $2, $3, $3)"#,
        )
        .bind(plan_id)
        .bind(event.id)
        .bind(now)
        .execute(&self.pool)
        .await;

        Ok(event)
    }

    pub async fn update_event(
        &self,
        id: i64,
        req: &UpdateEventRequest,
    ) -> Result<Option<Event>, sqlx::Error> {
        let now = Utc::now();
        let sql = format!(
            r#"UPDATE isahl.zc_id_event SET
                notice = COALESCE($1, notice),
                fk_place = COALESCE($2, fk_place),
                fk_subject = COALESCE($3, fk_subject),
                qk_date = COALESCE($4, qk_date),
                updated_at = $5
            WHERE id = $6 AND deleted_at IS NULL
            RETURNING {}"#,
            Event::SELECT_FIELDS
        );
        sqlx::query_as(AssertSqlSafe(sql.as_str()))
            .bind(&req.notice)
            .bind(req.fk_place)
            .bind(req.fk_subject)
            .bind(req.qk_date)
            .bind(now)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
    }

    pub async fn delete_event(&self, id: i64) -> Result<u64, sqlx::Error> {
        let now = Utc::now();
        let result = sqlx::query(
            "UPDATE isahl.zc_id_event SET deleted_at = $1 WHERE id = $2 AND deleted_at IS NULL",
        )
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    // ----- Overview -----

    /// 获取指定日期范围内的计划数
    pub async fn get_plan_count_by_date_range(
        &self,
        date_start: &DateTime<Utc>,
        date_end: &DateTime<Utc>,
        user_id: Option<i64>,
        visible_ids: Option<&[i64]>,
    ) -> Result<i64, sqlx::Error> {
        let rls_clause = if visible_ids.is_some() {
            "AND p.id = ANY($4)"
        } else {
            ""
        };
        let sql = format!(
            r#"SELECT COUNT(*) FROM isahl.zc_id_plan p
            JOIN isahl."zc_id_segm-date" ds ON ds.id = p."qk_date-segm"
            WHERE p.deleted_at IS NULL
              AND ds.date_st >= $1 AND ds.date_st <= $2
              AND ($3::bigint IS NULL OR p.created_by_id = $3)
              {rls_clause}"#,
            rls_clause = rls_clause
        );
        let mut q = sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(sql.as_str()))
            .bind(date_start)
            .bind(date_end)
            .bind(user_id);
        if let Some(ids) = visible_ids {
            q = q.bind(ids.to_vec());
        }
        q.fetch_one(&self.pool).await
    }

    /// 通过日期段外键获取 segm-date 详情
    pub async fn find_date_segm(&self, id: i64) -> Result<Option<DateSegm>, sqlx::Error> {
        let sql = format!(
            r#"SELECT {} FROM isahl."zc_id_segm-date" WHERE id = $1"#,
            DateSegm::SELECT_FIELDS
        );
        sqlx::query_as(AssertSqlSafe(sql.as_str()))
            .bind(id)
            .fetch_optional(&self.pool)
            .await
    }

    /// 获取当前用户的未完成待办数量（徽标计数）。
    ///
    /// 行级谓词与 `list_todo_items` 共用 `TODO_ROW_PREDICATE`，创建者过滤亦与其对齐
    /// （仅当前用户创建），再追加 `TODO_PENDING_PREDICATE`——保证
    /// 「徽标数字 = 清单中 `done == false` 的行数」。未认证（`None`）返回 0。
    pub async fn get_pending_todo_count(
        &self,
        user_id: Option<i64>,
        visible_ids: Option<&[i64]>,
    ) -> Result<i64, sqlx::Error> {
        let Some(user_id) = user_id else {
            return Ok(0);
        };
        let rls_clause = if visible_ids.is_some() {
            "AND e.id = ANY($2)"
        } else {
            ""
        };
        let sql = format!(
            r#"SELECT COUNT(*) FROM isahl."zc_id_even-alert" e
            LEFT JOIN isahl."zc_id_lifecycle_r_primary-status" ps ON ps.ref_left = e.id AND ps.deleted_at IS NULL
            LEFT JOIN isahl."zc_id_stus-event" st ON st.id = ps.ref_right
            WHERE e.created_by_id = $1
              {rows}
              {pending}
              {rls_clause}"#,
            rows = TODO_ROW_PREDICATE,
            pending = TODO_PENDING_PREDICATE,
            rls_clause = rls_clause
        );
        let mut q = sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(sql.as_str())).bind(user_id);
        if let Some(ids) = visible_ids {
            q = q.bind(ids.to_vec());
        }
        q.fetch_one(&self.pool).await
    }

    // Event-based queries

    /// 列出待办项（基于 zc_id_event + 主体 + 状态）。
    ///
    /// 行级谓词 = `TODO_ROW_PREDICATE`（与徽标计数同源）；**不过滤状态**——
    /// 已完成行照常返回（`done` 由主状态推导），供面板「显示已完成」展开与
    /// `completedCount/totalCount` 统计使用，默认隐藏由前端 `items.filter(!done)` 负责。
    pub(crate) async fn list_todo_items(
        &self,
        user_id: i64,
        limit: i64,
        offset: i64,
        visible_ids: Option<&[i64]>,
    ) -> Result<Vec<RawTodoItem>, sqlx::Error> {
        let rls_clause = if visible_ids.is_some() {
            "AND e.id = ANY($4)"
        } else {
            ""
        };
        let sql = format!(
            r#"SELECT 
                e.id as event_id,
                e.notice as event_notice,
                s.notice as subject_name,
                st.notice as status_notice,
                st.flag::text as status_flag
            FROM isahl."zc_id_even-alert" e
            LEFT JOIN isahl.zc_id_subjects s ON s.id = e.fk_subject
            LEFT JOIN isahl."zc_id_lifecycle_r_primary-status" ps ON ps.ref_left = e.id AND ps.deleted_at IS NULL
            LEFT JOIN isahl."zc_id_stus-event" st ON st.id = ps.ref_right
            WHERE e.created_by_id = $1
              {rows}
              {rls_clause}
            ORDER BY e.qk_date ASC NULLS LAST, e.created_at DESC
            LIMIT $2 OFFSET $3"#,
            rows = TODO_ROW_PREDICATE,
            rls_clause = rls_clause
        );
        let mut q = sqlx::query_as::<_, RawTodoItem>(sqlx::AssertSqlSafe(sql.as_str()))
            .bind(user_id)
            .bind(limit)
            .bind(offset);
        if let Some(ids) = visible_ids {
            q = q.bind(ids.to_vec());
        }
        q.fetch_all(&self.pool).await
    }

    /// 获取事件的客体列表（zc_id_operation_rr_event → zc_id_object）
    pub(crate) async fn get_event_objects(
        &self,
        event_id: i64,
    ) -> Result<Vec<EventObjectRow>, sqlx::Error> {
        sqlx::query_as(
            r#"SELECT 
                o.id,
                o.notice
            FROM isahl.zc_id_operation_rr_event eo
            JOIN isahl.zc_id_object o ON o.id = eo.ref_left
            WHERE eo.ref_right = $1 AND eo.deleted_at IS NULL"#,
        )
        .bind(event_id)
        .fetch_all(&self.pool)
        .await
    }
}

// ============================================
// Service
// ============================================

#[derive(Clone)]
pub struct ScheduleService {
    repo: ScheduleRepository,
}

impl ScheduleService {
    pub fn new(repo: ScheduleRepository) -> Self {
        Self { repo }
    }

    // ----- Schedule Item Assembly -----

    pub async fn list_items(
        &self,
        query: &ScheduleListQuery,
        visible_ids: Option<&[i64]>,
    ) -> Result<Vec<ScheduleItemResponse>, ScheduleError> {
        let raw_items = self.repo.list_schedule_items(query, visible_ids).await?;
        let mut items = Vec::with_capacity(raw_items.len());

        for raw in raw_items {
            items.push(self.assemble_item(raw).await);
        }

        Ok(items)
    }

    /// 组装单条日程项 DTO（参与人 + 派生列）——`list_items` 与 `find_item` 共用。
    ///
    /// 参与人查询失败沿用列表读的容错语义（`unwrap_or_default`），不因参与者子查询
    /// 异常丢掉整条日程项。
    async fn assemble_item(&self, raw: RawScheduleItem) -> ScheduleItemResponse {
        let participants = self
            .repo
            .get_participants(raw.plan_id)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|row| ParticipantResponse {
                id: row.id,
                name: row.notice.unwrap_or_default(),
                role: row
                    .resp_type
                    .and_then(|v| v.as_str().map(|s| s.to_string())),
            })
            .collect();

        into_item_response(raw, participants)
    }

    /// 按计划 ID 取单条日程项（Gateway `GET /schedule/items/{id}`）。
    ///
    /// 走 [`ScheduleRepository::get_schedule_item`]（`WHERE p.id = $1`）——**不得**
    /// 退回「`list_items(limit = 1)` + Rust 侧匹配」：那会把取数限制在排序首行，
    /// 非首行计划恒判不存在。
    pub async fn find_item(
        &self,
        plan_id: i64,
    ) -> Result<Option<ScheduleItemResponse>, ScheduleError> {
        match self.repo.get_schedule_item(plan_id).await? {
            Some(raw) => Ok(Some(self.assemble_item(raw).await)),
            None => Ok(None),
        }
    }
    pub async fn list_todos(
        &self,
        user_id: i64,
        limit: i64,
        offset: i64,
        visible_ids: Option<&[i64]>,
    ) -> Result<Vec<TodoItemResponse>, ScheduleError> {
        let raw_items = self
            .repo
            .list_todo_items(user_id, limit, offset, visible_ids)
            .await?;
        let mut items = Vec::new();

        for raw in raw_items {
            // 获取客体列表
            let objects: Vec<TodoObjectResponse> = self
                .repo
                .get_event_objects(raw.event_id)
                .await
                .unwrap_or_default()
                .into_iter()
                .map(|row| {
                    let obj_type = infer_object_type(row.notice.as_deref().unwrap_or(""));
                    TodoObjectResponse {
                        id: row.id,
                        name: row.notice.unwrap_or_default(),
                        object_type: obj_type,
                    }
                })
                .collect();

            // 推导完成状态：st.notice = '完成' 且 st.flag = 'end' → done = true
            let done = raw.status_notice.as_deref() == Some("完成")
                && raw.status_flag.as_deref() == Some("end");

            items.push(TodoItemResponse {
                id: raw.event_id,
                title: raw
                    .event_notice
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| objects.first().map(|o| o.name.clone()).unwrap_or_default()),
                subject: raw.subject_name,
                objects,
                due_date: None, // qk_date 是维度键，需要 JOIN zc_id_scal-date 解析
                status: raw.status_notice,
                done,
            });
        }

        Ok(items)
    }

    // ----- Plan Service -----

    pub async fn create_plan(
        &self,
        req: CreatePlanRequest,
    ) -> Result<ScheduleItemResponse, ScheduleError> {
        let plan = self.repo.create_plan(&req).await?;
        let reminder = self.repo.read_reminder(plan.id).await?;
        let participants = Vec::new();
        Ok(into_item_response_from_plan(
            plan,
            None,
            None,
            None,
            None,
            None,
            participants,
            reminder,
        ))
    }

    pub async fn update_plan(
        &self,
        id: i64,
        req: UpdatePlanRequest,
    ) -> Result<Option<ScheduleItemResponse>, ScheduleError> {
        let plan = self.repo.update_plan(id, &req).await?;
        match plan {
            Some(p) => {
                let reminder = self.repo.read_reminder(p.id).await?;
                let participants = Vec::new();
                Ok(Some(into_item_response_from_plan(
                    p,
                    None,
                    None,
                    None,
                    None,
                    None,
                    participants,
                    reminder,
                )))
            }
            None => Ok(None),
        }
    }

    pub async fn delete_plan(&self, id: i64) -> Result<bool, ScheduleError> {
        let rows = self.repo.delete_plan(id).await?;
        Ok(rows > 0)
    }

    pub async fn toggle_plan_done(
        &self,
        id: i64,
    ) -> Result<Option<ScheduleItemResponse>, ScheduleError> {
        let plan = self.repo.toggle_plan_done(id).await?;
        match plan {
            Some(p) => {
                let reminder = self.repo.read_reminder(p.id).await?;
                let participants = Vec::new();
                Ok(Some(into_item_response_from_plan(
                    p,
                    None,
                    None,
                    None,
                    None,
                    None,
                    participants,
                    reminder,
                )))
            }
            None => Ok(None),
        }
    }

    /// 切换事件完成状态（/schedule/todos 的待办 checkbox 传 event id 到 /toggle）
    pub async fn toggle_event_done(&self, id: i64) -> Result<Option<Event>, ScheduleError> {
        self.repo
            .toggle_event_done(id)
            .await
            .map_err(ScheduleError::from)
    }

    // ----- Event Service -----

    pub async fn create_event(&self, req: CreateEventRequest) -> Result<Event, ScheduleError> {
        let event = self.repo.create_event(&req).await?;
        Ok(event)
    }

    pub async fn update_event(
        &self,
        id: i64,
        req: UpdateEventRequest,
    ) -> Result<Option<Event>, ScheduleError> {
        let event = self.repo.update_event(id, &req).await?;
        Ok(event)
    }

    pub async fn delete_event(&self, id: i64) -> Result<bool, ScheduleError> {
        let rows = self.repo.delete_event(id).await?;
        Ok(rows > 0)
    }

    // ----- Overview -----

    pub async fn get_overview(
        &self,
        date_start: DateTime<Utc>,
        date_end: DateTime<Utc>,
        user_id: Option<i64>,
        code: Option<&str>,
        visible_ids: Option<&[i64]>,
    ) -> Result<ScheduleOverviewResponse, ScheduleError> {
        let today_count = self
            .repo
            .get_plan_count_by_date_range(&date_start, &date_end, user_id, visible_ids)
            .await?;

        let upcoming_query = ScheduleListQuery {
            start_date_segm: None,
            end_date_segm: None,
            qk_date_segm: None,
            // 类型筛选（fix-workspace-dock-contracts P1-7）：前端 ?code= 传类型，
            // 未匹配时忽略（fail-open）
            _t_: code.map(|c| c.to_string()),
            done: Some(false),
            limit: 5,
            offset: 0,
        };
        let upcoming = self.list_items(&upcoming_query, visible_ids).await?;

        let pending_todo_count = self
            .repo
            .get_pending_todo_count(user_id, visible_ids)
            .await?;

        Ok(ScheduleOverviewResponse {
            today_event_count: today_count,
            pending_todo_count,
            upcoming_items: upcoming,
        })
    }
}

// ============================================
// Helpers
// ============================================

/// 根据客体 label 精确推断客体类型
/// 固化 Alioth 模型元数据：production = 产品服务，bill = 单据
fn infer_object_type(label: &str) -> String {
    match label {
        // === Bill（单据）===
        "实现-单据" | "单据-清算账单" | "单据-价格清单" | "实现-发票" => {
            "bill".to_string()
        }

        // === Production（产品服务）===
        "实现-产品"
        | "产品-销售"
        | "产品-租赁"
        | "产品-组配齐套"
        | "产品-数据内容"
        | "产品-咨询报告"
        | "产品-制造"
        | "产品-采购"
        | "产品-诉求"
        | "产品-行政事务"
        | "产品-海关结清⟨清关⟩"
        | "产品-海关申报⟨报关⟩"
        | "产品-金融服务"
        | "产品-保险服务"
        | "产品-物流运输" => "production".to_string(),

        _ => {
            // 前缀回退（覆盖未枚举的子类）
            if label.starts_with("产品-") || label.starts_with("zc_id_prod-") {
                "production".to_string()
            } else if label.starts_with("单据-") || label.starts_with("zc_id_bill") {
                "bill".to_string()
            } else {
                "other".to_string()
            }
        }
    }
}

fn into_item_response(
    raw: RawScheduleItem,
    participants: Vec<ParticipantResponse>,
) -> ScheduleItemResponse {
    let done = raw.done;
    let progress = if done {
        Decimal::from(100u8)
    } else {
        Decimal::ZERO
    };
    // 日期/时间跨度（单 JOIN segm-date）
    let date_start = raw
        .segm_date_st
        .map(|d| d.date_naive().format("%Y-%m-%d").to_string());
    let date_end = raw
        .segm_date_ed
        .map(|d| d.date_naive().format("%Y-%m-%d").to_string());
    let time_start = raw.segm_time_st.map(|t| t.format("%H:%M").to_string());
    let time_end = raw.segm_time_ed.map(|t| t.format("%H:%M").to_string());

    // 计算时长描述
    let duration = if raw.plan_cron.is_some() {
        "周期性".to_string()
    } else if let (Some(st), Some(ed)) = (&raw.segm_time_st, &raw.segm_time_ed) {
        let minutes = ed.signed_duration_since(*st).num_minutes();
        if minutes >= 60 {
            format!("{}h", minutes / 60)
        } else {
            format!("{}min", minutes)
        }
    } else {
        "1h".to_string()
    };

    ScheduleItemResponse {
        id: raw.plan_id,
        title: raw.plan_notice.unwrap_or_default(),
        item_type: raw.plan_type.unwrap_or_else(|| "other".to_string()),
        span: DateTimeSpanResponse {
            date_start,
            date_end,
            time_start,
            time_end,
        },
        duration,
        location: raw
            .place_name
            .or(raw.event_fk_place.map(|id| format!("场所#{}", id))),
        subject: raw
            .subject_name
            .or(raw.event_fk_subject.map(|id| format!("主体#{}", id))),
        participants,
        done,
        progress_pct: progress,
        reminder: match (raw.segm_date_st, raw.remind_at) {
            (Some(start_at), Some(remind_at)) => reminder_from_times(start_at, remind_at),
            _ => None,
        },
        linked_approval: raw.approval_status.map(|status| LinkedApprovalResponse {
            id: raw.event_id.unwrap_or(0),
            title: raw.approval_title.unwrap_or_default(),
            status,
            applicant: None,
        }),
        cron: raw.plan_cron,
    }
}

fn into_item_response_from_plan(
    plan: Plan,
    date_segm: Option<DateSegm>,
    place_name: Option<String>,
    subject_name: Option<String>,
    approval_status: Option<String>,
    approval_title: Option<String>,
    participants: Vec<ParticipantResponse>,
    reminder: Option<ReminderResponse>,
) -> ScheduleItemResponse {
    let progress = Decimal::ZERO;
    let done = false;
    let date_start = date_segm
        .as_ref()
        .and_then(|ds| ds.date_st.map(|d| d.format("%Y-%m-%d").to_string()));
    let date_end = date_segm
        .as_ref()
        .and_then(|ds| ds.date_ed.map(|d| d.format("%Y-%m-%d").to_string()));
    let time_start = date_segm
        .as_ref()
        .and_then(|ds| ds.time_st.map(|t| t.format("%H:%M").to_string()));
    let time_end = date_segm
        .as_ref()
        .and_then(|ds| ds.time_ed.map(|t| t.format("%H:%M").to_string()));

    let duration = if plan.cron.is_some() {
        "周期性".to_string()
    } else {
        let st = date_segm.as_ref().and_then(|ds| ds.time_st);
        let ed = date_segm.as_ref().and_then(|ds| ds.time_ed);
        if let (Some(st_val), Some(ed_val)) = (&st, &ed) {
            let minutes = ed_val.signed_duration_since(*st_val).num_minutes();
            if minutes >= 60 {
                format!("{}h", minutes / 60)
            } else {
                format!("{}min", minutes)
            }
        } else {
            "1h".to_string()
        }
    };

    ScheduleItemResponse {
        id: plan.id,
        title: plan.notice.unwrap_or_default(),
        item_type: plan._t_.unwrap_or_else(|| "other".to_string()),
        span: DateTimeSpanResponse {
            date_start,
            date_end,
            time_start,
            time_end,
        },
        duration,
        location: place_name,
        subject: subject_name,
        participants,
        done,
        progress_pct: progress,
        reminder,
        linked_approval: approval_status.map(|status| LinkedApprovalResponse {
            id: plan.id,
            title: approval_title.unwrap_or_default(),
            status,
            applicant: None,
        }),
        cron: plan.cron.map(|v| v.to_string()),
    }
}
