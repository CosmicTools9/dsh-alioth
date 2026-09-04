//! 库存统计 Service 实体模型 — L2 DTO 层
//!
//! 映射（重制后，ADR D-018）：
//! - `StockStat` → `isahl.zc_id_production_rr_storage`（只读统计，JOIN 标量 mark）
//! - `Voucher` → `isahl.zc_id_stat-sto-voucher`
//! - `Counting` → `isahl.zc_id_even-counting`
//! - `StorageNest` → `isahl.zc_id_storage_rr_stock-in`（行存在即置入，无 direction）

use chrono::{DateTime, Utc};
use crud::entity::{AliothDbEntity, Identifiable};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

// ── StockStat：只读库存统计（物化储量）────────────────────────────

/// 库存统计行：产品↔储元 关系的物化储量（读侧解析标量）
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct StockStat {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    /// 产品（ref_left）
    #[serde(with = "common::serde_zuid")]
    pub production_id: i64,
    /// 储元（ref_right）
    #[serde(with = "common::serde_zuid")]
    pub storage_id: i64,
    /// 储量/库存（qk_qty → zc_id_scal-common.mark 解析真值，触发器物化维护）
    pub qty: Decimal,
    /// 单位（sk_unit）
    #[serde(with = "common::serde_zuid::opt", default)]
    pub unit: Option<i64>,
    /// 最近盘点实盘截止值（deta-counting.qk_qty 标量真值；无盘点 = NULL）
    pub last_counted_qty: Option<Decimal>,
    /// 盘盈盘亏 = 实盘 − 物化（无盘点 = NULL；负值=盘亏）
    pub variance: Option<Decimal>,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
}

impl Identifiable for StockStat {
    fn id(&self) -> i64 {
        self.id
    }
}

impl AliothDbEntity for StockStat {
    fn table_name() -> &'static str {
        r#"isahl."zc_id_production_rr_storage""#
    }

    const SELECT_FIELDS: &'static str = "id, ref_left, ref_right, created_at, updated_at";
    const ENTITY_NAME: &'static str = "stock_stat";
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = false;
}

// ── Voucher：货 stock in/out 储元 ─────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Voucher {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    /// 货（fk_production）
    #[serde(with = "common::serde_zuid::opt", default)]
    pub production_id: Option<i64>,
    /// 出库位（fk_subj-storage）
    #[serde(with = "common::serde_zuid::opt", default)]
    pub from_storage_id: Option<i64>,
    /// 入库位（fk_obj-storage）
    #[serde(with = "common::serde_zuid::opt", default)]
    pub to_storage_id: Option<i64>,
    /// 数量（qk_qty）
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qty: Option<i64>,
    /// 流入（qk_income）
    #[serde(with = "common::serde_zuid::opt", default)]
    pub income: Option<i64>,
    /// 流出（qk_outgo）
    #[serde(with = "common::serde_zuid::opt", default)]
    pub outgo: Option<i64>,
    /// 期初余额（qk_pre_balance，事实发生前）
    #[serde(with = "common::serde_zuid::opt", default)]
    pub pre_balance: Option<i64>,
    /// 期末余额（qk_balance，事实发生后）
    #[serde(with = "common::serde_zuid::opt", default)]
    pub balance: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl Identifiable for Voucher {
    fn id(&self) -> i64 {
        self.id
    }
}

