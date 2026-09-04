//! stock-count 实体模型 — crud 模式（AliothDbEntity + HasReferenceJoins）。
//!
//! 自动生成（gen-service-backend.ts）；语义重命名与 _refs joins 按需后续补充。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crud::{AliothDbEntity, HasReferenceJoins, Identifiable, ReferenceJoin};

// ── StockCount → isahl."zc_id_even-counting" ──

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct StockCount {
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
    pub qk_date: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_place: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_subject: Option<i64>,
    pub timeline: Option<serde_json::Value>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_storage: Option<i64>,
    pub summary: Option<String>,
    /// 关联引用嵌入数据（_refs 名称解析；joins 待补）
    #[sqlx(default)]
    #[serde(rename = "_refs")]
    pub _refs: Option<serde_json::Value>,
}

impl Identifiable for StockCount {
    fn id(&self) -> i64 {
        self.id
    }
}

impl AliothDbEntity for StockCount {
    fn table_name() -> &'static str {
        r#"isahl."zc_id_even-counting""#
    }

    const SELECT_FIELDS: &'static str = r#"created_at, updated_at, id, created_by_id, updated_by_id, notice, t_color_, deleted_at, deleted_by_id, code, o_number, comments, ak_benefit_user, ak_permit_user, ak_access_user, projection, _f_, _t_, dk_scene, dk_factor, dk_function, tpl_id, ak_source, qk_date, fk_place, fk_subject, timeline, fk_storage, summary"#;
    const ENTITY_NAME: &'static str = "stock_count";
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = false;
}

impl HasReferenceJoins for StockCount {
    fn reference_joins() -> Vec<ReferenceJoin> {
        vec![]
    }
}

/// 创建输入（可写集 = 全列 − 系统列）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CreateStockCountRequest {
    pub notice: Option<String>,
    pub code: Option<String>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt_seq", default)]
    pub ak_source: Option<Vec<i64>>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_date: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_place: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_subject: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_storage: Option<i64>,
    pub summary: Option<String>,
}

/// 更新输入（全 Optional，同可写集）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateStockCountRequest {
    pub notice: Option<String>,
    pub code: Option<String>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt_seq", default)]
    pub ak_source: Option<Vec<i64>>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_date: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_place: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_subject: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_storage: Option<i64>,
    pub summary: Option<String>,
}

// ── StockCountDetail → isahl."zc_id_deta-counting" ──

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct StockCountDetail {
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
    pub qk_date: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub ck_category: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_list: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_biller: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_production: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_storage: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_qty: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_w_qty: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_v_qty: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_voucher: Option<i64>,
    /// 关联引用嵌入数据（_refs 名称解析；joins 待补）
    #[sqlx(default)]
    #[serde(rename = "_refs")]
    pub _refs: Option<serde_json::Value>,
}

impl Identifiable for StockCountDetail {
    fn id(&self) -> i64 {
        self.id
    }
}

impl AliothDbEntity for StockCountDetail {
    fn table_name() -> &'static str {
        r#"isahl."zc_id_deta-counting""#
    }

    const SELECT_FIELDS: &'static str = r#"created_at, updated_at, id, created_by_id, updated_by_id, notice, t_color_, deleted_at, deleted_by_id, code, o_number, comments, ak_benefit_user, ak_permit_user, ak_access_user, projection, _f_, _t_, dk_scene, dk_factor, dk_function, tpl_id, ak_source, qk_date, ck_category, fk_list, fk_biller, fk_production, fk_storage, qk_qty, qk_w_qty, qk_v_qty, fk_voucher"#;
    const ENTITY_NAME: &'static str = "stock_count_detail";
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = false;
}

impl HasReferenceJoins for StockCountDetail {
    fn reference_joins() -> Vec<ReferenceJoin> {
        vec![]
    }
}

/// 创建输入（可写集 = 全列 − 系统列）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CreateStockCountDetailRequest {
    pub notice: Option<String>,
    pub code: Option<String>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt_seq", default)]
    pub ak_source: Option<Vec<i64>>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_date: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub ck_category: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_list: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_biller: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_production: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_storage: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_qty: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_w_qty: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_v_qty: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_voucher: Option<i64>,
}

/// 更新输入（全 Optional，同可写集）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateStockCountDetailRequest {
    pub notice: Option<String>,
    pub code: Option<String>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt_seq", default)]
    pub ak_source: Option<Vec<i64>>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_date: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub ck_category: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_list: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_biller: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_production: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_storage: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_qty: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_w_qty: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_v_qty: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_voucher: Option<i64>,
}

// ── StockCountStatus → isahl."zc_id_stus-counting" ──

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct StockCountStatus {
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
    pub enable: Option<bool>,
    pub flag: Option<String>,
    /// 关联引用嵌入数据（_refs 名称解析；joins 待补）
    #[sqlx(default)]
    #[serde(rename = "_refs")]
    pub _refs: Option<serde_json::Value>,
}

impl Identifiable for StockCountStatus {
    fn id(&self) -> i64 {
        self.id
    }
}

impl AliothDbEntity for StockCountStatus {
    fn table_name() -> &'static str {
        r#"isahl."zc_id_stus-counting""#
    }

    const SELECT_FIELDS: &'static str = r#"created_at, updated_at, id, created_by_id, updated_by_id, notice, t_color_, deleted_at, deleted_by_id, code, o_number, comments, ak_benefit_user, ak_permit_user, ak_access_user, enable, flag"#;
    const ENTITY_NAME: &'static str = "stock_count_status";
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = false;
}

impl HasReferenceJoins for StockCountStatus {
    fn reference_joins() -> Vec<ReferenceJoin> {
        vec![]
    }
}

/// 创建输入（可写集 = 全列 − 系统列）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CreateStockCountStatusRequest {
    pub notice: Option<String>,
    pub code: Option<String>,
    pub comments: Option<String>,
    pub enable: Option<bool>,
}

/// 更新输入（全 Optional，同可写集）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateStockCountStatusRequest {
    pub notice: Option<String>,
    pub code: Option<String>,
    pub comments: Option<String>,
    pub enable: Option<bool>,
}
