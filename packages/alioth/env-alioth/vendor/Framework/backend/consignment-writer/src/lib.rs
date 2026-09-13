//! # consignment-writer — 委托写链共享事务
//!
//! 单一事实源：`transport-dispatch::repositories` 的 `create_consignment_inner`（600+ 行表级全链事务）抽取，
//! 供两类调用方共用（L3 change: extract-consignment-writer-crate）：
//!
//! - **Gateway WZ**（transport-dispatch service）：平台调度员创建正式委托；
//! - **OpenActivity 门户**（customer.rs）：客户企业自助创建正式委托（2026-09-06 用户裁决 portal CNS = 正式委托）。
//!
//! 约束：两应用禁止 HTTP 互调 → 写链以共享 crate 形式单份实现（禁止复制事务到任一应用）。
//!
//! ## 设计要点（详见 openspec/changes/extract-consignment-writer-crate/design.md）
//!
//! - **主体语义参数化**：`WriteContext { customer_subject_id, operator_org_id, actor_user_id }`——
//!   组织解析外置（dispatch 注入用户绑定运营组织；portal 注入 SUBJ-SYSTEM），事务内不再自解析。
//! - **表级全链**：scal-weight/amount → orde-land 主档 → order_rr_contract 合同桥（幂等）→
//!   下单合同对（诉求 `cont-request` + 销售 `cont-sales` 及各自一式两份镜像，见 `contracts.rs`）→ segm-date →
//!   scal-price（成交）→ freight_road-sales + freight_road-request + prod-sales 桥 →
//!   rr_stop×2（ST-DEPART/ARRIVE）→ prod-loading + production_rr_storage 运力占用 + com-voucher 守卫 →
//!   deta-trade_order 明细（DTL + DTL-LDG）；坐标 "GC"/"FJA"/"↓_BE"。
//! - **事务语义**：接受 `&mut PgConnection`（调用方管理 commit/rollback），行为与迁移前逐表一致。
//!
//! 迁移状态：writer.rs 全链事务已迁入（models/coords 同包）；dispatch/OpenActivity 接线进行中。

pub mod contracts;
pub mod coords;
pub mod mirror;
pub mod models;
pub mod writer;

pub use contracts::{insert_order_contracts_tx, OrderContractsInput};
pub use mirror::{insert_order_mirror_tx, sync_mirror_status_tx, OrderMirrorInput};
pub use models::{CreateConsignmentInput, CreateConsignmentOutput, WriteContext};
pub use writer::create_full;
