//! ledger-entry 实体模型 — crud 模式（AliothDbEntity + HasReferenceJoins）。
//!
//! 自动生成（gen-service-backend.ts）；语义重命名与 _refs joins 按需后续补充。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crud::{AliothDbEntity, HasReferenceJoins, Identifiable, ReferenceJoin};

// ── LedgerEntry → isahl."zc_id_docu-accounting" ──

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct LedgerEntry {
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
    /// 关联引用嵌入数据（_refs 名称解析；joins 待补）
    #[sqlx(default)]
    #[serde(rename = "_refs")]
    pub _refs: Option<serde_json::Value>,
}

impl Identifiable for LedgerEntry {
    fn id(&self) -> i64 {
        self.id
    }
}

impl AliothDbEntity for LedgerEntry {
    fn table_name() -> &'static str {
        r#"isahl."zc_id_docu-accounting""#
    }

    const SELECT_FIELDS: &'static str = r#"created_at, updated_at, id, created_by_id, updated_by_id, notice, t_color_, deleted_at, deleted_by_id, code, o_number, comments, ak_benefit_user, ak_permit_user, ak_access_user, projection, _f_, _t_, dk_scene, dk_factor, dk_function, tpl_id, ak_source, tk_version, tk_batch_no, fk_previous, ck_branch"#;
    const ENTITY_NAME: &'static str = "ledger_entry";
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = false;
}

impl HasReferenceJoins for LedgerEntry {
    fn reference_joins() -> Vec<ReferenceJoin> {
        vec![]
    }
}

/// 创建输入（可写集 = 全列 − 系统列）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CreateLedgerEntryRequest {
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
}

/// 更新输入（全 Optional，同可写集）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateLedgerEntryRequest {
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
}

// ── Account → isahl."zc_id_stor-account" ──

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Account {
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
    pub sk_unit: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_trustee: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_capacity: Option<i64>,
    pub name: Option<String>,
    pub account: Option<String>,
    /// 关联引用嵌入数据（_refs 名称解析；joins 待补）
    #[sqlx(default)]
    #[serde(rename = "_refs")]
    pub _refs: Option<serde_json::Value>,
}

impl Identifiable for Account {
    fn id(&self) -> i64 {
        self.id
    }
}

impl AliothDbEntity for Account {
    fn table_name() -> &'static str {
        r#"isahl."zc_id_stor-account""#
    }

    const SELECT_FIELDS: &'static str = r#"created_at, updated_at, id, created_by_id, updated_by_id, notice, t_color_, deleted_at, deleted_by_id, code, o_number, comments, ak_benefit_user, ak_permit_user, ak_access_user, projection, _f_, _t_, dk_scene, dk_factor, dk_function, tpl_id, ak_source, sk_unit, fk_trustee, qk_capacity, name, account"#;
    const ENTITY_NAME: &'static str = "account";
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = false;
}

impl HasReferenceJoins for Account {
    fn reference_joins() -> Vec<ReferenceJoin> {
        vec![]
    }
}

/// 创建输入（可写集 = 全列 − 系统列）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CreateAccountRequest {
    pub notice: Option<String>,
    pub code: Option<String>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt_seq", default)]
    pub ak_source: Option<Vec<i64>>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub sk_unit: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_trustee: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_capacity: Option<i64>,
    pub name: Option<String>,
    pub account: Option<String>,
}

/// 更新输入（全 Optional，同可写集）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateAccountRequest {
    pub notice: Option<String>,
    pub code: Option<String>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt_seq", default)]
    pub ak_source: Option<Vec<i64>>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub sk_unit: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_trustee: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_capacity: Option<i64>,
    pub name: Option<String>,
    pub account: Option<String>,
}

// ── SubjectAccount → isahl.zc_id_subjects_rr_account ──

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct SubjectAccount {
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
    #[serde(with = "common::serde_zuid::opt", default)]
    pub ref_left: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub ref_right: Option<i64>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_period: Option<i64>,
    /// 关联引用嵌入数据（_refs 名称解析；joins 待补）
    #[sqlx(default)]
    #[serde(rename = "_refs")]
    pub _refs: Option<serde_json::Value>,
}

impl Identifiable for SubjectAccount {
    fn id(&self) -> i64 {
        self.id
    }
}

impl AliothDbEntity for SubjectAccount {
    fn table_name() -> &'static str {
        r#"isahl.zc_id_subjects_rr_account"#
    }

    const SELECT_FIELDS: &'static str = r#"created_at, updated_at, id, created_by_id, updated_by_id, notice, t_color_, deleted_at, deleted_by_id, code, ref_left, ref_right, comments, qk_period"#;
    const ENTITY_NAME: &'static str = "subject_account";
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = false;
}

impl HasReferenceJoins for SubjectAccount {
    fn reference_joins() -> Vec<ReferenceJoin> {
        vec![]
    }
}

/// 创建输入（可写集 = 全列 − 系统列）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CreateSubjectAccountRequest {
    pub notice: Option<String>,
    pub code: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub ref_left: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub ref_right: Option<i64>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_period: Option<i64>,
}

/// 更新输入（全 Optional，同可写集）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateSubjectAccountRequest {
    pub notice: Option<String>,
    pub code: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub ref_left: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub ref_right: Option<i64>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_period: Option<i64>,
}
