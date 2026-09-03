//! Schedule Models
//!
//! 日程管理数据模型，对齐 zc_id_plan + zc_id_event 双表结构。
//! 日程项由 Plan（计划侧）为主体，通过 zc_id_plan_rr_event 关联 Event（执行侧）补充信息。
//!
//! 迁移自 Framework/backend/schedule/src/models.rs

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

// ============================================
// Core Entities (映射真实表)
// ============================================

/// 时间片段实体（zc_id_segm-date）
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct DateSegm {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub notice: Option<String>,
    pub date_st: Option<DateTime<Utc>>,
    pub date_ed: Option<DateTime<Utc>>,
    pub time_st: Option<DateTime<Utc>>,
    pub time_ed: Option<DateTime<Utc>>,
}

impl DateSegm {
    pub const SELECT_FIELDS: &'static str = "id, notice, date_st, date_ed, time_st, time_ed";
}

/// 日程计划实体（zc_id_plan）
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Plan {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub notice: Option<String>,
    // 业务分类
    pub _f_: Option<String>,
    pub _t_: Option<String>,
    // 提醒等扩展元数据（comments JSON：{"reminder_offset_min": N}）
    pub comments: Option<String>,
    // 重复模式
    pub cron: Option<serde_json::Value>,
    pub exclude: Option<serde_json::Value>,
    // 排序
    #[serde(with = "common::serde_zuid::opt")]
    pub sort: Option<i64>,
    // 外键
    #[serde(with = "common::serde_zuid::opt")]
    pub qk_date_segm: Option<i64>,
    #[serde(with = "common::serde_zuid::opt")]
    pub qk_time_segm: Option<i64>,
    // 审计字段
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl crud::Identifiable for Plan {
    fn id(&self) -> i64 {
        self.id
    }
}

impl Plan {
    // cron 列在库中为 TEXT（dev/test 均如此），模型为 JSON — 用 to_jsonb 包装保证解码兼容；
    // qk_* 物理列带连字符，需别名对齐 Rust 字段名
    pub const SELECT_FIELDS: &'static str = r#"id, notice, code, _f_, _t_, comments, to_jsonb(cron) AS cron, exclude, sort, "qk_date-segm" AS qk_date_segm, "qk_time-segm" AS qk_time_segm, created_at, updated_at, deleted_at"#;
}

/// 日程事件实体（zc_id_event）
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Event {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub notice: Option<String>,
    pub _f_: Option<String>,
    pub _t_: Option<String>,
    // 日期
    #[serde(with = "common::serde_zuid::opt")]
    pub qk_date: Option<i64>,
    // 外键
    #[serde(with = "common::serde_zuid::opt")]
    pub fk_place: Option<i64>,
    #[serde(with = "common::serde_zuid::opt")]
    pub fk_subject: Option<i64>,
    // 审计字段
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl crud::Identifiable for Event {
    fn id(&self) -> i64 {
        self.id
    }
}

impl Event {
    pub const SELECT_FIELDS: &'static str = r#"id, notice, _f_, _t_, qk_date, fk_place, fk_subject, created_at, updated_at, deleted_at"#;
}

/// 审批事件实体（zc_id_even-approve，继承 zc_id_event）
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct EventApprove {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub notice: Option<String>,
    pub number: Option<String>,
}

/// 计划↔事件关联（zc_id_plan_rr_event）
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct PlanEventRelation {
    #[serde(with = "common::serde_zuid")]
    pub ref_left: i64,
    #[serde(with = "common::serde_zuid")]
    pub ref_right: i64,
}

/// 计划↔参与主体关联（zc_id_plan_rr_participants）
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct PlanParticipantRelation {
    #[serde(with = "common::serde_zuid")]
    pub ref_left: i64,
    #[serde(with = "common::serde_zuid")]
    pub ref_right: i64,
    pub resp_type: Option<serde_json::Value>,
}

/// 场所（zc_id_place）
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Place {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub notice: Option<String>,
}

/// 主体（zc_id_subjects）
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Subject {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub notice: Option<String>,
}

/// 通用客体（zc_id_object 基类，用于 operation_rr_event 的 ref_left）
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct ObjectEntity {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub notice: Option<String>,
}

/// 事件↔客体关联（zc_id_operation_rr_event，ref_left=操作/客体, ref_right=事件）
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct EventObjectRelation {
    #[serde(with = "common::serde_zuid")]
    pub ref_left: i64,
    #[serde(with = "common::serde_zuid")]
    pub ref_right: i64,
    pub r_notice: Option<String>,
}

/// 生命周期主状态关系（zc_id_lifecycle_r_primary-status）
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct EventStatusRelation {
    #[serde(with = "common::serde_zuid")]
    pub ref_left: i64,
    #[serde(with = "common::serde_zuid")]
    pub ref_right: i64,
}

/// 事件状态（zc_id_stus-event，继承 zc_id_status）
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct EventStatus {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub notice: Option<String>,
    pub flag: Option<String>,
}

// ============================================
// Request Types
// ============================================

#[derive(Debug, Clone, Deserialize)]
pub struct CreatePlanRequest {
    pub notice: Option<String>,
    /// 业务类型代码（meeting/sync/client/development/team/review/personal/other）
    /// 写入 zc_id_plan-personal.code 或 zc_id_thre-meeting.code
    pub code: Option<String>,
    #[serde(with = "common::serde_zuid::opt")]
    pub qk_date_segm: Option<i64>,
    #[serde(with = "common::serde_zuid::opt")]
    pub qk_time_segm: Option<i64>,
    pub cron: Option<serde_json::Value>,
    pub exclude: Option<serde_json::Value>,
    #[serde(with = "common::serde_zuid::opt")]
    pub sort: Option<i64>,
    // ── 前端 QuickAdd 友好字段（workspace-dock 契约，serde default 兼容）──
    /// 标题（QuickAdd）——解析优先级低于 notice：notice 存在时 notice 生效
    #[serde(default)]
    pub title: Option<String>,
    /// 开始日期（"YYYY-MM-DD"）
    #[serde(default)]
    pub date_start: Option<String>,
    /// 结束日期（"YYYY-MM-DD"）
    #[serde(default)]
    pub date_end: Option<String>,
    /// 开始时间（"HH:MM"）
    #[serde(default)]
    pub time_start: Option<String>,
    /// 结束时间（"HH:MM"）
    #[serde(default)]
    pub time_end: Option<String>,
    /// 类型（event/todo/meeting…）——解析优先级低于 code
    #[serde(default)]
    pub r#type: Option<String>,
    /// 提醒提前分钟数（0/5/15/30/60/1440；None=不设置）。落 comments JSON {"reminder_offset_min": N}
    #[serde(default)]
    pub reminder_offset_min: Option<i32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdatePlanRequest {
    pub notice: Option<String>,
    pub code: Option<String>,
    #[serde(with = "common::serde_zuid::opt")]
    pub qk_date_segm: Option<i64>,
    #[serde(with = "common::serde_zuid::opt")]
    pub qk_time_segm: Option<i64>,
    pub cron: Option<serde_json::Value>,
    pub exclude: Option<serde_json::Value>,
    #[serde(with = "common::serde_zuid::opt")]
    pub sort: Option<i64>,
    /// 提醒提前分钟数（0/5/15/30/60/1440；None=不修改）。落 comments JSON
    #[serde(default)]
    pub reminder_offset_min: Option<i32>,
}

/// 创建事件请求（与计划关联）
#[derive(Debug, Clone, Deserialize)]
pub struct CreateEventRequest {
    pub notice: Option<String>,
    #[serde(with = "common::serde_zuid::opt")]
    pub fk_place: Option<i64>,
    #[serde(with = "common::serde_zuid::opt")]
    pub fk_subject: Option<i64>,
    #[serde(with = "common::serde_zuid::opt")]
    pub qk_date: Option<i64>,
}

/// 更新事件请求
#[derive(Debug, Clone, Deserialize)]
pub struct UpdateEventRequest {
    pub notice: Option<String>,
    #[serde(with = "common::serde_zuid::opt")]
    pub fk_place: Option<i64>,
    #[serde(with = "common::serde_zuid::opt")]
    pub fk_subject: Option<i64>,
    #[serde(with = "common::serde_zuid::opt")]
    pub qk_date: Option<i64>,
}

// ============================================
// Response Types（前端契约）
// ============================================

/// 参与主体响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParticipantResponse {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub name: String,
    pub role: Option<String>,
}

/// 关联审批响应（审批联动）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkedApprovalResponse {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub title: String,
    pub status: String,
    pub applicant: Option<String>,
}

/// 日期/时间跨度响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DateTimeSpanResponse {
    /// 日期开始 YYYY-MM-DD
    pub date_start: Option<String>,
    /// 日期结束 YYYY-MM-DD
    pub date_end: Option<String>,
    /// 时间开始 HH:MM
    pub time_start: Option<String>,
    /// 时间结束 HH:MM
    pub time_end: Option<String>,
}

