//! 主体/组织/身份共享模型（identity-org）
//!
//! 提取自 WZ isahl-db models（extract-identity-org-core）。

use chrono::{DateTime, Utc};
use common::AliothError as ApiError;
use crud::entity::{AliothDbEntity, Identifiable};
use crud::reference::{Card, HasReferenceJoins, JoinKind, ReferenceJoin};
use crud::SubtableRouter;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::FromRow;

/// 证照类型字典（zc_id_cate-identity）
#[derive(Debug, serde::Deserialize, Serialize)]
pub struct IdentityCategory {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub code: String,
    pub name: String,
}

/// 主体证照查询行（zc_id_identity + entity_rr_identity + segm-date）
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubjectIdentityRow {
    /// 关联行 id（zc_id_entity_rr_identity.id）
    #[serde(with = "common::serde_zuid")]
    pub rel_id: i64,
    /// 证照实例 id（zc_id_identity.id）
    #[serde(with = "common::serde_zuid")]
    pub identity_id: i64,
    /// 证照号
    pub cert_no: String,
    /// 证照名称
    pub name: String,
    pub category_code: Option<String>,
    pub category_name: Option<String>,
    pub valid_from: Option<DateTime<Utc>>,
    pub valid_to: Option<DateTime<Utc>>,
    /// 距到期天数（无有效期止日为 null；已过期为负数）
    pub days_to_expire: Option<i64>,
    pub expired: bool,
}

/// 主体类型 → 真叶表映射（isahl 全局，含业务别名）
/// 未知/中间层输入 MUST 返回 None（fail-fast，2026-08-29 裁决）——
/// 旧实现静默回退 zc_id_subj-group：传 zc_id_orga-legal 等中间层名会被错分类为
/// 「组」且无任何报错，属违规源头级缺陷，已废除。
pub fn subject_leaf_table(kind: &str) -> Option<&'static str> {
    Some(match kind {
        // 叶表名直通（白名单字面量；create 时还会再校验叶表 ∈ subjects 继承链）
        "zc_id_orga-non-banking-legal" => "\"isahl\".\"zc_id_orga-non-banking-legal\"",
        "zc_id_bank-commercial" => "\"isahl\".\"zc_id_bank-commercial\"",
        "zc_id_bank-central" => "\"isahl\".\"zc_id_bank-central\"",
        "zc_id_empl-natural" => "\"isahl\".\"zc_id_empl-natural\"",
        "zc_id_empl-agent" => "\"isahl\".\"zc_id_empl-agent\"",
        "zc_id_subj-group" => "\"isahl\".\"zc_id_subj-group\"",
        "zc_id_orga-department" => "\"isahl\".\"zc_id_orga-department\"",
        "zc_id_subj-country" => "\"isahl\".\"zc_id_subj-country\"",
        "zc_id_subj-supranational" => "\"isahl\".\"zc_id_subj-supranational\"",
        "zc_id_subj-hierarchy" => "\"isahl\".\"zc_id_subj-hierarchy\"",
        "zc_id_subj-position" => "\"isahl\".\"zc_id_subj-position\"",
        "zc_id_subj-sovereign" => "\"isahl\".\"zc_id_subj-sovereign\"",
        "zc_id_subj-ministry" => "\"isahl\".\"zc_id_subj-ministry\"",
        "zc_id_subj-bank" => "\"isahl\".\"zc_id_subj-bank\"",
        // 业务中文/英文别名（映射到真叶表）
        "natural" | "自然人" => "\"isahl\".\"zc_id_empl-natural\"",
        // 法人（中间表）→ 企业法人叶表
        "legal" | "法人" | "non-banking" | "非银行法人" => {
            "\"isahl\".\"zc_id_orga-non-banking-legal\""
        }
        "commercial-bank" | "商业银行" => "\"isahl\".\"zc_id_bank-commercial\"",
        "central-bank" | "中央银行" => "\"isahl\".\"zc_id_bank-central\"",
        "agent" | "智能体" => "\"isahl\".\"zc_id_empl-agent\"",
        "group" | "组" => "\"isahl\".\"zc_id_subj-group\"",
        "department" | "部门" => "\"isahl\".\"zc_id_orga-department\"",
        // 雇员（中间表）→ 自然人叶表（雇员族无独立叶表）
        "employee" | "雇员" => "\"isahl\".\"zc_id_empl-natural\"",
        "country" | "国家" => "\"isahl\".\"zc_id_subj-country\"",
        "supranational" | "超国家" => "\"isahl\".\"zc_id_subj-supranational\"",
        _ => return None,
    })
}

// 身份实体模型（从 WZ isahl-db 提取）：软删除 + SELECT_FIELDS 匹配 DB 列。

// ═══════════════════════════════════════════════
// Identity (已有)
// ═══════════════════════════════════════════════

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Identity {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub name: String,
    pub code: Option<String>,
    pub notice: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl Identifiable for Identity {
    fn id(&self) -> i64 {
        self.id
    }
}
impl AliothDbEntity for Identity {
    fn table_name() -> &'static str {
        "\"isahl\".\"zc_id_subjects\""
    }
    const SELECT_FIELDS: &'static str =
        "id, COALESCE(notice, '') AS name, code, notice, created_at, updated_at, deleted_at";
    const ENTITY_NAME: &'static str = "identity";
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = false;
}

// ═══════════════════════════════════════════════
// NaturalPerson — zc_id_empl-natural（司机/操作员主数据）
// ═══════════════════════════════════════════════

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct NaturalPerson {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub code: Option<String>,
    pub notice: Option<String>,
    pub o_number: Option<String>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_user: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub ck_category: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub sk_unit: Option<i64>,
    #[sqlx(default)]
    #[serde(rename = "_refs")]
    pub _refs: Option<Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl Identifiable for NaturalPerson {
    fn id(&self) -> i64 {
        self.id
    }
}
impl AliothDbEntity for NaturalPerson {
    fn table_name() -> &'static str {
        "\"isahl\".\"zc_id_empl-natural\""
    }
    const SELECT_FIELDS: &'static str =
        "id, code, notice, o_number, comments, fk_user, ck_category, sk_unit, created_at, updated_at, deleted_at";
    const ENTITY_NAME: &'static str = "natural-person";
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = false;
}

