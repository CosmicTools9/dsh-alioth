//! OpenAPI 数据服务产品实体模型 — L2 DTO 层
//!
//! 映射 isahl 中 OpenAPI 数据服务（产品）体系（openapi-external-access）：
//! - `isahl.zc_id_prot-openapi_config`  — 对接配置（含 settings/enc_fields jsonb）
//! - `isahl.zc_id_prod-openapi-sales`    — 数据服务产品（销售侧）
//! - `isahl.zc_id_prod-openapi-purchase` — 数据服务产品（采购侧）
//! - `isahl.zc_id_prod-openapi-made`     — 数据服务产品（制造侧）
//!
//! 定义与配置数据在 isahl 对应表管理（标准 CRUD + NGAC 授权 + 审计），
//! UI 位于 Gateway（跨 APP 通用能力）。

use chrono::{DateTime, Utc};
use crud::entity::{AliothDbEntity, Identifiable};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

// ── 1. 对接配置（isahl.zc_id_prot-openapi_config，继承 zc_id_protocol） ────────────

/// 对接配置 DTO
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct OpenApiConfig {
    pub id: i64,
    pub name: String, // notice AS name
    pub code: Option<String>,
    pub comments: Option<String>,
    pub settings: Option<serde_json::Value>,
    pub enc_fields: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl Identifiable for OpenApiConfig {
    fn id(&self) -> i64 {
        self.id
    }
}

impl AliothDbEntity for OpenApiConfig {
    fn table_name() -> &'static str {
        r#"isahl."zc_id_prot-openapi_config""#
    }

    const SELECT_FIELDS: &'static str =
        "id, notice AS name, code, comments, settings, enc_fields, created_at, updated_at, deleted_at";
    const ENTITY_NAME: &'static str = "config";
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = false;
}

/// 创建请求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateOpenApiConfigRequest {
    pub name: String,
    pub code: Option<String>,
    pub comments: Option<String>,
    pub settings: Option<serde_json::Value>,
    pub enc_fields: Option<serde_json::Value>,
}

/// 更新请求（PATCH 语义 — 全部可选）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateOpenApiConfigRequest {
    pub name: Option<String>,
    pub code: Option<String>,
    pub comments: Option<String>,
    pub settings: Option<serde_json::Value>,
    pub enc_fields: Option<serde_json::Value>,
}

// ── 2. 数据服务产品（销售侧） ───────────────────────────────────────────────

/// 销售侧数据服务产品 DTO
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct OpenApiSales {
    pub id: i64,
    pub name: String, // notice AS name
    pub code: Option<String>,
    pub comments: Option<String>,
    pub projection: Option<String>,
    pub tpl_id: Option<i64>,
    pub p_number: Option<String>,
    pub fk_subj_demand: Option<i64>,
    pub fk_subj_provider: Option<i64>,
    pub qk_price: Option<i64>,
    pub fk_process: Option<i64>,
    pub sk_currency: Option<i64>,
    pub qk_size: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl Identifiable for OpenApiSales {
    fn id(&self) -> i64 {
        self.id
    }
}

impl AliothDbEntity for OpenApiSales {
    fn table_name() -> &'static str {
        r#"isahl."zc_id_prod-openapi-sales""#
    }

    const SELECT_FIELDS: &'static str =
        "id, notice AS name, code, comments, projection, tpl_id, p_number, \
\"fk_subj-demand\" AS fk_subj_demand, \"fk_subj-provider\" AS fk_subj_provider, \
qk_price, fk_process, sk_currency, qk_size, created_at, updated_at, deleted_at";
    const ENTITY_NAME: &'static str = "sales";
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = false;
}

/// 创建请求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateOpenApiSalesRequest {
    pub name: String,
    pub code: Option<String>,
    pub comments: Option<String>,
    pub projection: Option<String>,
    pub tpl_id: Option<i64>,
    pub p_number: Option<String>,
    pub fk_subj_demand: Option<i64>,
    pub fk_subj_provider: Option<i64>,
    pub qk_price: Option<i64>,
    pub fk_process: Option<i64>,
    pub sk_currency: Option<i64>,
    pub qk_size: Option<i64>,
}

/// 更新请求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateOpenApiSalesRequest {
    pub name: Option<String>,
    pub code: Option<String>,
    pub comments: Option<String>,
    pub projection: Option<String>,
    pub tpl_id: Option<i64>,
    pub p_number: Option<String>,
    pub fk_subj_demand: Option<i64>,
    pub fk_subj_provider: Option<i64>,
    pub qk_price: Option<i64>,
    pub fk_process: Option<i64>,
    pub sk_currency: Option<i64>,
    pub qk_size: Option<i64>,
}

