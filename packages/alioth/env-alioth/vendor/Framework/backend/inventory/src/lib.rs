//! # inventory — 通用库存统计服务
//!
//! 语义（用户裁定）：**库存 = 货（material）在储元（place）中的时空切片数量统计**——
//! 「货」「储元」语义同构通用，与具体 namespace 业务解耦：
//!
//! - **货**：被统计的实体（物料/产品/资产……），由 namespace 壳注入 refs 解析。
//! - **储元**：空间切片（库位/容器/仓位……），由 namespace 壳注入 refs 解析。
//! - **时空切片**：物化视图 `isahl.mv_inventory` 的 REFRESH 时点快照（时间维）
//!   × 储元（`fk_place` 空间维）——净数量 = 入库明细 SUM − 出库明细 SUM。
//!
//! ## 分层
//!
//! - 本 crate：通用查询抽象（mv_inventory 分页/过滤/排序 + 时空切片语义），
//!   响应/分页复用 `common::data::{ApiResponse, ListQuery, PaginatedResponse}`。
//! - namespace 壳（`Pre-Proc/{ns}/Sources/Services/inventory-*/backend`）：
//!   注册 `/service/{id}` 路由前缀 + 注入**货/储元名称解析**（`NameResolver` trait——
//!   目标表因 ns 而异，Alioth 的物料表 ≠ WZ 的物料表）。
//! - 前端 API 适配层：各 namespace 模块 `frontend/src/api/` 自持。

pub mod models;
pub mod routes;
pub mod service;

pub use models::{InventoryBalanceSummary, RefKind, RefNames};
pub use routes::configure_routes;
