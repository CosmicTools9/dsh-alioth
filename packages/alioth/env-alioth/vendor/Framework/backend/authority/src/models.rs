use chrono::{DateTime, Utc};
use crud::entity::{AliothDbEntity, Identifiable};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

// ── Employee ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Employee {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub name: String,
    pub code: Option<String>,
    #[serde(with = "common::serde_zuid::opt")]
    pub fk_user: Option<i64>,
    #[serde(with = "common::serde_zuid::opt")]
    pub sk_currency: Option<i64>,
    #[serde(with = "common::serde_zuid::opt")]
    pub ck_category: Option<i64>,
    #[serde(with = "common::serde_zuid::opt")]
    pub sk_unit: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl Identifiable for Employee {
    fn id(&self) -> i64 {
        self.id
    }
}

impl AliothDbEntity for Employee {
    fn table_name() -> &'static str {
        "isahl.\"zc_id_subj-employee\""
    }
    const SELECT_FIELDS: &'static str =
        "id, notice AS name, code, fk_user, sk_currency, ck_category, sk_unit, created_at, updated_at, deleted_at";
    const ENTITY_NAME: &'static str = "employee";
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = false;
}

// ── SkillTag ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct SkillTag {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub name: String,
    pub code: Option<String>,
    pub v_group: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl Identifiable for SkillTag {
    fn id(&self) -> i64 {
        self.id
    }
}

impl AliothDbEntity for SkillTag {
    fn table_name() -> &'static str {
        "isahl.\"zc_id_tags-skill\""
    }
    const SELECT_FIELDS: &'static str =
        "id, notice AS name, code, v_group, created_at, updated_at, deleted_at";
    const ENTITY_NAME: &'static str = "skill-tag";
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = false;
}

// ── ApprovalRole ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct ApprovalRole {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl Identifiable for ApprovalRole {
    fn id(&self) -> i64 {
        self.id
    }
}

impl AliothDbEntity for ApprovalRole {
    // 审批岗位专用表（models.rs 岗位映射注释的设计目标表）；
    // 曾误用共享字典 zc_id_category——全模型 150+ 行（发票/车型/维度态）
    // 混入审批人下拉。2026-09-02 修正。
    fn table_name() -> &'static str {
        "isahl.\"zc_id_cate-approve_role\""
    }
    const SELECT_FIELDS: &'static str = "id, notice AS name, created_at, updated_at, deleted_at";
    const ENTITY_NAME: &'static str = "approval-role";
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = false;
}

// Approver 链式语义（设计意图，未实现）：
// zc_id_oper-approve（审批操作）← zc_id_operation_rr_review（操作↔审查岗位）→ zc_id_subj-position（岗位）
// role 映射（ck_role → zc_id_cate-approve_role）与 weight 映射
// （lk_vote_weight → zc_id_leve-vote_weight.lv_value）尚未实现：
// zc_id_subj-position 无 lk_vote_weight 列（仅 lk_respo），zc_id_leve-vote_weight 无数据。
// 当前模型仅暴露 zc_id_subj-position 基础字段，响应不含 role/weight——
// 前端按可选字段消费，不得假定存在。

// ── Approver ─────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Approver {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub name: String,
    #[serde(with = "common::serde_zuid::opt")]
    pub ck_category: Option<i64>,
    #[serde(with = "common::serde_zuid::opt")]
    pub fk_user: Option<i64>,
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl Identifiable for Approver {
    fn id(&self) -> i64 {
        self.id
    }
}

impl AliothDbEntity for Approver {
    fn table_name() -> &'static str {
        "isahl.\"zc_id_subj-position\""
    }
    const SELECT_FIELDS: &'static str =
        "id, notice AS name, fk_user, ck_category, comments AS description, created_at, updated_at, deleted_at";
    const ENTITY_NAME: &'static str = "approver";
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = false;
}
// ── Request DTOs ──────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ListEmployeesQuery {
    #[serde(with = "common::serde_zuid::opt")]
    pub page: Option<i64>,
    #[serde(with = "common::serde_zuid::opt")]
    pub page_size: Option<i64>,
    pub search: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateEmployeeRequest {
    pub name: String,
    pub code: Option<String>,
    #[serde(with = "common::serde_zuid::opt")]
    pub role: Option<i64>,
    #[serde(with = "common::serde_zuid::opt")]
    pub team: Option<i64>,
    #[serde(with = "common::serde_zuid::opt")]
    pub fk_user: Option<i64>,
    #[serde(with = "common::serde_zuid::opt")]
    pub sk_currency: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateEmployeeRequest {
    pub name: Option<String>,
    pub code: Option<String>,
    #[serde(with = "common::serde_zuid::opt")]
    pub role: Option<i64>,
    #[serde(with = "common::serde_zuid::opt")]
    pub team: Option<i64>,
    #[serde(with = "common::serde_zuid::opt")]
    pub fk_user: Option<i64>,
    #[serde(with = "common::serde_zuid::opt")]
    pub sk_currency: Option<i64>,
}

// SkillTag DTOs
#[derive(Debug, Deserialize)]
pub struct CreateSkillTagRequest {
    pub name: String,
    pub code: Option<String>,
    pub category: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateSkillTagRequest {
    pub name: Option<String>,
    pub code: Option<String>,
    pub category: Option<String>,
}

// ApprovalRole DTOs
#[derive(Debug, Deserialize)]
pub struct CreateApprovalRoleRequest {
    pub name: String,
    #[serde(with = "common::serde_zuid::opt")]
    pub role: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateApprovalRoleRequest {
    pub name: Option<String>,
    #[serde(with = "common::serde_zuid::opt")]
    pub role: Option<i64>,
}

// Approver DTOs
#[derive(Debug, Deserialize)]
pub struct CreateApproverRequest {
    pub name: String,
    #[serde(with = "common::serde_zuid::opt")]
    pub role: Option<i64>,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateApproverRequest {
    pub name: Option<String>,
    #[serde(with = "common::serde_zuid::opt")]
    pub role: Option<i64>,
    pub description: Option<String>,
}