// ── 3. 数据服务产品（采购侧） ───────────────────────────────────────────────

/// 采购侧数据服务产品 DTO
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct OpenApiPurchase {
    pub id: i64,
    pub name: String, // notice AS name
    pub code: Option<String>,
    pub comments: Option<String>,
    pub projection: Option<String>,
    pub tpl_id: Option<i64>,
    pub p_number: Option<String>,
    pub fk_subj_demand: Option<i64>,
    pub fk_subj_provider: Option<i64>,
    pub qk_price: Option<i64>,
    pub fk_process: Option<i64>,
    pub sk_currency: Option<i64>,
    pub qk_size: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl Identifiable for OpenApiPurchase {
    fn id(&self) -> i64 {
        self.id
    }
}

impl AliothDbEntity for OpenApiPurchase {
    fn table_name() -> &'static str {
        r#"isahl."zc_id_prod-openapi-purchase""#
    }

    const SELECT_FIELDS: &'static str =
        "id, notice AS name, code, comments, projection, tpl_id, p_number, \
\"fk_subj-demand\" AS fk_subj_demand, \"fk_subj-provider\" AS fk_subj_provider, \
qk_price, fk_process, sk_currency, qk_size, created_at, updated_at, deleted_at";
    const ENTITY_NAME: &'static str = "purchase";
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = false;
}

/// 创建请求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateOpenApiPurchaseRequest {
    pub name: String,
    pub code: Option<String>,
    pub comments: Option<String>,
    pub projection: Option<String>,
    pub tpl_id: Option<i64>,
    pub p_number: Option<String>,
    pub fk_subj_demand: Option<i64>,
    pub fk_subj_provider: Option<i64>,
    pub qk_price: Option<i64>,
    pub fk_process: Option<i64>,
    pub sk_currency: Option<i64>,
    pub qk_size: Option<i64>,
}

/// 更新请求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateOpenApiPurchaseRequest {
    pub name: Option<String>,
    pub code: Option<String>,
    pub comments: Option<String>,
    pub projection: Option<String>,
    pub tpl_id: Option<i64>,
    pub p_number: Option<String>,
    pub fk_subj_demand: Option<i64>,
    pub fk_subj_provider: Option<i64>,
    pub qk_price: Option<i64>,
    pub fk_process: Option<i64>,
    pub sk_currency: Option<i64>,
    pub qk_size: Option<i64>,
}

// ── 4. 数据服务产品（制造侧） ───────────────────────────────────────────────

/// 制造侧数据服务产品 DTO
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct OpenApiMade {
    pub id: i64,
    pub name: String, // notice AS name
    pub code: Option<String>,
    pub comments: Option<String>,
    pub projection: Option<String>,
    pub tpl_id: Option<i64>,
    pub p_number: Option<String>,
    pub fk_subj_demand: Option<i64>,
    pub fk_subj_provider: Option<i64>,
    pub qk_price: Option<i64>,
    pub fk_process: Option<i64>,
    pub sk_currency: Option<i64>,
    pub qk_size: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl Identifiable for OpenApiMade {
    fn id(&self) -> i64 {
        self.id
    }
}

impl AliothDbEntity for OpenApiMade {
    fn table_name() -> &'static str {
        r#"isahl."zc_id_prod-openapi-made""#
    }

    const SELECT_FIELDS: &'static str =
        "id, notice AS name, code, comments, projection, tpl_id, p_number, \
\"fk_subj-demand\" AS fk_subj_demand, \"fk_subj-provider\" AS fk_subj_provider, \
qk_price, fk_process, sk_currency, qk_size, created_at, updated_at, deleted_at";
    const ENTITY_NAME: &'static str = "made";
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = false;
}

/// 创建请求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateOpenApiMadeRequest {
    pub name: String,
    pub code: Option<String>,
    pub comments: Option<String>,
    pub projection: Option<String>,
    pub tpl_id: Option<i64>,
    pub p_number: Option<String>,
    pub fk_subj_demand: Option<i64>,
    pub fk_subj_provider: Option<i64>,
    pub qk_price: Option<i64>,
    pub fk_process: Option<i64>,
    pub sk_currency: Option<i64>,
    pub qk_size: Option<i64>,
}

/// 更新请求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateOpenApiMadeRequest {
    pub name: Option<String>,
    pub code: Option<String>,
    pub comments: Option<String>,
    pub projection: Option<String>,
    pub tpl_id: Option<i64>,
    pub p_number: Option<String>,
    pub fk_subj_demand: Option<i64>,
    pub fk_subj_provider: Option<i64>,
    pub qk_price: Option<i64>,
    pub fk_process: Option<i64>,
    pub sk_currency: Option<i64>,
    pub qk_size: Option<i64>,
}
