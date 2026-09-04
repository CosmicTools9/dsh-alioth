//! inbound-order 实体模型 — crud 模式（AliothDbEntity + HasReferenceJoins）。
//!
//! 自动生成（gen-service-backend.ts）；语义重命名与 _refs joins 按需后续补充。

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crud::{AliothDbEntity, HasReferenceJoins, Identifiable, ReferenceJoin};

// ── InboundOrder → isahl."zc_id_plan-inbound" ──

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct InboundOrder {
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub created_by_id: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub updated_by_id: Option<i64>,
    pub notice: Option<String>,
    pub t_color_: Option<String>,
    pub deleted_at: Option<DateTime<Utc>>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub deleted_by_id: Option<i64>,
    pub code: Option<String>,
    pub o_number: Option<String>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt_seq", default)]
    pub ak_benefit_user: Option<Vec<i64>>,
    #[serde(with = "common::serde_zuid::opt_seq", default)]
    pub ak_permit_user: Option<Vec<i64>>,
    #[serde(with = "common::serde_zuid::opt_seq", default)]
    pub ak_access_user: Option<Vec<i64>>,
    pub projection: Option<String>,
    pub _f_: Option<String>,
    pub _t_: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub dk_scene: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub dk_factor: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub dk_function: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub tpl_id: Option<i64>,
    #[serde(with = "common::serde_zuid::opt_seq", default)]
    pub ak_source: Option<Vec<i64>>,
    pub cron: Option<String>,
    pub exclude: Option<serde_json::Value>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub sort: Option<i64>,
    #[sqlx(rename = "qk_date-segm")]
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_date_segm: Option<i64>,
    #[sqlx(rename = "qk_time-segm")]
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_time_segm: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_progress: Option<i64>,
    pub progress_pct: Option<Decimal>,
    pub schedule_pct: Option<Decimal>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub lk_health: Option<i64>,
    /// 关联引用嵌入数据（_refs 名称解析；joins 待补）
    #[sqlx(default)]
    #[serde(rename = "_refs")]
    pub _refs: Option<serde_json::Value>,
}

impl Identifiable for InboundOrder {
    fn id(&self) -> i64 {
        self.id
    }
}

impl AliothDbEntity for InboundOrder {
    fn table_name() -> &'static str {
        r#"isahl."zc_id_plan-inbound""#
    }

    const SELECT_FIELDS: &'static str = r#"created_at, updated_at, id, created_by_id, updated_by_id, notice, t_color_, deleted_at, deleted_by_id, code, o_number, comments, ak_benefit_user, ak_permit_user, ak_access_user, projection, _f_, _t_, dk_scene, dk_factor, dk_function, tpl_id, ak_source, cron, exclude, sort, "qk_date-segm", "qk_time-segm", qk_progress, progress_pct, schedule_pct, lk_health"#;
    const ENTITY_NAME: &'static str = "inbound_order";
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = false;
}

impl HasReferenceJoins for InboundOrder {
    fn reference_joins() -> Vec<ReferenceJoin> {
        vec![]
    }
}

/// 创建输入（可写集 = 全列 − 系统列）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CreateInboundOrderRequest {
    pub notice: Option<String>,
    pub code: Option<String>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt_seq", default)]
    pub ak_source: Option<Vec<i64>>,
    pub cron: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub sort: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_date_segm: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_time_segm: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_progress: Option<i64>,
    pub progress_pct: Option<Decimal>,
    pub schedule_pct: Option<Decimal>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub lk_health: Option<i64>,
}

/// 更新输入（全 Optional，同可写集）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateInboundOrderRequest {
    pub notice: Option<String>,
    pub code: Option<String>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt_seq", default)]
    pub ak_source: Option<Vec<i64>>,
    pub cron: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub sort: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_date_segm: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_time_segm: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_progress: Option<i64>,
    pub progress_pct: Option<Decimal>,
    pub schedule_pct: Option<Decimal>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub lk_health: Option<i64>,
}

// ── InboundBom → isahl."zc_id_bom-inbound" ──

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct InboundBom {
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub created_by_id: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub updated_by_id: Option<i64>,
    pub notice: Option<String>,
    pub t_color_: Option<String>,
    pub deleted_at: Option<DateTime<Utc>>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub deleted_by_id: Option<i64>,
    pub code: Option<String>,
    pub o_number: Option<String>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt_seq", default)]
    pub ak_benefit_user: Option<Vec<i64>>,
    #[serde(with = "common::serde_zuid::opt_seq", default)]
    pub ak_permit_user: Option<Vec<i64>>,
    #[serde(with = "common::serde_zuid::opt_seq", default)]
    pub ak_access_user: Option<Vec<i64>>,
    pub projection: Option<String>,
    pub _f_: Option<String>,
    pub _t_: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub dk_scene: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub dk_factor: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub dk_function: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub tpl_id: Option<i64>,
    #[serde(with = "common::serde_zuid::opt_seq", default)]
    pub ak_source: Option<Vec<i64>>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub tk_version: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub tk_batch_no: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_previous: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub ck_branch: Option<i64>,
    pub b_number: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_editor: Option<i64>,
    pub r#type: Option<String>,
    /// 关联引用嵌入数据（_refs 名称解析；joins 待补）
    #[sqlx(default)]
    #[serde(rename = "_refs")]
    pub _refs: Option<serde_json::Value>,
}

impl Identifiable for InboundBom {
    fn id(&self) -> i64 {
        self.id
    }
}

impl AliothDbEntity for InboundBom {
    fn table_name() -> &'static str {
        r#"isahl."zc_id_bom-inbound""#
    }

    const SELECT_FIELDS: &'static str = r#"created_at, updated_at, id, created_by_id, updated_by_id, notice, t_color_, deleted_at, deleted_by_id, code, o_number, comments, ak_benefit_user, ak_permit_user, ak_access_user, projection, _f_, _t_, dk_scene, dk_factor, dk_function, tpl_id, ak_source, tk_version, tk_batch_no, fk_previous, ck_branch, b_number, fk_editor, "type""#;
    const ENTITY_NAME: &'static str = "inbound_bom";
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = false;
}

impl HasReferenceJoins for InboundBom {
    fn reference_joins() -> Vec<ReferenceJoin> {
        vec![]
    }
}

/// 创建输入（可写集 = 全列 − 系统列）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CreateInboundBomRequest {
    pub notice: Option<String>,
    pub code: Option<String>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt_seq", default)]
    pub ak_source: Option<Vec<i64>>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub tk_version: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub tk_batch_no: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_previous: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub ck_branch: Option<i64>,
    pub b_number: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_editor: Option<i64>,
    pub r#type: Option<String>,
}

/// 更新输入（全 Optional，同可写集）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateInboundBomRequest {
    pub notice: Option<String>,
    pub code: Option<String>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt_seq", default)]
    pub ak_source: Option<Vec<i64>>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub tk_version: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub tk_batch_no: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_previous: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub ck_branch: Option<i64>,
    pub b_number: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_editor: Option<i64>,
    pub r#type: Option<String>,
}