impl HasReferenceJoins for NaturalPerson {
    fn reference_joins() -> Vec<ReferenceJoin> {
        vec![
            ReferenceJoin {
                name: "fk_user",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "fk_user",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_subjects""#,
                display_fields: &["notice", "code"],
            },
            ReferenceJoin {
                name: "ck_category",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "ck_category",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_category""#,
                display_fields: &["notice"],
            },
            ReferenceJoin {
                name: "sk_unit",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "sk_unit",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_unit""#,
                display_fields: &["notice"],
            },
        ]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateNaturalPersonRequest {
    pub code: Option<String>,
    pub notice: Option<String>,
    pub o_number: Option<String>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_user: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub ck_category: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub sk_unit: Option<i64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateNaturalPersonRequest {
    pub code: Option<String>,
    pub notice: Option<String>,
    pub o_number: Option<String>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_user: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub ck_category: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub sk_unit: Option<i64>,
}

impl HasReferenceJoins for Identity {
    fn reference_joins() -> Vec<ReferenceJoin> {
        vec![]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateIdentityRequest {
    pub name: String,
    /// subject_type → zc_id_subjects 叶表路由（Alioth 契约缺省 → "group"）
    #[serde(default = "default_subject_type")]
    pub subject_type: String,
    pub code: Option<String>,
    pub notice: Option<String>,
}

fn default_subject_type() -> String {
    "group".to_string()
}

/// subject_type → zc_id_subjects 叶表路由（仅接受真叶表，叶表名直通 + 业务别名映射）。
/// 中间表（法人/组织/雇员）有子表不提供创建路径；未知输入 400（fail-fast）。
impl SubtableRouter for CreateIdentityRequest {
    fn resolve_subtable(&self, _discriminator: Option<&str>) -> Result<&'static str, ApiError> {
        subject_leaf_table(&self.subject_type).ok_or_else(|| {
            ApiError::BadRequest(format!(
                "未知主体类型: {}（法人须明确叶表：非银行法人/商业银行）",
                self.subject_type
            ))
        })
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateIdentityRequest {
    pub name: Option<String>,
    pub code: Option<String>,
    pub notice: Option<String>,
    pub comments: Option<String>,
    /// MDM 主数据编码（wz_fssc.subject_mdm 侧表；Some(空串)=清除，None=不动）
    #[serde(default, rename = "mdmCode", alias = "mdm_code")]
    pub mdm_code: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Environment {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl Identifiable for Environment {
    fn id(&self) -> i64 {
        self.id
    }
}
impl AliothDbEntity for Environment {
    fn table_name() -> &'static str {
        "\"isahl\".\"zc_id_prot-env_config\""
    }
    const SELECT_FIELDS: &'static str = "id, notice AS name, created_at, updated_at, deleted_at";
    const ENTITY_NAME: &'static str = "environment";
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = false;
}

impl HasReferenceJoins for Environment {
    fn reference_joins() -> Vec<ReferenceJoin> {
        vec![]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateEnvironmentRequest {
    pub name: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateEnvironmentRequest {
    pub name: Option<String>,
}

// ═══════════════════════════════════════════════
// License — isahl.zc_id_prod-license-purchase
// ═══════════════════════════════════════════════

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct License {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub name: String,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_qty: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_period: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl Identifiable for License {
    fn id(&self) -> i64 {
        self.id
    }
}
impl AliothDbEntity for License {
    fn table_name() -> &'static str {
        "\"isahl\".\"zc_id_prod-license-purchase\""
    }
    const SELECT_FIELDS: &'static str =
        "id, notice AS name, qk_capacity AS qk_qty, qk_period, created_at, updated_at, deleted_at";
    const ENTITY_NAME: &'static str = "license";
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = false;
}

impl HasReferenceJoins for License {
    fn reference_joins() -> Vec<ReferenceJoin> {
        vec![ReferenceJoin {
            name: "qk_qty",
            card: Card::ToOne,
            kind: JoinKind::Forward {
                local_fk: "qk_capacity",
                target_key: "id",
            },
            target_table: r#"isahl."zc_id_scal-common""#,
            display_fields: &["notice", "mark"],
        }]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateLicenseRequest {
    pub name: String,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_qty: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_period: Option<i64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateLicenseRequest {
    pub name: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_qty: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_period: Option<i64>,
}
// ═══════════════════════════════════════════════
// P0: 运输执行 — zc_id_orde-land (Consignment + Waybill 共享)
// ═══════════════════════════════════════════════

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Consignment {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub code: Option<String>,
    pub notice: Option<String>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_subject: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_object: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_contract: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_date: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub sk_currency: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub ck_category: Option<i64>,
    #[sqlx(default)]
    pub _refs: Option<Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl Identifiable for Consignment {
    fn id(&self) -> i64 {
        self.id
    }
}
impl AliothDbEntity for Consignment {
    fn table_name() -> &'static str {
        "\"isahl\".\"zc_id_orde-land\""
    }
    const SELECT_FIELDS: &'static str = "id, code, notice, comments, fk_subject, fk_object, fk_contract, qk_date, sk_currency, ck_category, created_at, updated_at, deleted_at";
    const ENTITY_NAME: &'static str = "consignment";
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = false;
}
impl HasReferenceJoins for Consignment {
    fn reference_joins() -> Vec<ReferenceJoin> {
        vec![
            ReferenceJoin {
                name: "fk_subject",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "fk_subject",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_subjects""#,
                display_fields: &["notice", "code"],
            },
            ReferenceJoin {
                name: "fk_object",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "fk_object",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_subjects""#,
                display_fields: &["notice", "code"],
            },
            ReferenceJoin {
                name: "fk_contract",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "fk_contract",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_contract""#,
                display_fields: &["notice", "code"],
            },
            ReferenceJoin {
                name: "qk_date",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "qk_date",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_scal-date""#,
                display_fields: &["notice", "date"],
            },
            ReferenceJoin {
                name: "sk_currency",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "sk_currency",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_unit""#,
                display_fields: &["notice"],
            },
            ReferenceJoin {
                name: "ck_category",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "ck_category",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_category""#,
                display_fields: &["notice"],
            },
        ]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateConsignmentRequest {
    pub code: Option<String>,
    pub notice: Option<String>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_subject: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_object: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_contract: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_date: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub sk_currency: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub ck_category: Option<i64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateConsignmentRequest {
    pub code: Option<String>,
    pub notice: Option<String>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_subject: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_object: Option<i64>,
    /// 关联合同 id；`0` 为清除哨兵（置 NULL，zuid 永不为 0）
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_contract: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_date: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub sk_currency: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub ck_category: Option<i64>,
    /// 运输线路 id（非本表列；更新时透传到创建期产品 comments.traffic_line_id，
    /// 列表/详情经产品 comments 读取线路——修复编辑改线路不生效）
    #[serde(with = "common::serde_zuid::opt", default)]
    pub traffic_line_id: Option<i64>,
    /// 货量（吨，数值）——非本表列；更新 qk_total 指向的标量真值，
    /// 修复编辑货量仅塞 comments 不更新标量（公路委托读 qk_total mark 不变）
    #[serde(default)]
    pub volume: Option<f64>,
    /// 运费（金额，数值）——非本表列；更新 qk_amount 指向的 scal-amount 标量真值，
    /// 批注 a2bd97b6：行编辑运费仅写 comments 摘要文本（读侧不解析）→ 列表运费不变
    #[serde(default)]
    pub amount: Option<f64>,
}

// Waybill 共享 zc_id_orde-land，通过 ck_category='waybill' 区分
pub type Waybill = Consignment;
pub type CreateWaybillRequest = CreateConsignmentRequest;
pub type UpdateWaybillRequest = UpdateConsignmentRequest;

// ═══════════════════════════════════════════════
// P0: 车辆 — zc_id_stor-ctn-vehicle
// ═══════════════════════════════════════════════

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Vehicle {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub code: Option<String>,
    pub notice: Option<String>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub sk_unit: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_trustee: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_w_capacity: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_v_capacity: Option<i64>,
    // 模型升级后 qk_v/qk_w/qk_c_capacity、sk_currency 列已回归（schema-info 实测）；
    // sk_v_unit/sk_w_unit 已移除且不再需要——统一吨/立方米模式单位隐含于标量 notice
    //（批注 80e456cb 幻影字段 500 的历史处置已由模型升级对齐）
    #[sqlx(rename = "ck_r-type")]
    #[serde(with = "common::serde_zuid::opt", default)]
    pub ck_r_type: Option<i64>,
    #[sqlx(default)]
    pub _refs: Option<Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl Identifiable for Vehicle {
    fn id(&self) -> i64 {
        self.id
    }
}
impl AliothDbEntity for Vehicle {
    fn table_name() -> &'static str {
        "\"isahl\".\"zc_id_stor-ctn-vehicle\""
    }
    const SELECT_FIELDS: &'static str = "id, code, notice, comments, sk_unit, fk_trustee, qk_w_capacity, qk_v_capacity, \"ck_r-type\", created_at, updated_at, deleted_at";
    const ENTITY_NAME: &'static str = "vehicle";
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = false;
}

impl HasReferenceJoins for Vehicle {
    fn reference_joins() -> Vec<ReferenceJoin> {
        vec![
            ReferenceJoin {
                name: "sk_unit",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "sk_unit",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_unit""#,
                display_fields: &["notice"],
            },
            ReferenceJoin {
                name: "fk_trustee",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "fk_trustee",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_subjects""#,
                display_fields: &["notice", "code"],
            },
            ReferenceJoin {
                name: "qk_capacity",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "qk_capacity",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_scal-weight""#,
                display_fields: &["notice", "mark"],
            },
            ReferenceJoin {
                name: "qk_v_capacity",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "qk_v_capacity",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_scal-volume""#,
                display_fields: &["notice", "mark"],
            },
            ReferenceJoin {
                name: "ck_r-type",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "ck_r-type",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_cons-r-type-cate""#,
                display_fields: &["notice", "code"],
            },
        ]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateVehicleRequest {
    pub code: Option<String>,
    pub notice: Option<String>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub sk_unit: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_trustee: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_w_capacity: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub ck_r_type: Option<i64>,
    /// 载重结构化数值（吨，标准制式统一存储）——物化 zc_id_scal-weight 标量行挂 qk_w_capacity
    pub capacity_ton: Option<f64>,
    /// 体积容量结构化数值（立方米，统一存储）——物化 zc_id_scal-volume 标量行挂 qk_v_capacity
    pub capacity_m3: Option<f64>,
    /// 生命周期状态码（ST-VH-*，zc_id_stus-vehicle 字典）——物化 primary-status
    pub status_code: Option<String>,
    /// 位置上报经纬度（WGS84）——物化 zc_id_geog-point 行并挂 qk_point
    pub point_lng: Option<f64>,
    pub point_lat: Option<f64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateVehicleRequest {
    pub code: Option<String>,
    pub notice: Option<String>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub sk_unit: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_trustee: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_w_capacity: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub ck_r_type: Option<i64>,
    /// 载重结构化数值（吨，标准制式统一存储）——物化 zc_id_scal-weight 标量行挂 qk_w_capacity
    pub capacity_ton: Option<f64>,
    /// 体积容量结构化数值（立方米，统一存储）——物化 zc_id_scal-volume 标量行挂 qk_v_capacity
    pub capacity_m3: Option<f64>,
    /// 生命周期状态码（ST-VH-*，zc_id_stus-vehicle 字典）——物化 primary-status
    pub status_code: Option<String>,
    /// 位置上报经纬度（WGS84）——物化 zc_id_geog-point 行并挂 qk_point
    pub point_lng: Option<f64>,
    pub point_lat: Option<f64>,
}

// ═══════════════════════════════════════════════
// TrafficLine — zc_id_stor-traffic_line
// ═══════════════════════════════════════════════

// ═══════════════════════════════════════════════
// TrafficLine — zc_id_stor-traffic_line
// ═══════════════════════════════════════════════

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct TrafficLine {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub code: Option<String>,
    pub notice: Option<String>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_trustee: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_path: Option<i64>,
    #[sqlx(default)]
    pub _refs: Option<Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl Identifiable for TrafficLine {
    fn id(&self) -> i64 {
        self.id
    }
}

impl AliothDbEntity for TrafficLine {
    fn table_name() -> &'static str {
        "\"isahl\".\"zc_id_stor-traffic_line\""
    }
    const SELECT_FIELDS: &'static str =
        "id, code, notice, comments, fk_trustee, qk_path, created_at, updated_at, deleted_at";
    const ENTITY_NAME: &'static str = "traffic_line";
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = false;
}

impl HasReferenceJoins for TrafficLine {
    fn reference_joins() -> Vec<ReferenceJoin> {
        vec![
            ReferenceJoin {
                name: "fk_trustee",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "fk_trustee",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_subjects""#,
                display_fields: &["notice", "code"],
            },
            ReferenceJoin {
                name: "qk_path",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "qk_path",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_geom-path""#,
                display_fields: &["notice", "code", "ak_nodes"],
            },
        ]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateTrafficLineRequest {
    pub code: Option<String>,
    pub notice: Option<String>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_trustee: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_path: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateTrafficLineRequest {
    pub code: Option<String>,
    pub notice: Option<String>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_trustee: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_path: Option<i64>,
}

// ═══════════════════════════════════════════════
// FreightProduct — zc_id_prod-freight_road-sales
// ═══════════════════════════════════════════════

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct FreightProduct {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub code: Option<String>,
    pub notice: Option<String>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_previous: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub ck_vehicle_form: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_price: Option<i64>,
    /// 需求方/客户主体（产品属权，双层模型下可为 A 或 B；见 wz-product-family spec）
    #[sqlx(rename = "fk_subj-demand")]
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_subj_demand: Option<i64>,
    /// 销售方/提供方主体（委托链=B、运单链=C）
    #[sqlx(rename = "fk_subj-provider")]
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_subj_provider: Option<i64>,
    /// 范例/实例（_t_，LifecycleBizTemplate 派生）
    #[sqlx(rename = "_t_")]
    pub t_type: Option<String>,
    #[sqlx(default)]
    pub _refs: Option<Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl Identifiable for FreightProduct {
    fn id(&self) -> i64 {
        self.id
    }
}

impl AliothDbEntity for FreightProduct {
    fn table_name() -> &'static str {
        "\"isahl\".\"zc_id_prod-freight_road-sales\""
    }
    const ENTITY_NAME: &'static str = "freight_product";
    const SELECT_FIELDS: &'static str = r#"id, code, notice, comments, fk_previous, ck_vehicle-form, qk_price, "fk_subj-demand", "fk_subj-provider", "_t_", created_at, updated_at, deleted_at"#;
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = false;
}

impl HasReferenceJoins for FreightProduct {
    fn reference_joins() -> Vec<ReferenceJoin> {
        vec![ReferenceJoin {
            name: "fk_previous",
            card: Card::ToOne,
            kind: JoinKind::Forward {
                local_fk: "fk_previous",
                target_key: "id",
            },
            target_table: r#"isahl."zc_id_orde-land""#,
            display_fields: &["code", "notice"],
        }]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateFreightProductRequest {
    pub code: Option<String>,
    pub notice: Option<String>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_previous: Option<i64>,
    #[serde(rename = "ck_vehicle-form")]
    #[serde(with = "common::serde_zuid::opt", default)]
    pub ck_vehicle_form: Option<i64>,
    #[serde(rename = "qk_price")]
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_price: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateFreightProductRequest {
    pub code: Option<String>,
    pub notice: Option<String>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_previous: Option<i64>,
    #[serde(rename = "ck_vehicle-form")]
    #[serde(with = "common::serde_zuid::opt", default)]
    pub ck_vehicle_form: Option<i64>,
    #[serde(rename = "qk_price")]
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_price: Option<i64>,
}
// ═══════════════════════════════════════════════
// P0: 运输追踪 — zc_id_oper-transport_tracking
// ═══════════════════════════════════════════════

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct TransportTracking {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub code: Option<String>,
    pub notice: Option<String>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_operator: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_subject: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_approve: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_arrived: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_work_duration: Option<i64>,
    #[sqlx(rename = "ck_cate-wh")]
    #[serde(with = "common::serde_zuid::opt", default)]
    pub ck_cate_wh: Option<i64>,
    #[sqlx(rename = "ck_cate-biz")]
    #[serde(with = "common::serde_zuid::opt", default)]
    pub ck_cate_biz: Option<i64>,
    #[sqlx(default)]
    pub _refs: Option<Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl Identifiable for TransportTracking {
    fn id(&self) -> i64 {
        self.id
    }
}
impl AliothDbEntity for TransportTracking {
    fn table_name() -> &'static str {
        "\"isahl\".\"zc_id_oper-transport_tracking\""
    }
    // fix-fk-approve-residual-consumers：fk_approve 物理列已移除——
    // 经 operation_rr_event 桥子查询派生（ref_left=本行, ref_right=even-approve 事件）
    const SELECT_FIELDS: &'static str = r#"id, code, notice, comments, fk_operator, fk_subject, (SELECT oe.ref_right FROM isahl.zc_id_operation_rr_event oe JOIN isahl."zc_id_even-approve" ea ON ea.id = oe.ref_right AND ea.deleted_at IS NULL WHERE oe.ref_left = e.id AND oe.deleted_at IS NULL ORDER BY oe.created_at LIMIT 1) AS fk_approve, qk_arrived, qk_work_duration, "ck_cate-wh", "ck_cate-biz", created_at, updated_at, deleted_at"#;
    const ENTITY_NAME: &'static str = "transport_tracking";
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = false;
}

impl HasReferenceJoins for TransportTracking {
    fn reference_joins() -> Vec<ReferenceJoin> {
        vec![
            ReferenceJoin {
                name: "fk_operator",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "fk_operator",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_subjects""#,
                display_fields: &["notice", "code"],
            },
            ReferenceJoin {
                name: "fk_subject",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "fk_subject",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_subjects""#,
                display_fields: &["notice", "code"],
            },
            ReferenceJoin {
                name: "fk_approve",
                card: Card::ToOne,
                kind: JoinKind::Junction {
                    junction_table: "isahl.zc_id_operation_rr_event",
                    source_fk: "ref_left",
                    target_fk: "ref_right",
                    order_by: Some("created_at"),
                },
                target_table: r#"isahl."zc_id_even-approve""#,
                display_fields: &["notice"],
            },
            ReferenceJoin {
                name: "qk_arrived",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "qk_arrived",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_scal-date""#,
                display_fields: &["notice", "date"],
            },
            ReferenceJoin {
                name: "qk_work_duration",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "qk_work_duration",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_scal-common""#,
                display_fields: &["notice", "mark"],
            },
            ReferenceJoin {
                name: "ck_cate_wh",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "ck_cate-wh",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_category""#,
                display_fields: &["notice"],
            },
            ReferenceJoin {
                name: "ck_cate_biz",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "ck_cate-biz",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_category""#,
                display_fields: &["notice"],
            },
        ]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateTransportTrackingRequest {
    pub code: Option<String>,
    pub notice: Option<String>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_operator: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_subject: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_approve: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_arrived: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_work_duration: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub ck_cate_wh: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub ck_cate_biz: Option<i64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateTransportTrackingRequest {
    pub code: Option<String>,
    pub notice: Option<String>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_operator: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_subject: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_approve: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_arrived: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_work_duration: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub ck_cate_wh: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub ck_cate_biz: Option<i64>,
}

// ═══════════════════════════════════════════════
// P1: 对账核销
// ═══════════════════════════════════════════════

// 交易订单明细 — zc_id_deta-trade_order
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct TradeOrder {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub code: Option<String>,
    pub notice: Option<String>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_goods: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_demand: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_delivery: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_deal: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_purchase: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_biller: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_counterparty: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_price: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_qty: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_amount: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub sk_currency: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub sk_unit: Option<i64>,
    #[sqlx(default)]
    pub _refs: Option<Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl Identifiable for TradeOrder {
    fn id(&self) -> i64 {
        self.id
    }
}
impl AliothDbEntity for TradeOrder {
    fn table_name() -> &'static str {
        "\"isahl\".\"zc_id_deta-trade_order\""
    }
    const SELECT_FIELDS: &'static str = "id, code, notice, comments, fk_goods, fk_demand, fk_delivery, fk_deal, fk_biller, fk_counterparty, qk_price, qk_qty, qk_amount, sk_currency, sk_unit, created_at, updated_at, deleted_at";
    const ENTITY_NAME: &'static str = "trade_order";
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = false;
}

impl HasReferenceJoins for TradeOrder {
    fn reference_joins() -> Vec<ReferenceJoin> {
        vec![
            // 语义对齐：fk_goods→销售范例、fk_deal→销售实例、fk_demand→买方诉求、
            // fk_delivery→制造产品、fk_purchase→采购产品
            ReferenceJoin {
                name: "fk_goods",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "fk_goods",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_prod-sales""#,
                display_fields: &["notice", "code"],
            },
            ReferenceJoin {
                name: "fk_demand",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "fk_demand",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_prod-request""#,
                display_fields: &["notice", "code"],
            },
            ReferenceJoin {
                name: "fk_delivery",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "fk_delivery",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_prod-made""#,
                display_fields: &["notice", "code"],
            },
            ReferenceJoin {
                name: "fk_deal",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "fk_deal",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_prod-sales""#,
                display_fields: &["notice", "code"],
            },
            // 卖方（fk_biller）/ 买方（fk_counterparty）主体
            ReferenceJoin {
                name: "fk_biller",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "fk_biller",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_subjects""#,
                display_fields: &["notice", "code"],
            },
            ReferenceJoin {
                name: "fk_counterparty",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "fk_counterparty",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_subjects""#,
                display_fields: &["notice", "code"],
            },
            ReferenceJoin {
                name: "qk_price",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "qk_price",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_scal-price""#,
                display_fields: &["notice", "mark"],
            },
            ReferenceJoin {
                name: "qk_qty",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "qk_qty",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_scal-common""#,
                display_fields: &["notice", "mark"],
            },
            ReferenceJoin {
                name: "qk_amount",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "qk_amount",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_scal-amount""#,
                display_fields: &["notice", "mark"],
            },
            ReferenceJoin {
                name: "sk_currency",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "sk_currency",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_unit-currency""#,
                display_fields: &["notice", "code"],
            },
            ReferenceJoin {
                name: "sk_unit",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "sk_unit",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_unit""#,
                display_fields: &["notice"],
            },
        ]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateTradeOrderRequest {
    pub code: Option<String>,
    pub notice: Option<String>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_goods: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_demand: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_delivery: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_deal: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_purchase: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_biller: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_counterparty: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_price: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_qty: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_amount: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub sk_currency: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub sk_unit: Option<i64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateTradeOrderRequest {
    pub code: Option<String>,
    pub notice: Option<String>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_goods: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_demand: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_delivery: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_deal: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_purchase: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_biller: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_counterparty: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_price: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_qty: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_amount: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub sk_currency: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub sk_unit: Option<i64>,
}

// 清算账单 — zc_id_bill-check (也是 Receipt 收款单)
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct BillCheck {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub code: Option<String>,
    pub notice: Option<String>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_settle: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_account: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_amount: Option<i64>,
    #[sqlx(rename = "qk_write-off")]
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_write_off: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_tax: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub sk_currency: Option<i64>,
    #[sqlx(default)]
    pub _refs: Option<Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl Identifiable for BillCheck {
    fn id(&self) -> i64 {
        self.id
    }
}
impl AliothDbEntity for BillCheck {
    fn table_name() -> &'static str {
        "\"isahl\".\"zc_id_bill-check\""
    }
    const SELECT_FIELDS: &'static str = "id, code, notice, comments, fk_settle, fk_account, qk_amount, \"qk_write-off\", qk_tax, sk_currency, created_at, updated_at, deleted_at";
    const ENTITY_NAME: &'static str = "bill_check";
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = false;
}

impl HasReferenceJoins for BillCheck {
    fn reference_joins() -> Vec<ReferenceJoin> {
        vec![
            ReferenceJoin {
                name: "fk_settle",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "fk_settle",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_subjects""#,
                display_fields: &["notice", "code"],
            },
            ReferenceJoin {
                name: "fk_account",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "fk_account",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_stor-acc-business""#,
                display_fields: &["notice", "code"],
            },
            ReferenceJoin {
                name: "qk_amount",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "qk_amount",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_scal-amount""#,
                display_fields: &["notice", "mark"],
            },
            ReferenceJoin {
                name: "qk_write_off",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "qk_write-off",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_scal-amount""#,
                display_fields: &["notice", "mark"],
            },
            ReferenceJoin {
                name: "qk_tax",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "qk_tax",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_scal-amount""#,
                display_fields: &["notice", "mark"],
            },
            ReferenceJoin {
                name: "sk_currency",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "sk_currency",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_unit-currency""#,
                display_fields: &["notice", "code"],
            },
        ]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateBillCheckRequest {
    pub code: Option<String>,
    pub notice: Option<String>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_settle: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_account: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_amount: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_write_off: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_tax: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub sk_currency: Option<i64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateBillCheckRequest {
    pub code: Option<String>,
    pub notice: Option<String>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_settle: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_account: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_amount: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_write_off: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_tax: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub sk_currency: Option<i64>,
}

pub type Receipt = BillCheck;
pub type CreateReceiptRequest = CreateBillCheckRequest;
pub type UpdateReceiptRequest = UpdateBillCheckRequest;

// 清算明细 — zc_id_deta-bill-check
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct DetaBillCheck {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub code: Option<String>,
    pub notice: Option<String>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_list: Option<i64>,
    /// 不存在于表 DDL（预存漂移）——保留字段兼容，SQL 不引用
    #[sqlx(default)]
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_order: Option<i64>,
    #[sqlx(default)]
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_matter: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub ck_category: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_qty: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_price: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_amount: Option<i64>,
    #[sqlx(default)]
    pub _refs: Option<Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl Identifiable for DetaBillCheck {
    fn id(&self) -> i64 {
        self.id
    }
}
impl AliothDbEntity for DetaBillCheck {
    fn table_name() -> &'static str {
        "\"isahl\".\"zc_id_deta-bill-check\""
    }
    const SELECT_FIELDS: &'static str = "id, code, notice, comments, fk_list, ck_category, qk_qty, qk_price, qk_amount, created_at, updated_at, deleted_at";
    const ENTITY_NAME: &'static str = "deta_bill_check";
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = false;
}

impl HasReferenceJoins for DetaBillCheck {
    fn reference_joins() -> Vec<ReferenceJoin> {
        vec![
            ReferenceJoin {
                name: "fk_list",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "fk_list",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_bill-check""#,
                display_fields: &["notice", "code"],
            },
            ReferenceJoin {
                name: "ck_category",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "ck_category",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_category""#,
                display_fields: &["notice"],
            },
            ReferenceJoin {
                name: "qk_qty",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "qk_qty",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_scal-common""#,
                display_fields: &["notice", "mark"],
            },
            ReferenceJoin {
                name: "qk_price",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "qk_price",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_scal-price""#,
                display_fields: &["notice", "mark"],
            },
            ReferenceJoin {
                name: "qk_amount",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "qk_amount",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_scal-amount""#,
                display_fields: &["notice", "mark"],
            },
        ]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateDetaBillCheckRequest {
    pub code: Option<String>,
    pub notice: Option<String>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_list: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_order: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_matter: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub ck_category: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_qty: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_price: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_amount: Option<i64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateDetaBillCheckRequest {
    pub code: Option<String>,
    pub notice: Option<String>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_list: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_order: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_matter: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub ck_category: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_qty: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_price: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_amount: Option<i64>,
}

// ═══════════════════════════════════════════════
// Invoice — zc_id_invoice (SalesInvoice / PurchaseInvoice)
// ═══════════════════════════════════════════════

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Invoice {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub code: Option<String>,
    pub notice: Option<String>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_sender: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_recipient: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_issue_date: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_amount: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_tax: Option<i64>,
    #[sqlx(default)]
    pub _refs: Option<Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl Identifiable for Invoice {
    fn id(&self) -> i64 {
        self.id
    }
}
impl AliothDbEntity for Invoice {
    fn table_name() -> &'static str {
        "\"isahl\".\"zc_id_invo-electric\""
    }
    const SELECT_FIELDS: &'static str = "id, code, notice, comments, fk_sender, fk_recipient, qk_issue_date, qk_amount, qk_tax, created_at, updated_at, deleted_at";
    const ENTITY_NAME: &'static str = "invoice";
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = false;
}

impl HasReferenceJoins for Invoice {
    fn reference_joins() -> Vec<ReferenceJoin> {
        vec![
            ReferenceJoin {
                name: "fk_sender",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "fk_sender",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_subjects""#,
                display_fields: &["notice", "code"],
            },
            ReferenceJoin {
                name: "fk_recipient",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "fk_recipient",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_subjects""#,
                display_fields: &["notice", "code"],
            },
            ReferenceJoin {
                name: "qk_issue_date",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "qk_issue_date",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_scal-date""#,
                display_fields: &["notice", "date"],
            },
            ReferenceJoin {
                name: "qk_amount",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "qk_amount",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_scal-amount""#,
                display_fields: &["notice", "mark"],
            },
            ReferenceJoin {
                name: "qk_tax",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "qk_tax",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_scal-amount""#,
                display_fields: &["notice", "mark"],
            },
        ]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateInvoiceRequest {
    pub code: Option<String>,
    pub notice: Option<String>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_sender: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_recipient: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_issue_date: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_amount: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_tax: Option<i64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateInvoiceRequest {
    pub code: Option<String>,
    pub notice: Option<String>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_sender: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_recipient: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_issue_date: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_amount: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_tax: Option<i64>,
}

pub type SalesInvoice = Invoice;
pub type PurchaseInvoice = Invoice;

// 发票明细 — zc_id_deta-invoice
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct InvoiceDetail {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub code: Option<String>,
    pub notice: Option<String>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_list: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_subject: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub ck_category: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_qty: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_price: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_amount: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_tax_amount: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_tax_ratio: Option<i64>,
    #[sqlx(default)]
    pub _refs: Option<Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl Identifiable for InvoiceDetail {
    fn id(&self) -> i64 {
        self.id
    }
}
impl AliothDbEntity for InvoiceDetail {
    fn table_name() -> &'static str {
        "\"isahl\".\"zc_id_deta-invoice\""
    }
    const SELECT_FIELDS: &'static str = "id, code, notice, comments, fk_list, fk_subject, ck_category, qk_qty, qk_price, qk_amount, qk_tax_amount, qk_tax_ratio, created_at, updated_at, deleted_at";
    const ENTITY_NAME: &'static str = "invoice_detail";
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = false;
}

impl HasReferenceJoins for InvoiceDetail {
    fn reference_joins() -> Vec<ReferenceJoin> {
        vec![
            ReferenceJoin {
                name: "fk_list",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "fk_list",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_invo-electric""#,
                display_fields: &["notice", "code"],
            },
            ReferenceJoin {
                name: "fk_subject",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "fk_subject",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_subjects""#,
                display_fields: &["notice", "code"],
            },
            ReferenceJoin {
                name: "ck_category",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "ck_category",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_category""#,
                display_fields: &["notice"],
            },
            ReferenceJoin {
                name: "qk_qty",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "qk_qty",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_scal-common""#,
                display_fields: &["notice", "mark"],
            },
            ReferenceJoin {
                name: "qk_price",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "qk_price",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_scal-price""#,
                display_fields: &["notice", "mark"],
            },
            ReferenceJoin {
                name: "qk_amount",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "qk_amount",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_scal-amount""#,
                display_fields: &["notice", "mark"],
            },
            ReferenceJoin {
                name: "qk_tax_amount",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "qk_tax_amount",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_scal-amount""#,
                display_fields: &["notice", "mark"],
            },
            ReferenceJoin {
                name: "qk_tax_ratio",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "qk_tax_ratio",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_scal-common""#,
                display_fields: &["notice", "mark"],
            },
        ]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateInvoiceDetailRequest {
    pub code: Option<String>,
    pub notice: Option<String>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_list: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_subject: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub ck_category: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_qty: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_price: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_amount: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_tax_amount: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_tax_ratio: Option<i64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateInvoiceDetailRequest {
    pub code: Option<String>,
    pub notice: Option<String>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_list: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_subject: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub ck_category: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_qty: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_price: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_amount: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_tax_amount: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_tax_ratio: Option<i64>,
}

// ═══════════════════════════════════════════════
// Payment — zc_id_oper-payment
// ═══════════════════════════════════════════════

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Payment {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub code: Option<String>,
    pub notice: Option<String>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_operator: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_subject: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_approve: Option<i64>,
    #[sqlx(rename = "ck_cate-wh")]
    #[serde(with = "common::serde_zuid::opt", default)]
    pub ck_cate_wh: Option<i64>,
    #[sqlx(rename = "ck_cate-biz")]
    #[serde(with = "common::serde_zuid::opt", default)]
    pub ck_cate_biz: Option<i64>,
    #[sqlx(default)]
    pub _refs: Option<Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl Identifiable for Payment {
    fn id(&self) -> i64 {
        self.id
    }
}
impl AliothDbEntity for Payment {
    fn table_name() -> &'static str {
        "\"isahl\".\"zc_id_oper-payment\""
    }
    const SELECT_FIELDS: &'static str = r#"id, code, notice, comments, fk_operator, fk_subject, (SELECT oe.ref_right FROM isahl.zc_id_operation_rr_event oe JOIN isahl."zc_id_even-approve" ea ON ea.id = oe.ref_right AND ea.deleted_at IS NULL WHERE oe.ref_left = e.id AND oe.deleted_at IS NULL ORDER BY oe.created_at LIMIT 1) AS fk_approve, "ck_cate-wh", "ck_cate-biz", created_at, updated_at, deleted_at"#;
    const ENTITY_NAME: &'static str = "payment";
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = false;
    fn column_renames() -> Vec<(&'static str, &'static str)> {
        vec![("ck_cate_biz", "ck_cate-biz"), ("ck_cate_wh", "ck_cate-wh")]
    }
}

impl HasReferenceJoins for Payment {
    fn reference_joins() -> Vec<ReferenceJoin> {
        vec![
            ReferenceJoin {
                name: "fk_operator",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "fk_operator",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_subjects""#,
                display_fields: &["notice", "code"],
            },
            ReferenceJoin {
                name: "fk_subject",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "fk_subject",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_subjects""#,
                display_fields: &["notice", "code"],
            },
            ReferenceJoin {
                name: "fk_approve",
                card: Card::ToOne,
                kind: JoinKind::Junction {
                    junction_table: "isahl.zc_id_operation_rr_event",
                    source_fk: "ref_left",
                    target_fk: "ref_right",
                    order_by: Some("created_at"),
                },
                target_table: r#"isahl."zc_id_even-approve""#,
                display_fields: &["notice"],
            },
            ReferenceJoin {
                name: "ck_cate_wh",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "ck_cate-wh",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_category""#,
                display_fields: &["notice"],
            },
            ReferenceJoin {
                name: "ck_cate_biz",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "ck_cate-biz",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_category""#,
                display_fields: &["notice"],
            },
        ]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatePaymentRequest {
    pub code: Option<String>,
    pub notice: Option<String>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_operator: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_subject: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_approve: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub ck_cate_wh: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub ck_cate_biz: Option<i64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdatePaymentRequest {
    pub code: Option<String>,
    pub notice: Option<String>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_operator: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_subject: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_approve: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub ck_cate_wh: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub ck_cate_biz: Option<i64>,
}

// ═══════════════════════════════════════════════
// SettlementBank — isahl.zc_id_stat-smt-bank
// ═══════════════════════════════════════════════

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct SettlementBank {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_date: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_income: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_outgo: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_balance: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_total: Option<i64>,
    #[sqlx(rename = "qk_exchange-rate")]
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_exchange_rate: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl Identifiable for SettlementBank {
    fn id(&self) -> i64 {
        self.id
    }
}
impl AliothDbEntity for SettlementBank {
    fn table_name() -> &'static str {
        "\"isahl\".\"zc_id_stat-smt-bank\""
    }
    const SELECT_FIELDS: &'static str = r#"id, qk_date, qk_income, qk_outgo, qk_balance, qk_total, "qk_exchange-rate", created_at, updated_at, deleted_at"#;
    const ENTITY_NAME: &'static str = "settlement_bank";
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = false;
}

impl HasReferenceJoins for SettlementBank {
    fn reference_joins() -> Vec<ReferenceJoin> {
        vec![]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSettlementBankRequest {
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_date: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_income: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_outgo: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_balance: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_total: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_exchange_rate: Option<i64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateSettlementBankRequest {
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_date: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_income: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_outgo: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_balance: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_total: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_exchange_rate: Option<i64>,
}

// ═══════════════════════════════════════════════
// SettlementCash — isahl.zc_id_stat-smt-cash
// ═══════════════════════════════════════════════

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct SettlementCash {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_date: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_income: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_outgo: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_amount: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_total: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl Identifiable for SettlementCash {
    fn id(&self) -> i64 {
        self.id
    }
}
impl AliothDbEntity for SettlementCash {
    fn table_name() -> &'static str {
        "\"isahl\".\"zc_id_stat-smt-cash\""
    }
    const SELECT_FIELDS: &'static str =
        "id, qk_date, qk_income, qk_outgo, qk_amount, qk_total, created_at, updated_at, deleted_at";
    const ENTITY_NAME: &'static str = "settlement_cash";
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = false;
}

impl HasReferenceJoins for SettlementCash {
    fn reference_joins() -> Vec<ReferenceJoin> {
        vec![]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSettlementCashRequest {
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_date: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_income: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_outgo: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_amount: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_total: Option<i64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateSettlementCashRequest {
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_date: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_income: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_outgo: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_amount: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_total: Option<i64>,
}

// ═══════════════════════════════════════════════
// SettlementChannel — isahl.zc_id_stat-smt-channel
// ═══════════════════════════════════════════════

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct SettlementChannel {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_date: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_income: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_outgo: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_amount: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_total: Option<i64>,
    #[sqlx(rename = "qk_exchange-rate")]
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_exchange_rate: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl Identifiable for SettlementChannel {
    fn id(&self) -> i64 {
        self.id
    }
}
impl AliothDbEntity for SettlementChannel {
    fn table_name() -> &'static str {
        "\"isahl\".\"zc_id_stat-smt-channel\""
    }
    const SELECT_FIELDS: &'static str = r#"id, qk_date, qk_income, qk_outgo, qk_amount, qk_total, "qk_exchange-rate", created_at, updated_at, deleted_at"#;
    const ENTITY_NAME: &'static str = "settlement_channel";
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = false;
}

impl HasReferenceJoins for SettlementChannel {
    fn reference_joins() -> Vec<ReferenceJoin> {
        vec![]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSettlementChannelRequest {
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_date: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_income: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_outgo: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_amount: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_total: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_exchange_rate: Option<i64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateSettlementChannelRequest {
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_date: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_income: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_outgo: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_amount: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_total: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_exchange_rate: Option<i64>,
}

// ═══════════════════════════════════════════════
// Fence — 围栏几何族（add-fence-geometry-types）：
//   circle（方圆）→ zc_id_geog-circle（circle 几何 + qk_radius 半径标量引用）
//   area（区域）→ zc_id_geog-area（box 几何，对角两点矩形环）
//   polygon（自定义）→ zc_id_geog-polygon（polygon 几何，≥3 点闭合环）
//   类型由前端显式配置，后端按类型分派叶表；读路径经继承根跨叶表。
// ═══════════════════════════════════════════════

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Fence {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub notice: Option<String>,
    pub code: Option<String>,
    pub comments: Option<String>,
    /// 圆心（PostGIS geometry(Point,4326)，GeoJSON 输出；仅 circle 类型行非空）
    pub circle: Option<Value>,
    /// 围栏类型（circle/area/polygon——tableoid 解析物理叶表回标；add-fence-geometry-types）
    #[sqlx(default)]
    pub fence_type: Option<String>,
    /// 统一几何（GeoJSON：circle 行=圆心 Point，area 行=矩形 Polygon，polygon 行=多边形 Polygon）
    #[sqlx(default)]
    pub geometry: Option<Value>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub sk_unit: Option<i64>,
    /// 半径（qk_radius → zc_id_scal-distance 标量行，_refs 解析 mark 值）
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_radius: Option<i64>,
    pub t_color_: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
    #[sqlx(default)]
    pub _refs: Option<Value>,
}

impl Identifiable for Fence {
    fn id(&self) -> i64 {
        self.id
    }
}
impl AliothDbEntity for Fence {
    fn table_name() -> &'static str {
        r#""isahl"."zc_id_geog-circle""#
    }
    // add-fence-geometry-types：实体表绑定叶表（原 zc_id_geom-circle 为非叶中层表，
    // 读路径恒空/删路径脱靶——fence-gis 规约禁止）；list/get/update/delete 由
    // FenceRepository 按类型分派三张叶表，本绑定仅作通用路径兜底。
    const SELECT_FIELDS: &'static str = r#"id, notice, code, comments,
              ST_AsGeoJSON(circle)::jsonb as circle, sk_unit, qk_radius,
              t_color_, created_at, updated_at, deleted_at"#;
    const ENTITY_NAME: &'static str = "fence";
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = false;
}
impl HasReferenceJoins for Fence {
    fn reference_joins() -> Vec<ReferenceJoin> {
        vec![ReferenceJoin {
            name: "sk_unit",
            card: Card::ToOne,
            kind: JoinKind::Forward {
                local_fk: "sk_unit",
                target_key: "id",
            },
            target_table: r#"isahl."zc_id_unit""#,
            // 方案 B：sk_unit 语义 = 图商坐标系（WGS84/GCJ-02/BD-09），显示 code
            display_fields: &["code", "notice"],
        }]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateFenceRequest {
    pub notice: Option<String>,
    pub code: Option<String>,
    pub comments: Option<String>,
    /// 围栏类型（circle=方圆/area=区域/polygon=自定义；缺省 circle；add-fence-geometry-types）
    #[serde(default)]
    pub fence_type: Option<String>,
    /// 圆心：GeoJSON Point object（{type:'Point',coordinates:[lng,lat]}）、
    /// WKT 字符串，或 {lng,lat}/{lat,lng} 简单对象（自动转点）
    pub circle: Option<Value>,
    pub radius: Option<f64>,
    /// 区域围栏对角两点（area 类型必填）：{southwest:{lng,lat}, northeast:{lng,lat}}
    pub bounds: Option<Value>,
    /// 自定义多边形顶点（polygon 类型必填，≥3 点）：[{lng,lat},...]
    pub points: Option<Value>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub sk_unit: Option<i64>,
    /// 图商坐标系 code（WGS84/GCJ-02/BD-09，方案 B：sk_unit 语义=坐标系）。
    /// 缺省按 WGS84 解析；GCJ-02/BD-09 必须显式声明。
    pub coord_sys: Option<String>,
    pub t_color_: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateFenceRequest {
    pub notice: Option<String>,
    pub code: Option<String>,
    pub comments: Option<String>,
    /// 围栏类型（与创建同语义；None=不改类型几何，仅改公共列）
    #[serde(default)]
    pub fence_type: Option<String>,
    /// 圆心：GeoJSON Point object、WKT 字符串，或 {lng,lat}/{lat,lng}
    pub circle: Option<Value>,
    pub radius: Option<f64>,
    /// 区域围栏对角两点（area 类型）：{southwest:{lng,lat}, northeast:{lng,lat}}
    pub bounds: Option<Value>,
    /// 自定义多边形顶点（polygon 类型，≥3 点）：[{lng,lat},...]
    pub points: Option<Value>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub sk_unit: Option<i64>,
    /// 图商坐标系 code（WGS84/GCJ-02/BD-09）
    pub coord_sys: Option<String>,
    pub t_color_: Option<String>,
}

// ═══════════════════════════════════════════════
// Seal — zc_id_devi-seal（铅封管理：铅封号/状态/备注——批注 d59b405f）
// ═══════════════════════════════════════════════

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Seal {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub notice: Option<String>,
    pub code: Option<String>,
    pub comments: Option<String>,
    /// 铅封类型（ck_category → zc_id_cate-seal 字典行 id；add-wz-seal-batch-creation）
    #[serde(with = "common::serde_zuid::opt", default)]
    pub ck_category: Option<i64>,
    /// 铅封类型 code（SELECT 子查询，展示用）
    #[sqlx(default)]
    pub seal_type: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
    /// 关联运单号（批注轮 68：封签对运单进行——seal→装车条 voucher→运单桥反查）
    #[sqlx(default)]
    pub waybill_no: Option<String>,
    #[sqlx(default)]
    pub _refs: Option<Value>,
}

impl Identifiable for Seal {
    fn id(&self) -> i64 {
        self.id
    }
}
impl AliothDbEntity for Seal {
    fn table_name() -> &'static str {
        r#""isahl"."zc_id_devi-seal""#
    }
    const SELECT_FIELDS: &'static str = r#"e.id, e.notice, e.code, e.comments, e.ck_category, e.created_at, e.updated_at, e.deleted_at,
           (SELECT c.code FROM "isahl"."zc_id_cate-seal" c
            WHERE c.id = e.ck_category AND c.deleted_at IS NULL) AS seal_type,
           COALESCE(
             (SELECT w.code FROM "isahl"."zc_id_orde-land" w
              JOIN "isahl"."zc_id_orde-traffic_rr_tsp-voucher" bv ON bv.ref_left = w.id AND bv.deleted_at IS NULL
              JOIN "isahl"."zc_id_stat-tsp-voucher" v ON v.id = bv.ref_right AND v.deleted_at IS NULL
              JOIN "isahl"."zc_id_tsp-voucher_rr_devi-seal" ds ON ds.ref_left = v.id AND ds.deleted_at IS NULL
              WHERE ds.ref_right = e.id AND w.deleted_at IS NULL LIMIT 1),
             -- 批注轮 69：管理页新增/编辑关联运单（comments JSON waybill_id）
             (SELECT w2.code FROM "isahl"."zc_id_orde-land" w2
              WHERE w2.id = (CASE WHEN e.comments IS JSON OBJECT
                                  THEN (e.comments::json->>'waybill_id')::bigint END)
                AND w2.deleted_at IS NULL LIMIT 1)
           ) AS waybill_no"#;
    const ENTITY_NAME: &'static str = "seal";
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = false;
}
impl HasReferenceJoins for Seal {
    fn reference_joins() -> Vec<ReferenceJoin> {
        vec![ReferenceJoin {
            name: "ck_category",
            card: Card::ToOne,
            kind: JoinKind::Forward {
                local_fk: "ck_category",
                target_key: "id",
            },
            target_table: r#"isahl."zc_id_cate-seal""#,
            display_fields: &["code", "notice"],
        }]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateSealRequest {
    pub notice: Option<String>,
    pub code: Option<String>,
    pub comments: Option<String>,
    /// 铅封类型 code（如 TARPAULIN/CONTAINER；add-wz-seal-batch-creation）
    pub seal_type: Option<String>,
    /// 关联运单 id（批注轮 69：封签对运单进行——新增/编辑下拉选运单，comments JSON 承载）
    #[serde(with = "common::serde_zuid::opt", default)]
    pub waybill_id: Option<i64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSealRequest {
    pub notice: Option<String>,
    pub code: Option<String>,
    pub comments: Option<String>,
    /// 铅封类型 code（如 TARPAULIN/CONTAINER；add-wz-seal-batch-creation）
    pub seal_type: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub waybill_id: Option<i64>,
}

/// 批量创建铅封请求（POST /seals/batch；add-wz-seal-batch-creation，
/// refactor-dispatch-seal-code-generation 重构：去字典 o_number 依赖）
///
/// `sealType` 为铅封类型 code（固定常量列表，编号前缀）；`count` 缺省 1（1=单号、N=连号）。
/// `startCode` 缺省 → code 前缀自动续号（`<CODE>-000N`，服务端取该前缀最大尾部序号 +1）；
/// `startCode` 显式 → 从起始号等宽递增（铅封管理页手输场景保留）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateSealBatchRequest {
    pub seal_type: Option<String>,
    pub start_code: Option<String>,
    pub count: Option<i64>,
    pub notice: Option<String>,
    pub comments: Option<String>,
    /// 关联运单 id（comments JSON 承载，与单条创建一致）
    #[serde(with = "common::serde_zuid::opt", default)]
    pub waybill_id: Option<i64>,
}

// ═══════════════════════════════════════════════
// TransitRoute — zc_id_geom-path
// ═══════════════════════════════════════════════

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct TransitRoute {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub notice: Option<String>,
    pub code: Option<String>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt_seq", default)]
    pub ak_nodes: Option<Vec<i64>>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub sk_unit: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
    #[sqlx(default)]
    pub _refs: Option<Value>,
}

impl Identifiable for TransitRoute {
    fn id(&self) -> i64 {
        self.id
    }
}
impl AliothDbEntity for TransitRoute {
    fn table_name() -> &'static str {
        r#""isahl"."zc_id_geom-path""#
    }
    const SELECT_FIELDS: &'static str =
        r#"id, notice, code, comments, ak_nodes, sk_unit, created_at, updated_at, deleted_at"#;
    const ENTITY_NAME: &'static str = "transit_route";
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = false;
}
impl HasReferenceJoins for TransitRoute {
    fn reference_joins() -> Vec<ReferenceJoin> {
        vec![
            ReferenceJoin {
                name: "ak_nodes",
                card: Card::ToMany,
                kind: JoinKind::ArrayFk {
                    array_fk: "ak_nodes",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_geom-coordinate""#,
                display_fields: &["notice", "point"],
            },
            ReferenceJoin {
                name: "sk_unit",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "sk_unit",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_unit""#,
                display_fields: &["notice"],
            },
        ]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateTransitRouteRequest {
    pub notice: Option<String>,
    pub code: Option<String>,
    pub comments: Option<String>,
    pub ak_nodes: Option<Value>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub sk_unit: Option<i64>,
    /// 图商坐标系 code（WGS84/GCJ-02/BD-09，方案 B：sk_unit 语义=坐标系）
    pub coord_sys: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateTransitRouteRequest {
    pub notice: Option<String>,
    pub code: Option<String>,
    pub comments: Option<String>,
    pub ak_nodes: Option<Value>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub sk_unit: Option<i64>,
    /// 图商坐标系 code（WGS84/GCJ-02/BD-09）
    pub coord_sys: Option<String>,
}

// ═══════════════════════════════════════════════
// 库存统计关系 — zc_id_production_rr_storage（产品↔储元）
// （运力容量迁移：容量=qk_p_capacity→scal-common.mark，
//   占用/实际=qk_qty→scal-common.mark，无 _t_/fk_production/qk_date 列）
// ═══════════════════════════════════════════════

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct InventorySales {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub code: Option<String>,
    pub notice: Option<String>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub ref_left: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub ref_right: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_p_capacity: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_qty: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub sk_unit: Option<i64>,
    #[sqlx(default)]
    pub _refs: Option<Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl Identifiable for InventorySales {
    fn id(&self) -> i64 {
        self.id
    }
}
impl AliothDbEntity for InventorySales {
    fn table_name() -> &'static str {
        "\"isahl\".\"zc_id_production_rr_storage\""
    }
    const SELECT_FIELDS: &'static str = "id, code, notice, comments, ref_left, ref_right, qk_p_capacity, qk_qty, sk_unit, created_at, updated_at, deleted_at";
    const ENTITY_NAME: &'static str = "inventory_sales";
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = false;
}

impl HasReferenceJoins for InventorySales {
    fn reference_joins() -> Vec<ReferenceJoin> {
        vec![
            ReferenceJoin {
                name: "ref_left",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "ref_left",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_production""#,
                display_fields: &["notice", "code"],
            },
            ReferenceJoin {
                name: "ref_right",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "ref_right",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_stor-traffic_line""#,
                display_fields: &["notice", "code"],
            },
            ReferenceJoin {
                name: "qk_p_capacity",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "qk_p_capacity",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_scal-common""#,
                display_fields: &["notice", "mark"],
            },
            ReferenceJoin {
                name: "qk_qty",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "qk_qty",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_scal-common""#,
                display_fields: &["notice", "mark"],
            },
            ReferenceJoin {
                name: "sk_unit",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "sk_unit",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_unit""#,
                display_fields: &["notice"],
            },
        ]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateInventorySalesRequest {
    pub code: Option<String>,
    pub notice: Option<String>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub ref_left: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub ref_right: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_p_capacity: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_qty: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub sk_unit: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateInventorySalesRequest {
    pub code: Option<String>,
    pub notice: Option<String>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub ref_left: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub ref_right: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_p_capacity: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_qty: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub sk_unit: Option<i64>,
}

// ═══════════════════════════════════════════════
// PricingAgreement — zc_id_agre-pricing
// ═══════════════════════════════════════════════

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct PricingAgreement {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub code: Option<String>,
    pub notice: Option<String>,
    pub comments: Option<String>,
    pub t_color_: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub tpl_id: Option<i64>,
    #[sqlx(default)]
    pub _refs: Option<Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl Identifiable for PricingAgreement {
    fn id(&self) -> i64 {
        self.id
    }
}
impl AliothDbEntity for PricingAgreement {
    fn table_name() -> &'static str {
        "\"isahl\".\"zc_id_agre-pricing\""
    }
    const SELECT_FIELDS: &'static str =
        "id, code, notice, comments, t_color_, tpl_id, created_at, updated_at, deleted_at";
    const ENTITY_NAME: &'static str = "pricing_agreement";
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = false;
}
impl HasReferenceJoins for PricingAgreement {
    fn reference_joins() -> Vec<ReferenceJoin> {
        vec![]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatePricingAgreementRequest {
    pub code: Option<String>,
    pub notice: Option<String>,
    pub comments: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdatePricingAgreementRequest {
    pub code: Option<String>,
    pub notice: Option<String>,
    pub comments: Option<String>,
}

// ═══════════════════════════════════════════════
// Contract — zc_id_contract
// ═══════════════════════════════════════════════

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Contract {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    #[serde(rename = "name")]
    pub notice: Option<String>,
    pub code: Option<String>,
    pub o_number: Option<String>,
    pub comments: Option<String>,
    pub projection: Option<String>,
    pub t_color_: Option<String>,
    #[serde(skip)]
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_date: Option<i64>,
    #[sqlx(rename = "qk_valid-segm")]
    #[serde(skip)]
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_valid_segm: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub tpl_id: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub dk_scene: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub dk_factor: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub dk_function: Option<i64>,
    #[serde(rename = "createdAt")]
    pub created_at: DateTime<Utc>,
    #[serde(rename = "updatedAt")]
    pub updated_at: Option<DateTime<Utc>>,
    #[serde(skip)]
    pub deleted_at: Option<DateTime<Utc>>,
    #[sqlx(default)]
    pub _refs: Option<Value>,
    // ── Computed fields（非 DB 列，repository 填充）──
    #[sqlx(default)]
    pub amount: Option<f64>,
    #[sqlx(default)]
    #[serde(rename = "partyA")]
    pub party_a: Option<String>,
    #[sqlx(default)]
    #[serde(rename = "partyB")]
    pub party_b: Option<String>,
    /// 状态 code（draft/pending_approval/active/executing/expired/pending_renewal/renewed/not_renew）
    #[sqlx(default)]
    pub status: Option<String>,
    /// 续约状态 code（pending_renewal/renewed/not_renew；r_status 桥 code='renewal_status'，
    /// 无桥接行回退 status 的续约态值——批注 ae714531 双状态分离）
    #[sqlx(default)]
    #[serde(rename = "renewalStatus")]
    pub renewal_status: Option<String>,
    /// 合同类型（transport-sales / transport-procurement），由 TABLEOID 叶表派生
    #[sqlx(default)]
    #[serde(rename = "type")]
    pub contract_type: Option<String>,
    /// 合同性质（T04：normal/master/supplement/single，从 comments JSON 解析）
    #[sqlx(default)]
    pub kind: Option<String>,
    #[sqlx(default)]
    #[serde(rename = "signDate")]
    pub sign_date: Option<String>,
    #[sqlx(default)]
    #[serde(rename = "effectiveDate")]
    pub effective_date: Option<String>,
    #[sqlx(default)]
    #[serde(rename = "expiryDate")]
    pub expiry_date: Option<String>,
}

impl Identifiable for Contract {
    fn id(&self) -> i64 {
        self.id
    }
}
impl AliothDbEntity for Contract {
    fn table_name() -> &'static str {
        "\"isahl\".\"zc_id_contract\""
    }
    const SELECT_FIELDS: &'static str =
        "id, notice, code, o_number, comments, projection, t_color_, qk_date, \"qk_valid-segm\", tpl_id, \
         dk_scene, dk_factor, dk_function, created_at, updated_at, deleted_at";
    const ENTITY_NAME: &'static str = "contract";
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = false;
}
impl HasReferenceJoins for Contract {
    fn reference_joins() -> Vec<ReferenceJoin> {
        vec![ReferenceJoin {
            name: "qk_date",
            card: Card::ToOne,
            kind: JoinKind::Forward {
                local_fk: "qk_date",
                target_key: "id",
            },
            target_table: r#"isahl."zc_id_scal-date""#,
            display_fields: &["date", "notice"],
        }]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateContractRequest {
    pub code: Option<String>,
    pub notice: Option<String>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub sign_date: Option<i64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateContractRequest {
    pub code: Option<String>,
    pub notice: Option<String>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub sign_date: Option<i64>,
}

// ═══════════════════════════════════════════════════════════════════════════════
// 主体域叶表（strengthen-identity-org：subj-group/employee/empl-agent/country/bank/
// ministry/sovereign/supranational）——同构基础列集（code/notice/o_number/comments），
// 经宏生成消除重复样板。
// ═══════════════════════════════════════════════════════════════════════════════

/// 生成主体域叶表实体：Model + Identifiable + AliothDbEntity + HasReferenceJoins(空)
/// + Create/Update 请求（基础列集 code/notice/o_number/comments）。
macro_rules! subject_leaf_models {
    ($entity:ident, $create:ident, $update:ident, $table:literal, $name:literal) => {
        #[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
        pub struct $entity {
            #[serde(with = "common::serde_zuid")]
            pub id: i64,
            pub code: Option<String>,
            pub notice: Option<String>,
            pub o_number: Option<String>,
            pub comments: Option<String>,
            #[sqlx(default)]
            #[serde(rename = "_refs")]
            pub _refs: Option<Value>,
            pub created_at: DateTime<Utc>,
            pub updated_at: Option<DateTime<Utc>>,
            pub deleted_at: Option<DateTime<Utc>>,
        }

        impl Identifiable for $entity {
            fn id(&self) -> i64 {
                self.id
            }
        }
        impl AliothDbEntity for $entity {
            fn table_name() -> &'static str {
                $table
            }
            const SELECT_FIELDS: &'static str =
                "id, code, notice, o_number, comments, created_at, updated_at, deleted_at";
            const ENTITY_NAME: &'static str = $name;
            const SOFT_DELETE: bool = true;
            const HAS_AUDIT: bool = false;
        }
        impl HasReferenceJoins for $entity {
            fn reference_joins() -> Vec<ReferenceJoin> {
                vec![]
            }
        }

        #[derive(Debug, Clone, Serialize, Deserialize)]
        pub struct $create {
            pub code: Option<String>,
            pub notice: Option<String>,
            pub o_number: Option<String>,
            pub comments: Option<String>,
        }
        #[derive(Debug, Clone, Serialize, Deserialize)]
        pub struct $update {
            pub code: Option<String>,
            pub notice: Option<String>,
            pub o_number: Option<String>,
            pub comments: Option<String>,
        }
    };
}

subject_leaf_models!(
    SubjectGroup,
    CreateSubjectGroupRequest,
    UpdateSubjectGroupRequest,
    "\"isahl\".\"zc_id_subj-group\"",
    "subject-group"
);
subject_leaf_models!(
    SubjectEmployee,
    CreateSubjectEmployeeRequest,
    UpdateSubjectEmployeeRequest,
    "\"isahl\".\"zc_id_subj-employee\"",
    "subject-employee"
);
subject_leaf_models!(
    EmploymentAgent,
    CreateEmploymentAgentRequest,
    UpdateEmploymentAgentRequest,
    "\"isahl\".\"zc_id_empl-agent\"",
    "empl-agent"
);
subject_leaf_models!(
    SubjectCountry,
    CreateSubjectCountryRequest,
    UpdateSubjectCountryRequest,
    "\"isahl\".\"zc_id_subj-country\"",
    "subject-country"
);
subject_leaf_models!(
    SubjectBank,
    CreateSubjectBankRequest,
    UpdateSubjectBankRequest,
    "\"isahl\".\"zc_id_subj-bank\"",
    "subject-bank"
);
subject_leaf_models!(
    SubjectMinistry,
    CreateSubjectMinistryRequest,
    UpdateSubjectMinistryRequest,
    "\"isahl\".\"zc_id_subj-ministry\"",
    "subject-ministry"
);
subject_leaf_models!(
    SubjectSovereign,
    CreateSubjectSovereignRequest,
    UpdateSubjectSovereignRequest,
    "\"isahl\".\"zc_id_subj-sovereign\"",
    "subject-sovereign"
);
subject_leaf_models!(
    SubjectSupranational,
    CreateSubjectSupranationalRequest,
    UpdateSubjectSupranationalRequest,
    "\"isahl\".\"zc_id_subj-supranational\"",
    "subject-supranational"
);