impl AliothDbEntity for Voucher {
    fn table_name() -> &'static str {
        r#"isahl."zc_id_stat-sto-voucher""#
    }

    const SELECT_FIELDS: &'static str = concat!(
        r#"id, fk_production AS production_id, "fk_subj-storage" AS from_storage_id, "#,
        r#""fk_obj-storage" AS to_storage_id, qk_qty AS qty, qk_income AS income, "#,
        r#"qk_outgo AS outgo, qk_pre_balance AS pre_balance, qk_balance AS balance, "#,
        r#"created_at, updated_at, deleted_at"#
    );
    const ENTITY_NAME: &'static str = "voucher";
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = false;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateVoucherRequest {
    #[serde(with = "common::serde_zuid::opt", default)]
    pub production_id: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub from_storage_id: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub to_storage_id: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qty: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub income: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub outgo: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateVoucherRequest {
    #[serde(with = "common::serde_zuid::opt", default)]
    pub production_id: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub from_storage_id: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub to_storage_id: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qty: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub income: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub outgo: Option<i64>,
}

// ── Counting：盘点（事件-盘点）──────────────────────────────────────

/// 盘点范围项（m2n → `zc_id_event_rr_matter`，ref_right=产品）：产品 _refs 对象
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatterRef {
    /// 产品 ID（ref_right → zc_id_production）
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    /// 产品 notice（读侧 JOIN 解析）
    pub notice: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Counting {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    /// 盘点储位（fk_place，储元引用）
    #[serde(with = "common::serde_zuid::opt", default)]
    pub place_id: Option<i64>,
    /// 盘点日期（qk_date）
    #[serde(with = "common::serde_zuid::opt", default)]
    pub counted_date: Option<i64>,
    /// 抽象层级（`_t_`，后端按范例/实例规则注入：'范例' / '实例'）
    #[sqlx(rename = "_t_")]
    #[serde(rename = "_t_")]
    pub t: Option<String>,
    /// 范例引用（tpl_id → 范例盘点行 `_t_='范例'`；实例创建时携带）
    #[serde(with = "common::serde_zuid::opt", default)]
    pub tpl_id: Option<i64>,
    /// 盘点范围（m2n → `zc_id_event_rr_matter`，ref_left=事件 / ref_right=产品；读取时由 repository 填充）
    #[sqlx(skip)]
    pub matters: Vec<MatterRef>,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl Identifiable for Counting {
    fn id(&self) -> i64 {
        self.id
    }
}

impl AliothDbEntity for Counting {
    fn table_name() -> &'static str {
        r#"isahl."zc_id_even-counting""#
    }

    const SELECT_FIELDS: &'static str = concat!(
        "id, fk_place AS place_id, ",
        "qk_date AS counted_date, ",
        r#""_t_", tpl_id, "#,
        "created_at, updated_at, deleted_at"
    );
    const ENTITY_NAME: &'static str = "counting";
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = false;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateCountingRequest {
    #[serde(with = "common::serde_zuid::opt", default)]
    pub place_id: Option<i64>,
    /// 盘点日期（实际值 "YYYY-MM-DD"，service 转 zc_id_scal-date 标量 ID）
    pub counted_date: Option<String>,
    /// 范例引用（可选）：实例创建时指向范例盘点行；后端据此注入 `_t_`（有 tpl_id → '实例'，缺省 → '范例'）
    #[serde(with = "common::serde_zuid::opt", default)]
    pub tpl_id: Option<i64>,
    /// 盘点范围（产品 ID 列表 → `zc_id_event_rr_matter`，创建时同事务写入）
    #[serde(default)]
    #[serde(with = "common::serde_zuid::seq")]
    pub matters: Vec<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateCountingRequest {
    #[serde(with = "common::serde_zuid::opt", default)]
    pub place_id: Option<i64>,
    pub counted_date: Option<String>,
    /// 范例引用（可选）：更新 tpl_id 时后端同步重注入 `_t_`
    #[serde(with = "common::serde_zuid::opt", default)]
    pub tpl_id: Option<i64>,
}

// ── CountingDetail：盘点明细（实盘数 = 统计库存后的截止值载体）───

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct CountingDetail {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    /// 所属盘点事件（fk_list → zc_id_even-counting）
    #[serde(with = "common::serde_zuid::opt", default)]
    pub counting_id: Option<i64>,
    /// 盘点的货（fk_production，储位上的存货）
    #[serde(with = "common::serde_zuid::opt", default)]
    pub production_id: Option<i64>,
    /// 储元（fk_storage；与事件头 fk_storage 可同可异，异则经 zc_id_storage_rr_stock-in 嵌套）
    #[serde(with = "common::serde_zuid::opt", default)]
    pub storage_id: Option<i64>,
    /// 触发盘点的交易凭证（fk_voucher → zc_id_stat-sto-voucher：储位完成该笔交易后盘点）
    #[serde(with = "common::serde_zuid::opt", default)]
    pub voucher_id: Option<i64>,
    /// 实盘数量（qk_qty → zc_id_scal-common）
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qty: Option<i64>,
    /// 实盘重量（qk_w_qty → zc_id_scal-weight）
    #[serde(with = "common::serde_zuid::opt", default)]
    pub w_qty: Option<i64>,
    /// 实盘体积（qk_v_qty → zc_id_scal-volume）
    #[serde(with = "common::serde_zuid::opt", default)]
    pub v_qty: Option<i64>,
    /// 盘点日期（qk_date → zc_id_scal-date）
    #[serde(with = "common::serde_zuid::opt", default)]
    pub counted_date: Option<i64>,
    /// 经办人（fk_biller → zc_id_subjects）
    #[serde(with = "common::serde_zuid::opt", default)]
    pub biller_id: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl Identifiable for CountingDetail {
    fn id(&self) -> i64 {
        self.id
    }
}

impl AliothDbEntity for CountingDetail {
    fn table_name() -> &'static str {
        r#"isahl."zc_id_deta-counting""#
    }

    const SELECT_FIELDS: &'static str = concat!(
        "id, fk_list AS counting_id, fk_production AS production_id, ",
        "fk_storage AS storage_id, fk_voucher AS voucher_id, ",
        "qk_qty AS qty, qk_w_qty AS w_qty, qk_v_qty AS v_qty, ",
        "qk_date AS counted_date, fk_biller AS biller_id, ",
        "created_at, updated_at, deleted_at"
    );
    const ENTITY_NAME: &'static str = "counting_detail";
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = false;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateCountingDetailRequest {
    #[serde(with = "common::serde_zuid::opt", default)]
    pub counting_id: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub production_id: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub storage_id: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub voucher_id: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qty: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub w_qty: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub v_qty: Option<i64>,
    /// 盘点日期（实际值 "YYYY-MM-DD"，service 转 zc_id_scal-date 标量 ID）
    pub counted_date: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub biller_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateCountingDetailRequest {
    #[serde(with = "common::serde_zuid::opt", default)]
    pub counting_id: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub production_id: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub storage_id: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub voucher_id: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qty: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub w_qty: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub v_qty: Option<i64>,
    pub counted_date: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub biller_id: Option<i64>,
}

// ── StorageNest：储元⇲储元 时空嵌套（行存在即置入，无 direction）──

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct StorageNest {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    /// 父容器（ref_left）
    #[serde(with = "common::serde_zuid::opt", default)]
    pub parent_id: Option<i64>,
    /// 子储元（ref_right）
    #[serde(with = "common::serde_zuid::opt", default)]
    pub child_id: Option<i64>,
    /// 置入期（qk_period → zc_id_segm-date）
    #[serde(with = "common::serde_zuid::opt", default)]
    pub period_id: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl Identifiable for StorageNest {
    fn id(&self) -> i64 {
        self.id
    }
}

impl AliothDbEntity for StorageNest {
    fn table_name() -> &'static str {
        r#"isahl."zc_id_storage_rr_stock-in""#
    }

    const SELECT_FIELDS: &'static str = concat!(
        "id, ref_left AS parent_id, ref_right AS child_id, ",
        "qk_period AS period_id, created_at, updated_at, deleted_at"
    );
    const ENTITY_NAME: &'static str = "storage_nest";
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = false;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateStorageNestRequest {
    #[serde(with = "common::serde_zuid::opt", default)]
    pub parent_id: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub child_id: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub period_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateStorageNestRequest {
    #[serde(with = "common::serde_zuid::opt", default)]
    pub parent_id: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub child_id: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub period_id: Option<i64>,
}
