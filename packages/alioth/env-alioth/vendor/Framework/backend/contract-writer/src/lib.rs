//! # contract-writer — 合约写件与一式两份镜像（Framework 公共 crate）
//!
//! 单一事实源：合约行（单行 / 主+镜像成对）、合同方行、`zc_id_contract_rr_symmetry` MIR 桥、
//! 镜像解析与级联软删、运输产品行（`zc_id_prod-freight_road-{sales,purchase}`）。
//! 供 WZ 平台（`contract` / `transport-dispatch`）与后续 namespace 复用；各 ns MUST NOT
//! 保留同语义的第二份 SQL（`REUSE_FIRST_SPEC`）。
//!
//! ## 设计要点
//!
//! - **形态单一派生源**：`_f_`/`_t_` 由本 crate 经
//!   `trigger_registry::lifecycle::derive_form_type` 从职能码派生后参数绑定——
//!   公开 API 只接受职能码，调用方无从手写字面量对（`ALIOTH_ONTOLOGY_SPEC.md` §4.3.3 形态 1）。
//! - **一式两份**：镜像落镜像叶（`ContractLeaf::mirrored`：`cont-sales` ↔ `cont-purchase` 相反叶表，
//!   诉求 `cont-request` 落同表），`code = {主code}-R`，**合同方与主合同逐字段相同（甲/乙不互换，
//!   2026-09-13 裁决；双边性由镜像行行级属权表达）**，互链走**叶**桥
//!   `zc_id_contract_rr_symmetry`（`code = MIR-{主id}-{镜像id}`，与续约行 `RNW-%` 同表异 code）。
//! - **事务语义**：公开写件接受 `&mut sqlx::PgConnection`，由调用方管理
//!   commit/rollback（同 `consignment-writer`）——本 crate MUST NOT 自启事务。
//! - **边界**：只承载物理写件；业务组装（标量服务、起讫桥 `rr_stop`、坐标解析器、
//!   桥路由 goods/deal/demand、业务校验、NGAC 行级注册）留在各 ns。

pub mod contract;
pub mod mirror;
pub mod models;
pub mod product;

pub use contract::{
    insert_contract_pair_tx, insert_contract_row_tx, insert_mirror_of_contract_tx,
    insert_parties_tx,
};
pub use mirror::{
    resolve_contract_product_ids_tx, resolve_mirror_ids_tx, soft_delete_contract_products_tx,
    soft_delete_contract_tx, soft_delete_mirror_tx,
};
pub use models::{
    ContractLeaf, ContractParty, ContractProductInput, ContractRowInput, DocParties,
    ProductPairInput, ProductRowInput,
};
pub use product::{
    create_contract_transport_product_tx, insert_product_pair_tx, insert_product_row_tx,
    insert_product_stops_tx,
};
