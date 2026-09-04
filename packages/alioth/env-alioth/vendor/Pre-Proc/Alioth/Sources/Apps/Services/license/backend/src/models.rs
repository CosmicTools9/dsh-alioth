//! 许可证模型 — L2 DTO 层

use chrono::{DateTime, Utc};
use crud::{AliothDbEntity, Identifiable};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct License {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub name: String,
    pub key: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub vendor: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub kind: Option<i64>,
    #[serde(with = "rust_decimal::serde::float_option")]
    pub seats: Option<Decimal>,
    pub expires: Option<DateTime<Utc>>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub status: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub used: Option<i64>,
    #[sqlx(default)]
    #[serde(rename = "_refs")]
    pub _refs: Option<serde_json::Value>,
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
        r#"isahl."zc_id_prod-license-purchase""#
    }
    const SELECT_FIELDS: &'static str = r#"id, notice AS name, code AS key, "fk_subj-provider" AS vendor, ck_category AS kind, qk_capacity::numeric AS seats, qk_duration, created_at, updated_at, deleted_at"#;
    const ENTITY_NAME: &'static str = "license";
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = false;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateLicenseRequest {
    pub name: String,
    pub key: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub vendor: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub kind: Option<i64>,
    pub vendor_name: Option<String>,
    pub kind_name: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub seats: Option<i64>,
    pub expires: Option<DateTime<Utc>>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub status: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateLicenseRequest {
    pub name: Option<String>,
    pub key: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub vendor: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub kind: Option<i64>,
    pub vendor_name: Option<String>,
    pub kind_name: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub seats: Option<i64>,
    pub expires: Option<DateTime<Utc>>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub status: Option<i64>,
}
