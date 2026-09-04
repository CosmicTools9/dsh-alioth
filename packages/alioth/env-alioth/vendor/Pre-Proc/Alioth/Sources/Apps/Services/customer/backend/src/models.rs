//! customer 实体模型 — crud 模式（AliothDbEntity + HasReferenceJoins）。
//!
//! 自动生成（gen-service-backend.ts）；语义重命名与 _refs joins 按需后续补充。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crud::{AliothDbEntity, HasReferenceJoins, Identifiable, ReferenceJoin};

// ── Customer → isahl."zc_id_cate-organization" ──

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Customer {
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
    #[serde(with = "common::serde_zuid::opt", default)]
    pub c_sort_: Option<i64>,
    /// 关联引用嵌入数据（_refs 名称解析；joins 待补）
    #[sqlx(default)]
    #[serde(rename = "_refs")]
    pub _refs: Option<serde_json::Value>,
}

impl Identifiable for Customer {
    fn id(&self) -> i64 {
        self.id
    }
}

impl AliothDbEntity for Customer {
    fn table_name() -> &'static str {
        r#"isahl."zc_id_cate-organization""#
    }

    const SELECT_FIELDS: &'static str = r#"created_at, updated_at, id, created_by_id, updated_by_id, notice, t_color_, deleted_at, deleted_by_id, code, o_number, comments, ak_benefit_user, ak_permit_user, ak_access_user, enable, c_sort_"#;
    const ENTITY_NAME: &'static str = "customer";
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = false;
}

impl HasReferenceJoins for Customer {
    fn reference_joins() -> Vec<ReferenceJoin> {
        vec![]
    }
}

/// 创建输入（可写集 = 全列 − 系统列）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CreateCustomerRequest {
    pub notice: Option<String>,
    pub code: Option<String>,
    pub comments: Option<String>,
    pub enable: Option<bool>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub c_sort_: Option<i64>,
}

/// 更新输入（全 Optional，同可写集）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateCustomerRequest {
    pub notice: Option<String>,
    pub code: Option<String>,
    pub comments: Option<String>,
    pub enable: Option<bool>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub c_sort_: Option<i64>,
}
