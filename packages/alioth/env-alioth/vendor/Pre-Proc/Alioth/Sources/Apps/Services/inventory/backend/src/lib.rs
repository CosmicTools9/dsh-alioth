//! # alioth-service-inventory — 库存统计
//!
//! Alioth 模型库存概念重制（ADR D-018）后的 Service 层：
//!
//! - **库存统计读取**：`StockStat`（`zc_id_production_rr_storage` 产品↔储元关系）——
//!   储量（库存）为统计值，物化于 `qk_qty → zc_id_scal-common.mark`，由 isahl 触发器增量
//!   维护（voucher 净变 / 盘点校正 / 嵌套 rollup），本服务读侧 JOIN 标量表取真值（O(1)）
//! - **写入口**（触发器自动物化）：
//!   - `Voucher`（`zc_id_stat-sto-voucher` 货 stock in/out 储元）
//!   - `Counting`（`zc_id_even-counting` 盘点）
//!   - `StorageNest`（`zc_id_storage_rr_stock-in` 储元⇲储元 时空嵌套，行存在即置入）

pub mod handlers;
pub mod models;
pub mod repositories;
pub mod services;

use actix_web::web;

pub fn register_service_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(web::scope("/service/inventory").configure(handlers::inventory::register));
}
