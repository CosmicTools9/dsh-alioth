//! 库存统计业务 DTO（用户裁定：DTO 按业务场景编写——库存 = 货在储元中的时空切片数量统计）
//!
//! 数据源：`isahl.mv_inventory`（物化视图——`zc_id_production_rr_storage` 容量行：
//! 货（production）× 储元（storage）的数量/容量切片，qty/capacity 经标量表解析）。

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 引用种类（货 / 储元）——名称解析目标表因 namespace 而异
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RefKind {
    /// 货（production/物料实体）
    Material,
    /// 储元（storage/库位容器实体）
    Place,
}

/// 库存余额统计行（业务场景 DTO）
///
/// - `id`：容量行 ID（mv_inventory.id）
/// - `production_id` / `storage_id`：货与储元实体 ID
/// - `production_name` / `storage_name`：经 namespace 注入的 NameResolver 解析（可空）
/// - `qty`：数量（时空切片时点在库数量，标量表解析）
/// - `capacity`：容量
/// - `unit`：单位（标量引用 ID）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InventoryBalanceSummary {
    pub id: i64,
    pub production_id: i64,
    pub production_name: Option<String>,
    pub storage_id: i64,
    pub storage_name: Option<String>,
    pub qty: Decimal,
    pub capacity: Decimal,
    pub unit: Option<i64>,
    pub refreshed_at: Option<DateTime<Utc>>,
}

/// 库存余额列表查询（业务场景：分页 + 货/储元过滤）
#[derive(Debug, Clone, Deserialize)]
pub struct BalanceListQuery {
    #[serde(flatten)]
    pub base: crud::ListQuery,
    /// 货（production）过滤
    pub production_id: Option<i64>,
    /// 储元（storage）过滤
    pub storage_id: Option<i64>,
}

/// 名称解析结果（RefKind → id → 名称）
pub type RefNames = HashMap<RefKind, HashMap<i64, String>>;

/// 货/储元名称解析器——由 namespace 壳注入（目标表因 ns 而异）
///
/// 契约：对给定 kind 的 id 集合返回 id → 业务名（notice）；未知 id 可省略
/// （DTO 对应字段保持 None）。pool 由调用方（handler/service）提供。
#[async_trait::async_trait]
pub trait NameResolver: Send + Sync {
    async fn resolve(&self, pool: &sqlx::PgPool, kind: RefKind, ids: &[i64]) -> RefNames;
}