/// 组装后的日程项响应（Plan + 关联 Event 信息 + segm-date 跨度）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleItemResponse {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub title: String,
    #[serde(rename = "type")]
    pub item_type: String,
    /// 日期/时间跨度（来自 zc_id_segm-date）
    pub span: DateTimeSpanResponse,
    /// 时长描述（计算得出）
    pub duration: String,
    pub location: Option<String>,
    pub subject: Option<String>,
    pub participants: Vec<ParticipantResponse>,
    pub done: bool,
    pub progress_pct: Decimal,
    pub reminder: Option<ReminderResponse>,
    pub linked_approval: Option<LinkedApprovalResponse>,
    pub cron: Option<String>,
}

/// 提醒设置响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReminderResponse {
    pub offset: i32,
    pub channel: String,
}

/// 客体响应（事件关联的客体）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoObjectResponse {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub name: String,
    /// 客体类型（production|bill|other）
    pub object_type: String,
}

/// 待办事项响应（贴合 zc_id_event 模型）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoItemResponse {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    /// 标题（来自 notice）
    pub title: String,
    /// 执行主体名称
    pub subject: Option<String>,
    /// 客体列表（真正需要做的事）
    pub objects: Vec<TodoObjectResponse>,
    /// 截止时间（qk_date 解析）
    pub due_date: Option<String>,
    /// 状态名称
    pub status: Option<String>,
    /// 是否已完成（由 status.flag 或 status.code 推导）
    pub done: bool,
}

