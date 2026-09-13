//! # waybill-writer — 派车/运单写链共享事务
//!
//! 单一事实源：`transport-dispatch::repositories::dispatch_core` 的
//! `dispatch_vehicles_tx_inner`（派车核心事务，约 1300 行）整体迁入，供两类调用方共用：
//!
//! - **Gateway WZ**（transport-dispatch service）：平台调度员派车 / 承运商开放 API 入向派车；
//! - **OpenActivity 门户**（portal_write.rs）：承运商在门户为本组织受托的委托自建运单。
//!
//! 约束：两应用禁止 HTTP 互调 → 写链以共享 crate 形式单份实现（禁止复制事务到任一应用）。
//!
//! ## 事务语义（与迁移前逐表一致）
//!
//! 接受 `&mut sqlx::Transaction<'_, Postgres>`（调用方管理 commit/rollback），
//! 每次提交全部车辆：运单主档（`zc_id_orde-land WB-*`）+ 一式两份镜像（`-R`）+
//! CSALE/REQ/DSP/PUR 产品实例对 + `deta-trade_order` 明细 + 分摊标量 +
//! 派车转换 tsp 凭证 + 履约库存实例 + 委托生命周期同码推进。
//! 坐标固定（"GC"/"FJA"/"↓_BE"、"TX"/"FJA"/"↓_EV"），禁运行时推导。

pub mod bom;
pub mod dispatch;
pub mod models;
pub mod ontology;

pub use dispatch::{dispatch_vehicles_tx_inner, resolve_consignment_operator_org};
pub use models::{DispatchParams, VehicleAllocation};
