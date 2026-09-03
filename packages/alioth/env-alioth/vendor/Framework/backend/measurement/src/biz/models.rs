//! 计量业务形状模型（biz 面）——consolidate-duplicated-services 共享内核
//!
//! 从 WZ/Alioth measurement 生产实现 1:1 提取（2026-08-19 A′ 案裁定），
//! 作为 ns 壳的唯一实现来源。物理面（rate/scale/unit 等 z_id 全量模型）保留
//! 供 AVIC 域未来使用；本模块是业务 DTO 形状（L2 语义命名，L3 前端 1:1 透传）。
//!
//! id 生成：INSERT 省略 id 列，依赖列默认 `gen_next_uid(table_code)`
//! （实测：zc_id_unit=14 / zc_id_rate=291 / zc_id_rate-exchange=384 / zc_id_scal-price=415）。
//! MUST NOT 使用 `gen_next_zuid()`（[NEVER] 仅限 isahl_auth/isahl_audit/zc_id_lifecycle）。

use chrono::{DateTime, Utc};
use crud::entity::{AliothDbEntity, Identifiable};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

// ── 计量单位 MeasurementUnit ──────────────────────────────────────────────────

/// 测量单位（米、千克、秒…），经 `zc_id_unit` 存储（INSERT 按量纲路由到 `zc_id_unit-*` 叶表）
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct MeasurementUnit {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub name: String,
    pub code: Option<String>,
    pub symbol: Option<String>,
    pub system: Option<String>,
    pub dimension: Option<String>,
    pub base: Option<bool>,
    pub t_color_: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl Identifiable for MeasurementUnit {
    fn id(&self) -> i64 {
        self.id
    }
}

impl AliothDbEntity for MeasurementUnit {
    fn table_name() -> &'static str {
        "isahl.zc_id_unit"
    }
    const SELECT_FIELDS: &'static str = "id, notice AS name, code, symbol, \
         CASE WHEN tableoid = 'isahl.zc_id_unit'::regclass THEN NULL \
              ELSE replace(replace(tableoid::regclass::text, '\"zc_id_unit-', ''), '\"', '') END AS dimension, \
         system::text AS system, base, t_color_, created_at, updated_at, deleted_at";
    const ENTITY_NAME: &'static str = "measurement_unit";
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = false;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateMeasurementUnitRequest {
    pub name: String,
    pub code: Option<String>,
    pub symbol: Option<String>,
    pub system: Option<String>,
    pub dimension_key: Option<String>,
    pub base: Option<bool>,
    /// 展示色（t_color_ 列，DB 业务字段）；ns 壳可先行补色（dimension_color 属展示策略，留壳）
    pub t_color_: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateMeasurementUnitRequest {
    pub name: Option<String>,
    pub code: Option<String>,
    pub symbol: Option<String>,
    pub system: Option<String>,
    pub base: Option<bool>,
}

// ── 汇率 ExchangeRate ────────────────────────────────────────────────────────

/// 货币汇率配对，经 `zc_id_rate-exchange` 存储
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct ExchangeRate {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub name: String,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub left_currency: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub right_currency: Option<i64>,
    pub rate: Option<Decimal>,
    /// 卖价（division 列）——WZ 契约不消费，Alioth 投影为 ask_price
    pub ask_price: Option<Decimal>,
    pub source: Option<String>,
    pub date: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl Identifiable for ExchangeRate {
    fn id(&self) -> i64 {
        self.id
    }
}

impl AliothDbEntity for ExchangeRate {
    fn table_name() -> &'static str {
        "isahl.\"zc_id_rate-exchange\""
    }
    const SELECT_FIELDS: &'static str =
        "id, notice AS name, ck_left AS left_currency, ck_right AS right_currency, multiply AS rate, division AS ask_price, code AS source, date, created_at, updated_at, deleted_at";
    const ENTITY_NAME: &'static str = "exchange_rate";
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = false;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateExchangeRateRequest {
    pub name: String,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub left_currency: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub right_currency: Option<i64>,
    pub rate: Option<Decimal>,
    pub ask_price: Option<Decimal>,
    pub source: Option<String>,
    pub date: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateExchangeRateRequest {
    pub name: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub left_currency: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub right_currency: Option<i64>,
    pub rate: Option<Decimal>,
    pub ask_price: Option<Decimal>,
    pub source: Option<String>,
    pub date: Option<DateTime<Utc>>,
}

// ── 标量值 ScalarPrice ──────────────────────────────────────────────────────

/// 标量价格/数值实体，经 `zc_id_scal-price`（`zc_id_scale` 子表）存储
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct ScalarPrice {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub name: String,
    pub code: Option<String>,
    pub comments: Option<String>,
    pub value: Option<Decimal>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub unit: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub precision_: Option<i64>,
    pub retain_signal: Option<bool>,
    pub t_color_: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub ref_count: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl Identifiable for ScalarPrice {
    fn id(&self) -> i64 {
        self.id
    }
}

impl AliothDbEntity for ScalarPrice {
    fn table_name() -> &'static str {
        "isahl.\"zc_id_scal-price\""
    }
    const SELECT_FIELDS: &'static str = "id, notice AS name, code, comments, mark AS value, \
         sk_unit AS unit, precision_, retain_signal, t_color_, ref_count, \
         created_at, updated_at, deleted_at";
    const ENTITY_NAME: &'static str = "scalar_price";
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = false;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateScalarPriceRequest {
    pub name: String,
    pub code: Option<String>,
    pub comments: Option<String>,
    pub value: Option<Decimal>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub unit: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub precision_: Option<i64>,
    pub retain_signal: Option<bool>,
    pub t_color_: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub ref_count: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateScalarPriceRequest {
    pub name: Option<String>,
    pub code: Option<String>,
    pub comments: Option<String>,
    pub value: Option<Decimal>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub unit: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub precision_: Option<i64>,
    pub retain_signal: Option<bool>,
    pub t_color_: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub ref_count: Option<i64>,
}

// ── 单位换算率 UnitConversionRate ─────────────────────────────────────────────

/// 同量纲单位间换算率，经 `zc_id_rate-*` 叶表存储（INSERT 按量纲路由）
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct UnitConversionRate {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub name: String,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub left: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub right: Option<i64>,
    pub multiply: Option<Decimal>,
    pub division: Option<Decimal>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub precision_: Option<i64>,
    pub intrinsic: Option<bool>,
    pub dimension: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl Identifiable for UnitConversionRate {
    fn id(&self) -> i64 {
        self.id
    }
}

impl AliothDbEntity for UnitConversionRate {
    fn table_name() -> &'static str {
        "isahl.zc_id_rate"
    }
    const SELECT_FIELDS: &'static str = "id, notice AS name, ck_left AS left, ck_right AS right, \
         multiply, division, precision_, intrinsic, \
         CASE WHEN tableoid = 'isahl.zc_id_rate'::regclass THEN NULL \
              ELSE replace(replace(tableoid::regclass::text, '\"zc_id_rate-', ''), '\"', '') END AS dimension, \
         created_at, updated_at, deleted_at";
    const ENTITY_NAME: &'static str = "unit_conversion_rate";
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = false;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateUnitConversionRateRequest {
    pub name: String,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub left: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub right: Option<i64>,
    pub multiply: Option<Decimal>,
    pub division: Option<Decimal>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub precision_: Option<i64>,
    pub intrinsic: Option<bool>,
    pub dimension_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateUnitConversionRateRequest {
    pub name: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub left: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub right: Option<i64>,
    pub multiply: Option<Decimal>,
    pub division: Option<Decimal>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub precision_: Option<i64>,
    pub intrinsic: Option<bool>,
}