/// 日程概览响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleOverviewResponse {
    #[serde(with = "common::serde_zuid")]
    pub today_event_count: i64,
    #[serde(with = "common::serde_zuid")]
    pub pending_todo_count: i64,
    pub upcoming_items: Vec<ScheduleItemResponse>,
}

// ============================================
// Query Types
// ============================================

/// 日程列表查询参数
#[derive(Debug, Deserialize)]
pub struct ScheduleListQuery {
    /// 起始日期段 ID（含）
    #[serde(with = "common::serde_zuid::opt")]
    pub start_date_segm: Option<i64>,
    /// 截止日期段 ID（含）
    #[serde(with = "common::serde_zuid::opt")]
    pub end_date_segm: Option<i64>,
    /// 精确日期段 ID
    #[serde(with = "common::serde_zuid::opt")]
    pub qk_date_segm: Option<i64>,
    /// 业务类型过滤（新: code 字段; 旧: _t_ 字段，alias 兼容）
    #[serde(alias = "code")]
    pub _t_: Option<String>,
    /// 完成状态（暂不可用于 Plan 表）
    pub done: Option<bool>,
    /// 每页数量
    #[serde(default = "default_limit")]
    #[serde(with = "common::serde_zuid")]
    pub limit: i64,
    /// 偏移量
    #[serde(default = "default_offset")]
    #[serde(with = "common::serde_zuid")]
    pub offset: i64,
}

/// 待办列表查询参数
#[derive(Debug, Deserialize)]
pub struct TodoListQuery {
    #[serde(with = "common::serde_zuid::opt")]
    pub limit: Option<i64>,
    #[serde(with = "common::serde_zuid::opt")]
    pub offset: Option<i64>,
}

fn default_limit() -> i64 {
    50
}
fn default_offset() -> i64 {
    0
}
