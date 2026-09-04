//! 运行环境模型 — L2 DTO 层

use chrono::{DateTime, Utc};
use crud::{AliothDbEntity, Identifiable};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Environment {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub name: String,
    pub host: Option<String>,
    pub os: Option<String>,
    pub runtime: Option<String>,
    #[serde(rename = "type")]
    pub type_: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub status: Option<i64>,
    pub services: Option<i32>,
    pub uptime: Option<String>,
    pub comments: Option<String>,
    pub settings: Option<serde_json::Value>,
    #[sqlx(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub _refs: Option<serde_json::Value>,
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
        r#"isahl."zc_id_prot-env_config""#
    }
    const SELECT_FIELDS: &'static str = "id, notice AS name, code AS host, \
         NULL::bigint AS status, NULL::text AS type_, \
         NULL::text AS runtime, NULL::text AS os, \
         (settings->>'services')::int AS services, \
         settings->>'uptime' AS uptime, comments, \
         settings, created_at, updated_at, deleted_at";
    const ENTITY_NAME: &'static str = "environment";
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = false;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateEnvironmentRequest {
    pub name: String,
    pub host: Option<String>,
    pub os: Option<String>,
    pub runtime: Option<String>,
    #[serde(rename = "type")]
    pub type_: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub status: Option<i64>,
    pub services: Option<i32>,
    pub uptime: Option<String>,
    pub comments: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateEnvironmentRequest {
    pub name: Option<String>,
    pub host: Option<String>,
    pub os: Option<String>,
    pub runtime: Option<String>,
    #[serde(rename = "type")]
    pub type_: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub status: Option<i64>,
    pub services: Option<i32>,
    pub uptime: Option<String>,
    pub comments: Option<String>,
}
