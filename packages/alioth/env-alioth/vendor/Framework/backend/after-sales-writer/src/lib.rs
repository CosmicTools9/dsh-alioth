//! # after-sales-writer — 售后段写链共享事务
//!
//! 单一事实源（change `wire-after-sales-order-chain` D2，damage-writer 同款模式）：
//!
//! - `zc_id_order-after_sales`（订单-售后服务，`zc_id_stat-trade_order` 子叶）主行
//!   + `zc_id_order_rr_contract` 幂等合同桥（G5 双写口径：`fk_contract` 物理列 + junction）；
//! - `zc_id_stat-appeal`（事实-售后申诉）主行——来源事件指针走框架来源列 `ak_source`
//!   （D3：MUST NOT 用 comments 承载结构化数据，comments 一律纯文本摘要）。
//!
//! 调用方：Gateway WZ（交易售后：货损/理赔申诉入口）与 Cosmic-Tools（bv-local 服务售后
//! 子叶切换）。两应用禁止 HTTP 互调 → 写事务以共享 crate 单份实现。
//!
//! ## 设计要点
//!
//! - **合同前置校验复用 G3 单源**：`consignment_writer::ensure_order_contract_valid_tx`
//!   （存在 / 状态 ∈ {active,executing} / 主体 ∈ 合同方；无桥 legacy 放行）。
//! - **事务语义**：`&mut sqlx::PgConnection` 由调用方管理 begin/commit/rollback。
//! - **坐标三元组参数化**：调用方经 `ontology_binding::resolve` 以 ns service.json 声明
//!   code 解析后注入（禁硬编码 ZUID）；标量先行（`qk_date` → `zc_id_scal-date` 等）
//!   由调用方在事务内自行完成。
//! - **id 口径**：子叶主行 `isahl.gen_next_zuid()`（`zc_id_lifecycle` 继承链）；
//!   桥行 `isahl.gen_next_uid(470)`（`zc_id_order_rr_contract` uid 段）。

pub mod writer;

pub use writer::{
    bind_after_sales_contract_tx, insert_after_sales_order_tx, insert_appeal_tx,
    AfterSalesOrderInput, AppealInput,
};
