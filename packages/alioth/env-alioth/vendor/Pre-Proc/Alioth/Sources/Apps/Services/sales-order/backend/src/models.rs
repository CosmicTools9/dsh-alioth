//! sales-order 实体模型 — crud 模式（AliothDbEntity + HasReferenceJoins）。
//!
//! 自动生成（gen-service-backend.ts）；语义重命名与 _refs joins 按需后续补充。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crud::{AliothDbEntity, HasReferenceJoins, Identifiable, ReferenceJoin};

// ── SalesOrder → isahl."zc_id_oper-sales_order" ──

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct SalesOrder {
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
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_work_duration: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_operator: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_subject: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_period: Option<i64>,
    #[sqlx(rename = "ck_cate-wh")]
    #[serde(with = "common::serde_zuid::opt", default)]
    pub ck_cate_wh: Option<i64>,
    #[sqlx(rename = "sk_unit-working")]
    #[serde(with = "common::serde_zuid::opt", default)]
    pub sk_unit_working: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_arrived: Option<i64>,
    #[sqlx(rename = "ck_cate-biz")]
    #[serde(with = "common::serde_zuid::opt", default)]
    pub ck_cate_biz: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_approve: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_sla: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub lk_urgent: Option<i64>,
    #[sqlx(rename = "ck_cate-proc_op")]
    #[serde(with = "common::serde_zuid::opt", default)]
    pub ck_cate_proc_op: Option<i64>,
    /// 关联引用嵌入数据（_refs 名称解析；joins 待补）
    #[sqlx(default)]
    #[serde(rename = "_refs")]
    pub _refs: Option<serde_json::Value>,
}

impl Identifiable for SalesOrder {
    fn id(&self) -> i64 {
        self.id
    }
}

impl AliothDbEntity for SalesOrder {
    fn table_name() -> &'static str {
        r#"isahl."zc_id_oper-sales_order""#
    }

    // fix-fk-approve-residual-consumers：fk_approve 物理列已移除——经 rr_event 桥派生
    const SELECT_FIELDS: &'static str = r#"created_at, updated_at, id, created_by_id, updated_by_id, notice, t_color_, deleted_at, deleted_by_id, code, o_number, comments, ak_benefit_user, ak_permit_user, ak_access_user, projection, _f_, _t_, dk_scene, dk_factor, dk_function, tpl_id, ak_source, tk_version, tk_batch_no, fk_previous, ck_branch, qk_work_duration, fk_operator, fk_subject, qk_period, "ck_cate-wh", "sk_unit-working", qk_arrived, "ck_cate-biz", (SELECT oe.ref_right FROM isahl.zc_id_operation_rr_event oe JOIN isahl."zc_id_even-approve" ea ON ea.id = oe.ref_right AND ea.deleted_at IS NULL WHERE oe.ref_left = e.id AND oe.deleted_at IS NULL ORDER BY oe.created_at LIMIT 1) AS fk_approve, qk_sla, lk_urgent, "ck_cate-proc_op""#;
    const ENTITY_NAME: &'static str = "sales_order";
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = false;
}

impl HasReferenceJoins for SalesOrder {
    fn reference_joins() -> Vec<ReferenceJoin> {
        vec![]
    }
}

/// 创建输入（可写集 = 全列 − 系统列）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CreateSalesOrderRequest {
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
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_work_duration: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_operator: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_subject: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_period: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub ck_cate_wh: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub sk_unit_working: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_arrived: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub ck_cate_biz: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_approve: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_sla: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub lk_urgent: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub ck_cate_proc_op: Option<i64>,
}

/// 更新输入（全 Optional，同可写集）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateSalesOrderRequest {
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
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_work_duration: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_operator: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_subject: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_period: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub ck_cate_wh: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub sk_unit_working: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_arrived: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub ck_cate_biz: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_approve: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_sla: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub lk_urgent: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub ck_cate_proc_op: Option<i64>,
}
